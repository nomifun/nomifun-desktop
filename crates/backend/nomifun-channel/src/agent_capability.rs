//! Target-generation Channel messaging owner and scene lifecycle.
//!
//! Only `reply` and `send` are Agent Actions. Ingress routing and group policy
//! are derived from the selected Channel scene binding; pairing stays entirely
//! in the transport owner and is intentionally absent from this adapter.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{StrictJsonValue, TypedResourceBinding};
use nomifun_agent_domain_wave4::{
    CHANNEL_GROUP_POLICY, CHANNEL_RESOURCE_KIND,
    Wave4CapabilityOperation, Wave4HostPort, Wave4HostPortError,
    Wave4HostRequest, Wave4TurnMiddlewareHostPort,
    Wave4TurnMiddlewareHostRequest, typed_resource_binding,
};
use nomifun_db::IChannelRepository;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::group_policy::GroupPolicyFence;
use crate::manager::ChannelManager;
use crate::types::{OutgoingMessageType, UnifiedOutgoingMessage};

pub const CHANNEL_MESSAGING_MODULE_ID: &str = "channel.messaging";
pub const CHANNEL_REPLY_ACTION_ID: &str = "channel.messaging/reply";
pub const CHANNEL_SEND_ACTION_ID: &str = "channel.messaging/send";
pub const CHANNEL_MESSAGING_ACTION_IDS: [&str; 2] = [
    CHANNEL_REPLY_ACTION_ID,
    CHANNEL_SEND_ACTION_ID,
];

pub const CHANNEL_ACTION_OUTCOME_UNKNOWN: &str =
    nomifun_agent_domain_wave4::WAVE4_ACTION_OUTCOME_UNKNOWN;
const CHANNEL_DELIVERY_FAILED: &str = "CHANNEL_DELIVERY_FAILED";
const CHANNEL_NOT_CONNECTED: &str = "CHANNEL_NOT_CONNECTED";
const RESOURCE_NOT_FOUND: &str = "RESOURCE_NOT_FOUND";

pub fn channel_action_resource_operation(action_id: &str) -> Option<&'static str> {
    match action_id {
        CHANNEL_REPLY_ACTION_ID => Some("reply"),
        CHANNEL_SEND_ACTION_ID => Some("send"),
        _ => None,
    }
}

pub fn channel_action_input_schema(action_id: &str) -> Option<Value> {
    match action_id {
        CHANNEL_REPLY_ACTION_ID => Some(
            nomifun_agent_domain_wave4::action_input_schema(CHANNEL_REPLY_ACTION_ID).0,
        ),
        CHANNEL_SEND_ACTION_ID => Some(
            nomifun_agent_domain_wave4::action_input_schema(CHANNEL_SEND_ACTION_ID).0,
        ),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelSceneTarget {
    Companion { companion_id: String },
    CustomerService { cs_agent_id: String },
}

impl ChannelSceneTarget {
    fn owner_domain(&self) -> &'static str {
        match self {
            Self::Companion { .. } => nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION,
            Self::CustomerService { .. } => {
                nomifun_db::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE
            }
        }
    }

    fn typed_parameters(&self) -> BTreeMap<String, String> {
        match self {
            Self::Companion { companion_id } => {
                BTreeMap::from([("companion_id".to_owned(), companion_id.clone())])
            }
            Self::CustomerService { cs_agent_id } => {
                BTreeMap::from([("cs_agent_id".to_owned(), cs_agent_id.clone())])
            }
        }
    }
}

/// Narrow authority seam used to revalidate a customer-service Channel at
/// invocation time without making the Channel crate own customer records.
#[async_trait]
pub trait ChannelCustomerBindingAuthority: Send + Sync {
    async fn bound_customer_agent(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<String>, Wave4HostPortError>;
}

/// Canonical Session ingress lease used by the Channel scene. The Channel
/// action owner does not read Conversation extras or construct Sessions; the
/// app supplies an implementation backed by the canonical AgentSession
/// resource owner.
#[async_trait]
pub trait ChannelSceneIngressPort: Send + Sync {
    async fn bind_scene(
        &self,
        channel_plugin_id: &str,
        companion_id: &str,
        agent_session_id: &str,
    ) -> Result<(), Wave4HostPortError>;

    async fn release_scene(
        &self,
        agent_session_id: &str,
    ) -> Result<usize, Wave4HostPortError>;
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ChannelSceneContext {
    pub kind: String,
    pub channel_plugin_id: String,
    pub platform: String,
    pub owner_domain: String,
    pub group_access_mode: String,
    pub agent_session_id: String,
}

/// Channel-owned target adapter. Resource resolution and every invocation
/// re-read the live plugin so a disabled or reassigned binding fails closed.
pub struct ChannelAgentCapabilityOwner {
    authoritative_owner_id: Arc<str>,
    manager: Arc<ChannelManager>,
    repository: Arc<dyn IChannelRepository>,
    group_policy_fence: Arc<GroupPolicyFence>,
    ingress: Option<Arc<dyn ChannelSceneIngressPort>>,
    customer_bindings: Option<Arc<dyn ChannelCustomerBindingAuthority>>,
}

impl ChannelAgentCapabilityOwner {
    pub fn new(
        authoritative_owner_id: impl Into<Arc<str>>,
        manager: Arc<ChannelManager>,
        repository: Arc<dyn IChannelRepository>,
        group_policy_fence: Arc<GroupPolicyFence>,
    ) -> Self {
        Self {
            authoritative_owner_id: authoritative_owner_id.into(),
            manager,
            repository,
            group_policy_fence,
            ingress: None,
            customer_bindings: None,
        }
    }

    pub fn with_ingress(mut self, ingress: Arc<dyn ChannelSceneIngressPort>) -> Self {
        self.ingress = Some(ingress);
        self
    }

    pub fn with_customer_binding_authority(
        mut self,
        authority: Arc<dyn ChannelCustomerBindingAuthority>,
    ) -> Self {
        self.customer_bindings = Some(authority);
        self
    }

    /// Resolve Action authority plus the scene-owned ingress/policy operations.
    /// The caller may select only the two real Agent Actions; `receive` and
    /// `manage` are server-derived lifecycle operations and cannot be supplied
    /// as grant IDs.
    pub async fn resolve_scene_binding(
        &self,
        principal_id: &str,
        channel_plugin_id: &str,
        target: &ChannelSceneTarget,
        allowed_action_ids: &BTreeSet<String>,
    ) -> Result<TypedResourceBinding, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        let mut operations = BTreeSet::from(["receive".to_owned(), "manage".to_owned()]);
        for action_id in allowed_action_ids {
            let operation = channel_action_resource_operation(action_id).ok_or_else(|| {
                Wave4HostPortError::resource_binding_invalid(format!(
                    "Channel scene cannot derive authority from undeclared Action {action_id}",
                ))
            })?;
            operations.insert(operation.to_owned());
        }
        let mut binding = typed_resource_binding(
            format!("channel:{channel_plugin_id}"),
            CHANNEL_RESOURCE_KIND,
            channel_plugin_id.to_owned(),
            principal_id.to_owned(),
            operations,
        );
        binding.typed_parameters = target.typed_parameters();
        let plugin = self.load_scene_plugin(principal_id, &binding).await?;
        if plugin.owner_domain != target.owner_domain() {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Channel scene target does not match the live Channel owner domain",
            ));
        }
        Ok(binding)
    }

    /// Activate ingress and return the group-policy Context for one exact
    /// Session. Pairing is not consulted: it belongs to transport setup.
    pub async fn activate_scene_binding(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        binding: &TypedResourceBinding,
    ) -> Result<ChannelSceneContext, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        require_binding_operation(binding, principal_id, "receive")?;
        require_binding_operation(binding, principal_id, "manage")?;
        let plugin = self.load_scene_plugin(principal_id, binding).await?;
        if !self.manager.is_plugin_running(binding.resource_id.as_ref()) {
            return Err(Wave4HostPortError::new(
                CHANNEL_NOT_CONNECTED,
                "bound Channel resource is not connected to the live message loop",
            ));
        }

        // Synchronize after any in-flight policy writer before publishing the
        // scene Context used by the turn.
        let _policy = self
            .group_policy_fence
            .read(binding.resource_id.as_ref())
            .await;
        if plugin.owner_domain == nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION {
            let companion_id = binding
                .typed_parameters
                .get("companion_id")
                .expect("load_scene_plugin validated companion_id");
            let ingress = self.ingress.as_ref().ok_or_else(|| {
                Wave4HostPortError::unavailable(
                    "Channel scene ingress owner has not completed startup",
                )
            })?;
            ingress
                .bind_scene(
                    binding.resource_id.as_ref(),
                    companion_id,
                    agent_session_id,
                )
                .await?;
        }
        Ok(ChannelSceneContext {
            kind: "channel_scene".to_owned(),
            channel_plugin_id: plugin.channel_plugin_id,
            platform: plugin.r#type,
            owner_domain: plugin.owner_domain,
            group_access_mode: plugin.group_access_mode,
            agent_session_id: agent_session_id.to_owned(),
        })
    }

    pub async fn release_scene_binding(
        &self,
        agent_session_id: &str,
    ) -> Result<usize, Wave4HostPortError> {
        match &self.ingress {
            Some(ingress) => ingress.release_scene(agent_session_id).await,
            None => Ok(0),
        }
    }

    /// Re-derive group policy immediately before a turn. No authorable policy
    /// capability or model-provided selector participates in this path.
    pub async fn apply_scene_policy(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        binding: &TypedResourceBinding,
        turn_input: &StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        if !turn_input.0.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Channel scene turn input must be an object",
            ));
        }
        require_binding_operation(binding, principal_id, "manage")?;
        let plugin_id = binding.resource_id.as_ref();
        let _policy = self.group_policy_fence.read(plugin_id).await;
        let plugin = self.load_scene_plugin(principal_id, binding).await?;
        Ok(StrictJsonValue(json!({
            "kind": "channel_group_policy",
            "channel_plugin_id": plugin.channel_plugin_id,
            "platform": plugin.r#type,
            "owner_domain": plugin.owner_domain,
            "mode": plugin.group_access_mode,
            "agent_session_id": agent_session_id,
        })))
    }

    /// Invoke one target Channel Action. Provider failure after dispatch begins
    /// is always `outcome_unknown`; callers must settle/reconcile the canonical
    /// effect receipt instead of replaying automatically.
    pub async fn invoke_target_action(
        &self,
        principal_id: &str,
        action_id: &str,
        bindings: &[TypedResourceBinding],
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        self.require_owner(principal_id)?;
        if !input.0.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Channel Action input must be an object",
            ));
        }
        let operation = channel_action_resource_operation(action_id).ok_or_else(|| {
            Wave4HostPortError::action_operation_mismatch(format!(
                "undeclared Channel Action {action_id}",
            ))
        })?;
        let binding = exact_channel_binding(bindings, principal_id, operation)?;
        let plugin_id = binding.resource_id.as_ref();
        self.load_scene_plugin(principal_id, binding).await?;
        if !self.manager.is_plugin_running(plugin_id) {
            return Err(Wave4HostPortError::new(
                CHANNEL_NOT_CONNECTED,
                format!("bound Channel resource {plugin_id} is not connected"),
            ));
        }
        let _policy = self.group_policy_fence.read(plugin_id).await;
        let (destination_ref, message) = match action_id {
            CHANNEL_SEND_ACTION_ID => {
                let input: ChannelSendInput = parse_input(input.0)?;
                bounded_reference(&input.destination_ref, "destination_ref")?;
                bounded_text(&input.text, "text")?;
                (
                    input.destination_ref,
                    outgoing_text(input.text, None),
                )
            }
            CHANNEL_REPLY_ACTION_ID => {
                let input: ChannelReplyInput = parse_input(input.0)?;
                bounded_reference(&input.destination_ref, "destination_ref")?;
                bounded_reference(&input.message_ref, "message_ref")?;
                bounded_text(&input.text, "text")?;
                (
                    input.destination_ref,
                    outgoing_text(input.text, Some(input.message_ref)),
                )
            }
            _ => unreachable!("validated Channel Action"),
        };

        let message_ref = self
            .manager
            .send_message(plugin_id, &destination_ref, message)
            .await
            .map_err(|error| {
                Wave4HostPortError::new(
                    CHANNEL_ACTION_OUTCOME_UNKNOWN,
                    format!(
                        "Channel delivery returned an uncertain result after dispatch began: {error}",
                    ),
                )
            })?;
        Ok(StrictJsonValue(json!({
            "channel_plugin_id": plugin_id,
            "destination_ref": destination_ref,
            "receipt_id": message_ref.clone(),
            "message_ref": message_ref,
        })))
    }

    fn require_owner(&self, principal_id: &str) -> Result<(), Wave4HostPortError> {
        if principal_id != self.authoritative_owner_id.as_ref() {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Channel resource belongs to another installation owner",
            ));
        }
        Ok(())
    }

    async fn load_scene_plugin(
        &self,
        principal_id: &str,
        binding: &TypedResourceBinding,
    ) -> Result<nomifun_db::models::ChannelPluginRow, Wave4HostPortError> {
        if binding.resource_kind.as_ref() != CHANNEL_RESOURCE_KIND {
            return Err(Wave4HostPortError::resource_binding_invalid(
                "Channel owner received a non-Channel resource binding",
            ));
        }
        if binding.owner_id != principal_id {
            return Err(Wave4HostPortError::resource_owner_mismatch(format!(
                "Channel binding belongs to {}, not {principal_id}",
                binding.owner_id,
            )));
        }
        let plugin_id = binding.resource_id.as_ref();
        let plugin = self
            .repository
            .get_plugin(plugin_id)
            .await
            .map_err(|error| {
                Wave4HostPortError::new(
                    CHANNEL_DELIVERY_FAILED,
                    format!("Channel resource lookup failed: {error}"),
                )
            })?
            .ok_or_else(|| {
                Wave4HostPortError::new(
                    RESOURCE_NOT_FOUND,
                    format!("bound Channel resource {plugin_id} does not exist"),
                )
            })?;
        if !plugin.enabled {
            return Err(Wave4HostPortError::new(
                CHANNEL_NOT_CONNECTED,
                format!("bound Channel resource {plugin_id} is disabled"),
            ));
        }
        match plugin.owner_domain.as_str() {
            nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION => {
                if binding.typed_parameters.contains_key("cs_agent_id") {
                    return Err(Wave4HostPortError::resource_owner_mismatch(
                        "companion Channel binding carries customer-service ownership",
                    ));
                }
                if binding.typed_parameters.len() != 1 {
                    return Err(Wave4HostPortError::resource_binding_invalid(
                        "companion Channel binding must contain only companion_id",
                    ));
                }
                let companion_id = binding
                    .typed_parameters
                    .get("companion_id")
                    .ok_or_else(|| {
                        Wave4HostPortError::resource_binding_invalid(
                            "companion Channel binding must contain only companion_id",
                        )
                    })?;
                if plugin.companion_id.as_deref() != Some(companion_id.as_str()) {
                    return Err(Wave4HostPortError::resource_owner_mismatch(
                        "companion Channel was reassigned after scene binding resolution",
                    ));
                }
            }
            nomifun_db::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE => {
                if plugin.companion_id.is_some()
                    || binding.typed_parameters.contains_key("companion_id")
                {
                    return Err(Wave4HostPortError::resource_owner_mismatch(
                        "customer-service Channel binding carries Companion ownership",
                    ));
                }
                if binding.typed_parameters.len() != 1 {
                    return Err(Wave4HostPortError::resource_binding_invalid(
                        "customer-service Channel binding must contain only cs_agent_id",
                    ));
                }
                let cs_agent_id = binding
                    .typed_parameters
                    .get("cs_agent_id")
                    .ok_or_else(|| {
                        Wave4HostPortError::resource_binding_invalid(
                            "customer-service Channel binding must contain only cs_agent_id",
                        )
                    })?;
                let authority = self.customer_bindings.as_ref().ok_or_else(|| {
                    Wave4HostPortError::unavailable(
                        "customer-service Channel binding authority is unavailable",
                    )
                })?;
                let current = authority.bound_customer_agent(plugin_id).await?;
                if current.as_deref() != Some(cs_agent_id.as_str()) {
                    return Err(Wave4HostPortError::resource_owner_mismatch(
                        "customer-service Channel was reassigned after scene binding resolution",
                    ));
                }
            }
            other => {
                return Err(Wave4HostPortError::resource_owner_mismatch(format!(
                    "Channel resource has unsupported owner domain {other}",
                )));
            }
        }
        Ok(plugin)
    }
}

impl Wave4HostPort for ChannelAgentCapabilityOwner {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            if request.context.principal.principal_kind != "user" {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "Channel Actions require a user principal",
                ));
            }
            let (action_id, input) = match request.operation {
                Wave4CapabilityOperation::ChannelReply { input } => {
                    (CHANNEL_REPLY_ACTION_ID, input)
                }
                Wave4CapabilityOperation::ChannelSend { input } => {
                    (CHANNEL_SEND_ACTION_ID, input)
                }
                _ => {
                    return Err(Wave4HostPortError::action_operation_mismatch(
                        "Channel owner received a foreign operation",
                    ));
                }
            };
            self.invoke_target_action(
                &request.context.principal.principal_id,
                action_id,
                &request.context.resource_bindings,
                input,
            )
            .await
        })
    }
}

/// Transitional adapter for the Wave4 registration. The policy is read from
/// the scene binding; callers cannot author or select policy content.
impl Wave4TurnMiddlewareHostPort for ChannelAgentCapabilityOwner {
    fn apply<'a>(
        &'a self,
        request: Wave4TurnMiddlewareHostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            if request.capability_id.as_ref() != CHANNEL_GROUP_POLICY {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "Channel middleware owner received a foreign contribution",
                ));
            }
            if request.principal.principal_kind != "user" {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "Channel scene policy requires a user principal",
                ));
            }
            let binding = exact_channel_binding(
                &request.resource_bindings,
                &request.principal.principal_id,
                "manage",
            )?;
            self.activate_scene_binding(
                &request.principal.principal_id,
                request.agent_session_id.as_ref(),
                binding,
            )
            .await?;
            self.apply_scene_policy(
                &request.principal.principal_id,
                request.agent_session_id.as_ref(),
                binding,
                &request.turn_input,
            )
            .await
        })
    }
}

fn exact_channel_binding<'a>(
    bindings: &'a [TypedResourceBinding],
    principal_id: &str,
    operation: &str,
) -> Result<&'a TypedResourceBinding, Wave4HostPortError> {
    let mut matches = bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == CHANNEL_RESOURCE_KIND);
    let binding = matches.next().ok_or_else(|| {
        Wave4HostPortError::resource_not_bound("Channel Action requires one Channel resource")
    })?;
    if matches.next().is_some() {
        return Err(Wave4HostPortError::resource_binding_invalid(
            "Channel Action received duplicate Channel resources",
        ));
    }
    require_binding_operation(binding, principal_id, operation)?;
    Ok(binding)
}

fn require_binding_operation(
    binding: &TypedResourceBinding,
    principal_id: &str,
    operation: &str,
) -> Result<(), Wave4HostPortError> {
    if binding.resource_kind.as_ref() != CHANNEL_RESOURCE_KIND
        || binding.resource_id.as_ref().trim().is_empty()
        || binding.connection_config_ref.is_some()
        || binding.operations.iter().any(|operation| {
            !matches!(operation.as_str(), "receive" | "manage" | "reply" | "send")
        })
    {
        return Err(Wave4HostPortError::resource_binding_invalid(
            "Channel binding has invalid identity metadata",
        ));
    }
    if binding.owner_id != principal_id {
        return Err(Wave4HostPortError::resource_owner_mismatch(format!(
            "Channel binding belongs to {}, not {principal_id}",
            binding.owner_id,
        )));
    }
    if !binding.operations.contains(operation) {
        return Err(Wave4HostPortError::resource_not_bound(format!(
            "Channel binding does not grant operation {operation}",
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelSendInput {
    destination_ref: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelReplyInput {
    destination_ref: String,
    message_ref: String,
    text: String,
}

fn parse_input<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, Wave4HostPortError> {
    serde_json::from_value(value)
        .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))
}

fn bounded_text(value: &str, field: &str) -> Result<(), Wave4HostPortError> {
    if value.trim().is_empty() || value.chars().count() > 16_384 {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{field} must be non-empty and at most 16384 characters",
        )));
    }
    Ok(())
}

fn bounded_reference(value: &str, field: &str) -> Result<(), Wave4HostPortError> {
    if value.trim().is_empty() || value.chars().count() > 512 {
        return Err(Wave4HostPortError::invalid_request(format!(
            "{field} must be non-empty and at most 512 characters",
        )));
    }
    Ok(())
}

fn outgoing_text(text: String, reply_to_message_id: Option<String>) -> UnifiedOutgoingMessage {
    UnifiedOutgoingMessage {
        message_type: OutgoingMessageType::Text,
        text: Some(text),
        parse_mode: None,
        buttons: None,
        keyboard: None,
        image_url: None,
        file_url: None,
        file_name: None,
        media_actions: None,
        reply_to_message_id,
        silent: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_send_and_reply_are_agent_actions() {
        assert_eq!(
            channel_action_resource_operation(CHANNEL_REPLY_ACTION_ID),
            Some("reply"),
        );
        assert_eq!(
            channel_action_resource_operation(CHANNEL_SEND_ACTION_ID),
            Some("send"),
        );
        for lifecycle_id in [
            nomifun_agent_domain_wave4::CHANNEL_RECEIVE,
            nomifun_agent_domain_wave4::CHANNEL_PAIRING,
            nomifun_agent_domain_wave4::CHANNEL_GROUP_POLICY,
        ] {
            assert_eq!(channel_action_resource_operation(lifecycle_id), None);
        }
    }

    #[test]
    fn scene_target_freezes_exact_domain_identity() {
        let companion = ChannelSceneTarget::Companion {
            companion_id: "companion-1".to_owned(),
        };
        assert_eq!(
            companion.typed_parameters(),
            BTreeMap::from([("companion_id".to_owned(), "companion-1".to_owned())]),
        );
        let customer = ChannelSceneTarget::CustomerService {
            cs_agent_id: "customer-1".to_owned(),
        };
        assert_eq!(
            customer.typed_parameters(),
            BTreeMap::from([("cs_agent_id".to_owned(), "customer-1".to_owned())]),
        );
    }

    #[test]
    fn action_payloads_are_strict_and_bounded() {
        let valid: ChannelReplyInput = parse_input(json!({
            "destination_ref": "chat",
            "message_ref": "message",
            "text": "hello",
        }))
        .unwrap();
        assert_eq!(valid.text, "hello");
        assert!(
            parse_input::<ChannelReplyInput>(json!({
                "destination_ref": "chat",
                "message_ref": "message",
                "text": "hello",
                "unexpected": true,
            }))
            .is_err()
        );
        assert!(bounded_text("", "text").is_err());
    }

}
