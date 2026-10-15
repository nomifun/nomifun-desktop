//! Canonical `robot` Module owner.
//!
//! Agent authority is one Module plus an exact Action allowlist. Pairing and a
//! live device connection are resource facts, while the voice/audio loop is a
//! Robot domain service; neither is an authorable capability. Device tool
//! discovery is frozen per live Session Runtime and every physical dispatch rechecks
//! the mutable device permission and pairing boundaries.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, CorrelationId, IdempotencyKey, OperationId,
    PrincipalRef, StrictJsonValue, TypedResourceBinding, digest_payload,
};
use nomifun_ai_agent::{
    NomiHostDynamicToolError, NomiHostDynamicToolInvocation, NomiHostDynamicToolInvoker,
};
use nomifun_robot::capability::{ROBOT_ACTION_IDS, ROBOT_MODULE_ID, RobotAction};
use nomifun_robot::effect_ledger::{
    RobotEffectAdmission, RobotEffectKey, RobotEffectLedger, RobotEffectRequest,
};
use nomifun_robot::mcp_bridge::ToolCallError;
use nomifun_robot::registry::{RobotRecord, RobotRegistry};
use nomifun_robot::tool_registry::RobotToolRegistry;
use nomifun_robot::vision::RobotVisionObservationRegistry;

const ROBOT_RESOURCE_KIND: &str = "robot";
const ROBOT_NOT_FOUND: &str = "ROBOT_NOT_FOUND";
const ROBOT_NOT_PAIRED: &str = "ROBOT_NOT_PAIRED";
const ROBOT_OFFLINE: &str = "ROBOT_OFFLINE";
const ROBOT_PERMISSION_DENIED: &str = "ROBOT_PERMISSION_DENIED";
const ROBOT_RESOURCE_NOT_BOUND: &str = "PRESET_RESOURCE_NOT_BOUND";
const ROBOT_RESOURCE_OWNER_MISMATCH: &str = "RESOURCE_OWNER_MISMATCH";
const ROBOT_INVALID_PAYLOAD: &str = "INVALID_PAYLOAD";
const ROBOT_ACTION_NOT_GRANTED: &str = "ACTION_NOT_GRANTED";
const ROBOT_DEVICE_REJECTED: &str = "ROBOT_DEVICE_REJECTED";
const ROBOT_EFFECT_FAILED: &str = "ROBOT_EFFECT_FAILED";
const ROBOT_EFFECT_OUTCOME_UNKNOWN: &str = "ROBOT_EFFECT_OUTCOME_UNKNOWN";
const ROBOT_EFFECT_RECEIPT_FAILED: &str = "ROBOT_EFFECT_RECEIPT_FAILED";
const MAX_TOOL_RESULT_CHARS: usize = 65_536;
const MAX_DEVICE_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_DEVICE_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;

pub(crate) fn module_capability_id() -> CapabilityId {
    CapabilityId::from(ROBOT_MODULE_ID)
}

pub(crate) fn action_ids() -> BTreeSet<ActionId> {
    ROBOT_ACTION_IDS.into_iter().map(ActionId::from).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RobotModuleError {
    pub code: String,
    pub message: String,
    pub retry_safe: bool,
}

impl RobotModuleError {
    fn new(code: impl Into<String>, message: impl Into<String>, retry_safe: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retry_safe,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new(ROBOT_INVALID_PAYLOAD, message, false)
    }

    fn not_bound(message: impl Into<String>) -> Self {
        Self::new(ROBOT_RESOURCE_NOT_BOUND, message, false)
    }

    fn owner_mismatch(message: impl Into<String>) -> Self {
        Self::new(ROBOT_RESOURCE_OWNER_MISMATCH, message, false)
    }
}

impl std::fmt::Display for RobotModuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RobotModuleError {}

#[derive(Clone)]
struct FrozenRobotDeviceTool {
    action: RobotAction,
    device_name: String,
    device_exposed_name: String,
    raw_input_schema: serde_json::Value,
    validator: Arc<jsonschema::Validator>,
    raw_validator: Arc<jsonschema::Validator>,
    default_arguments: serde_json::Map<String, serde_json::Value>,
    fixed_arguments: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RobotSessionToolDescriptor {
    pub action_id: ActionId,
    pub provider_name: String,
    pub description: String,
    pub input_schema: StrictJsonValue,
}

pub(crate) struct ResolvedRobotSessionTools {
    pub descriptors: Vec<RobotSessionToolDescriptor>,
    pub invoker: Arc<dyn NomiHostDynamicToolInvoker>,
    revocation: Arc<AtomicBool>,
}

impl ResolvedRobotSessionTools {
    pub(crate) fn revoke(&self) {
        self.revocation.store(true, Ordering::Release);
    }
}

struct BoundRobotSessionToolInvoker {
    owner: Arc<RobotModuleOwner>,
    principal: PrincipalRef,
    agent_session_id: AgentSessionId,
    binding: TypedResourceBinding,
    connection_id: String,
    tools: BTreeMap<String, FrozenRobotDeviceTool>,
    revoked: Arc<AtomicBool>,
}

/// Real native owner used by unified Runtime and source-integrated engines.
pub(crate) struct RobotModuleOwner {
    authoritative_user_id: Arc<str>,
    registry: Arc<RobotRegistry>,
    tools: Arc<RobotToolRegistry>,
    effects: Arc<RobotEffectLedger>,
    observations: Arc<RobotVisionObservationRegistry>,
}

impl RobotModuleOwner {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        robot: Arc<crate::robot_wiring::RobotServices>,
    ) -> Self {
        Self {
            authoritative_user_id,
            registry: Arc::clone(&robot.registry),
            tools: Arc::clone(&robot.tools),
            effects: Arc::clone(&robot.effect_ledger),
            observations: Arc::clone(&robot.vision_observations),
        }
    }

    #[cfg(test)]
    fn from_parts(
        authoritative_user_id: Arc<str>,
        registry: Arc<RobotRegistry>,
        tools: Arc<RobotToolRegistry>,
        effects: Arc<RobotEffectLedger>,
        observations: Arc<RobotVisionObservationRegistry>,
    ) -> Self {
        Self {
            authoritative_user_id,
            registry,
            tools,
            effects,
            observations,
        }
    }

    /// Freeze the exact live device tools permitted by one Module Action
    /// allowlist. No model-provided device identity or category is trusted.
    pub(crate) async fn resolve_session_tools(
        self: &Arc<Self>,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        binding: &TypedResourceBinding,
        allowed_actions: &BTreeSet<ActionId>,
    ) -> Result<ResolvedRobotSessionTools, RobotModuleError> {
        self.ensure_installation_owner(&principal.principal_id)?;
        if allowed_actions.is_empty() {
            return Err(RobotModuleError::new(
                ROBOT_ACTION_NOT_GRANTED,
                "the robot Module has no explicit Action grants",
                false,
            ));
        }
        for action_id in allowed_actions {
            if RobotAction::parse(action_id.as_ref()).is_none() {
                return Err(RobotModuleError::new(
                    ROBOT_ACTION_NOT_GRANTED,
                    format!(
                        "{} is not an Action declared by the robot Module",
                        action_id.as_ref()
                    ),
                    false,
                ));
            }
        }
        self.validate_binding(binding, &principal.principal_id)?;
        let robot = self.load_bound_robot(binding).await?;
        let connection_id = self.tools.connection_id(&robot.robot_id).await.ok_or_else(|| {
            RobotModuleError::new(
                ROBOT_OFFLINE,
                "The bound robot is offline. Connect it to this desktop before starting a new operation.",
                true,
            )
        })?;
        if !self
            .registry
            .connection_matches(&robot.robot_id, &connection_id)
            .await
        {
            return Err(RobotModuleError::new(
                ROBOT_OFFLINE,
                "The bound robot connection changed while its Action surface was being resolved.",
                true,
            ));
        }

        let mut descriptors = Vec::new();
        let mut frozen = BTreeMap::new();
        let configured = nomifun_robot::capability_profile::resolve_profile_tools(
            &robot.capability_profile, &self.tools.tools(&robot.robot_id).await,
        ).map_err(|error| RobotModuleError::invalid(error.to_string()))?;
        for tool in configured {
            let action = tool.action;
            let action_id = ActionId::from(action.id());
            if !allowed_actions.contains(&action_id) {
                continue;
            }
            if !binding.operations.contains(action.resource_operation()) {
                return Err(RobotModuleError::owner_mismatch(format!(
                    "Robot binding does not grant {} for {}",
                    action.resource_operation(),
                    action.id()
                )));
            }
            if !robot.permissions.allows_action(action)
                || !robot.permissions.allows_configured_tool(&robot.capability_profile, &tool.device_name) {
                continue;
            }
            validate_provider_tool_name(&tool.provider_name)?;
            if descriptors.len() >= 128 {
                return Err(RobotModuleError::invalid(
                    "Robot Session tool surface exceeds 128 actions",
                ));
            }
            let input_schema = strict_device_input_schema(tool.input_schema.clone())?;
            let validator = Arc::new(
                jsonschema::options()
                    .with_retriever(NoExternalDeviceSchema)
                    .build(&input_schema.0)
                    .map_err(|_| RobotModuleError::invalid("Robot device schema is invalid"))?,
            );
            let raw_schema = strict_device_input_schema(tool.raw_input_schema.clone())?;
            let raw_validator = Arc::new(jsonschema::options()
                .with_retriever(NoExternalDeviceSchema)
                .build(&raw_schema.0)
                .map_err(|_| RobotModuleError::invalid("Robot device schema is invalid"))?);
            let description = bounded_description(&tool.description);
            if frozen
                .insert(
                    tool.provider_name.clone(),
                    FrozenRobotDeviceTool {
                        action,
                        device_name: tool.device_name,
                        device_exposed_name: tool.device_exposed_name,
                        raw_input_schema: tool.raw_input_schema,
                        validator,
                        raw_validator,
                        default_arguments: tool.default_arguments,
                        fixed_arguments: tool.fixed_arguments,
                    },
                )
                .is_some()
            {
                return Err(RobotModuleError::invalid(format!(
                    "Robot published duplicate provider tool {}",
                    tool.provider_name
                )));
            }
            descriptors.push(RobotSessionToolDescriptor {
                action_id: action_id.clone(),
                provider_name: tool.provider_name,
                description,
                input_schema,
            });
        }
        descriptors.sort_by(|left, right| {
            (&left.action_id, &left.provider_name).cmp(&(&right.action_id, &right.provider_name))
        });
        let revoked = Arc::new(AtomicBool::new(false));
        let invoker: Arc<dyn NomiHostDynamicToolInvoker> = Arc::new(BoundRobotSessionToolInvoker {
            owner: Arc::clone(self),
            principal: principal.clone(),
            agent_session_id: agent_session_id.clone(),
            binding: binding.clone(),
            connection_id,
            tools: frozen,
            revoked: Arc::clone(&revoked),
        });
        Ok(ResolvedRobotSessionTools {
            descriptors,
            invoker,
            revocation: revoked,
        })
    }

    /// Produce recent camera context only under the exact `robot/vision`
    /// Action and bound device permission.
    pub(crate) async fn vision_context(
        &self,
        principal_id: &str,
        binding: &TypedResourceBinding,
        allowed_actions: &BTreeSet<ActionId>,
    ) -> Result<Option<StrictJsonValue>, RobotModuleError> {
        self.ensure_installation_owner(principal_id)?;
        self.validate_binding(binding, principal_id)?;
        if !allowed_actions.contains(&ActionId::from(RobotAction::Vision.id())) {
            return Err(RobotModuleError::new(
                ROBOT_ACTION_NOT_GRANTED,
                "robot/vision is not granted by this AgentSession Snapshot",
                false,
            ));
        }
        if !binding
            .operations
            .contains(RobotAction::Vision.resource_operation())
        {
            return Err(RobotModuleError::owner_mismatch(
                "Robot binding does not grant vision",
            ));
        }
        let robot = self.load_bound_robot(binding).await?;
        if !robot.permissions.allows_action(RobotAction::Vision) {
            // This is an optional read of an existing observation, not a
            // request to capture an image. A paired voice/display device must
            // remain usable with camera access disabled. Do not expose a
            // previously cached observation after that permission is revoked.
            // Explicit camera tools retain their dispatch-time permission gate.
            return Ok(None);
        }
        let companion_id = robot
            .companion_id
            .as_deref()
            .expect("load_bound_robot requires pairing");
        Ok(self
            .observations
            .latest_recent(&robot.robot_id, companion_id, now_ms())
            .await
            .map(|observation| {
                StrictJsonValue(serde_json::json!({
                    "kind": "robot_vision",
                    "robot_id": observation.robot_id,
                    "companion_id": observation.companion_id,
                    "question": observation.question,
                    "answer": observation.answer,
                    "observed_at_ms": observation.observed_at_ms,
                }))
            }))
    }

    fn ensure_installation_owner(&self, principal_id: &str) -> Result<(), RobotModuleError> {
        if principal_id != self.authoritative_user_id.as_ref() {
            return Err(RobotModuleError::owner_mismatch(
                "Robot request principal is not the installation owner",
            ));
        }
        Ok(())
    }

    fn validate_binding(
        &self,
        binding: &TypedResourceBinding,
        principal_id: &str,
    ) -> Result<(), RobotModuleError> {
        if binding.resource_kind.as_ref() != ROBOT_RESOURCE_KIND {
            return Err(RobotModuleError::not_bound(
                "Pair a robot and bind that device to this Agent before enabling Robot actions.",
            ));
        }
        if binding.owner_id != principal_id {
            return Err(RobotModuleError::owner_mismatch(
                "Robot resource binding belongs to another principal",
            ));
        }
        Ok(())
    }

    async fn load_bound_robot(
        &self,
        binding: &TypedResourceBinding,
    ) -> Result<RobotRecord, RobotModuleError> {
        let robot = self
            .registry
            .get(binding.resource_id.as_ref())
            .await
            .ok_or_else(|| {
                RobotModuleError::new(
                    ROBOT_NOT_FOUND,
                    "The bound robot is no longer registered. Pair a device and update this Agent binding.",
                    false,
                )
            })?;
        let companion_id = robot.companion_id.as_deref().ok_or_else(|| {
            RobotModuleError::new(
                ROBOT_NOT_PAIRED,
                "The bound robot is not paired with a Companion. Pair it in Device settings first.",
                false,
            )
        })?;
        let bound_companion = binding
            .typed_parameters
            .get("companion_id")
            .ok_or_else(|| {
                RobotModuleError::not_bound(
                    "Robot binding has no server-resolved companion identity",
                )
            })?;
        if companion_id != bound_companion {
            return Err(RobotModuleError::owner_mismatch(
                "The robot was rebound to another Companion; update this Agent binding.",
            ));
        }
        let expected_revision = binding.typed_parameters.get("capability_revision")
            .map(|value| value.parse::<u64>().map_err(|_| RobotModuleError::invalid("Invalid robot capability revision")))
            .transpose()?.unwrap_or(0);
        if expected_revision != robot.capability_revision {
            return Err(RobotModuleError::new("ROBOT_CAPABILITIES_CHANGED",
                "Robot capabilities changed. Reapply this robot to the idle conversation before starting a new operation.", false));
        }
        Ok(robot)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_bound_device_tool(
        &self,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        _operation_id: &OperationId,
        idempotency_key: &IdempotencyKey,
        _correlation_id: &CorrelationId,
        binding: &TypedResourceBinding,
        action: RobotAction,
        exposed_name: &str,
        device_exposed_name: &str,
        expected_device_name: &str,
        expected_connection_id: &str,
        expected_input_schema: &serde_json::Value,
        arguments: serde_json::Value,
    ) -> Result<StrictJsonValue, RobotModuleError> {
        if !arguments.is_object() {
            return Err(RobotModuleError::invalid(
                "Robot tool arguments must be an object",
            ));
        }
        self.ensure_installation_owner(&principal.principal_id)?;
        self.validate_binding(binding, &principal.principal_id)?;
        if !binding.operations.contains(action.resource_operation()) {
            return Err(RobotModuleError::owner_mismatch(format!(
                "Robot resource does not grant {}",
                action.resource_operation()
            )));
        }
        let robot = self.load_bound_robot(binding).await?;
        let Some(_action_lease) = self
            .registry
            .hold_action_connection(&robot.robot_id, expected_connection_id)
            .await
        else {
            return Err(RobotModuleError::new(
                ROBOT_OFFLINE,
                "The bound robot reconnected after this Session froze its Action surface. Start a new operation.",
                true,
            ));
        };
        // Re-read every mutable authority fact while the device operation
        // lease is held. Permission, pairing, removal and reconnect all need
        // the exclusive side of the same gate.
        let robot = self.load_bound_robot(binding).await?;
        if !robot.permissions.allows_action(action)
            || !robot.permissions.allows_configured_tool(&robot.capability_profile, expected_device_name)
        {
            return Err(RobotModuleError::new(
                ROBOT_PERMISSION_DENIED,
                format!(
                    "{} is disabled for the bound robot; enable it in Device settings.",
                    action.display_name()
                ),
                false,
            ));
        }
        if !self.tools.is_attached(&robot.robot_id).await {
            return Err(RobotModuleError::new(
                ROBOT_OFFLINE,
                "The bound robot is offline. Connect it to this desktop and try a new operation.",
                true,
            ));
        }

        let canonical_input = StrictJsonValue(serde_json::json!({
            "tool_name": exposed_name,
            "arguments": arguments,
        }));
        let input_digest = digest_payload(&canonical_input)
            .map_err(|error| RobotModuleError::invalid(error.to_string()))?;
        let effect_request = RobotEffectRequest {
            key: RobotEffectKey {
                principal_id: principal.principal_id.clone(),
                agent_session_id: agent_session_id.as_ref().to_owned(),
                module_id: ROBOT_MODULE_ID.to_owned(),
                action_id: action.id().to_owned(),
                idempotency_key: idempotency_key.as_ref().to_owned(),
            },
            input_digest: input_digest.as_ref().to_owned(),
            robot_id: robot.robot_id.clone(),
            tool_name: exposed_name.to_owned(),
            reserved_at_ms: now_ms(),
        };
        let reservation = match self
            .effects
            .admit(effect_request)
            .await
            .map_err(effect_ledger_error)?
        {
            RobotEffectAdmission::Dispatch(reservation) => reservation,
            RobotEffectAdmission::Completed { output } => {
                return Ok(tool_output(&robot.robot_id, exposed_name, output, true));
            }
            RobotEffectAdmission::Failed { code, message } => {
                return Err(RobotModuleError::new(code, message, false));
            }
            RobotEffectAdmission::OutcomeUnknown { reason } => {
                return Err(RobotModuleError::new(
                    ROBOT_EFFECT_OUTCOME_UNKNOWN,
                    reason,
                    false,
                ));
            }
        };

        let result = self
            .tools
            .call_configured_for_action(
                &robot.robot_id,
                action,
                device_exposed_name,
                Some(expected_device_name),
                Some(expected_connection_id),
                Some(expected_input_schema),
                &robot,
                canonical_input.0["arguments"].clone(),
            )
            .await;
        match result {
            Ok(output) => {
                let output = bounded_tool_result(output);
                self.effects
                    .complete(&reservation, output.clone(), now_ms())
                    .await
                    .map_err(|error| {
                        RobotModuleError::new(
                            ROBOT_EFFECT_RECEIPT_FAILED,
                            format!(
                                "physical effect completed but its receipt could not be persisted: {error}"
                            ),
                            false,
                        )
                    })?;
                Ok(tool_output(&robot.robot_id, exposed_name, output, false))
            }
            Err(ToolCallError::Rejected(message)) => {
                settle_known_failure(
                    &self.effects,
                    &reservation,
                    ROBOT_DEVICE_REJECTED,
                    &message,
                )
                .await?;
                Err(RobotModuleError::new(
                    ROBOT_DEVICE_REJECTED,
                    message,
                    false,
                ))
            }
            Err(ToolCallError::Failed(message)) => {
                settle_known_failure(
                    &self.effects,
                    &reservation,
                    ROBOT_EFFECT_FAILED,
                    &message,
                )
                .await?;
                Err(RobotModuleError::new(
                    ROBOT_EFFECT_FAILED,
                    message,
                    false,
                ))
            }
            Err(error @ (ToolCallError::Offline | ToolCallError::Timeout)) => {
                let reason = error.to_string();
                self.effects
                    .mark_outcome_unknown(&reservation, reason.clone(), now_ms())
                    .await
                    .map_err(effect_ledger_error)?;
                Err(RobotModuleError::new(
                    ROBOT_EFFECT_OUTCOME_UNKNOWN,
                    reason,
                    false,
                ))
            }
        }
    }
}

#[async_trait]
impl NomiHostDynamicToolInvoker for BoundRobotSessionToolInvoker {
    async fn invoke(
        &self,
        request: NomiHostDynamicToolInvocation,
    ) -> Result<StrictJsonValue, NomiHostDynamicToolError> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(NomiHostDynamicToolError::new(
                "ROBOT_SESSION_REVOKED",
                "Robot Session resources have been closed",
                false,
            ));
        }
        if request.capability_id.as_ref() != ROBOT_MODULE_ID {
            return Err(NomiHostDynamicToolError::new(
                ROBOT_ACTION_NOT_GRANTED,
                "Robot tool invocation does not belong to the robot Module",
                false,
            ));
        }
        let frozen = self.tools.get(&request.provider_name).ok_or_else(|| {
            NomiHostDynamicToolError::new(
                "ROBOT_SESSION_TOOL_NOT_BOUND",
                format!(
                    "Robot Session tool {} was not frozen into this AgentSession",
                    request.provider_name
                ),
                false,
            )
        })?;
        if !request.arguments.0.is_object() || !frozen.validator.is_valid(&request.arguments.0) {
            return Err(NomiHostDynamicToolError::new(
                ROBOT_INVALID_PAYLOAD,
                "Robot Session arguments do not match the frozen device schema",
                false,
            ));
        }
        let arguments = if let Some(fixed) = &frozen.fixed_arguments {
            serde_json::Value::Object(fixed.clone())
        } else {
            let mut merged = frozen.default_arguments.clone();
            merged.extend(request.arguments.0.as_object().expect("validated object").clone());
            serde_json::Value::Object(merged)
        };
        if !frozen.raw_validator.is_valid(&arguments) {
            return Err(NomiHostDynamicToolError::new(ROBOT_INVALID_PAYLOAD,
                "Configured arguments do not match the device's original schema", false));
        }
        self.owner
            .execute_bound_device_tool(
                &self.principal,
                &self.agent_session_id,
                &request.operation_id,
                &request.idempotency_key,
                &request.correlation_id,
                &self.binding,
                frozen.action,
                &request.provider_name,
                &frozen.device_exposed_name,
                &frozen.device_name,
                &self.connection_id,
                &frozen.raw_input_schema,
                arguments,
            )
            .await
            .map_err(dynamic_tool_error)
    }
}

fn dynamic_tool_error(error: RobotModuleError) -> NomiHostDynamicToolError {
    NomiHostDynamicToolError::new(error.code, error.message, error.retry_safe)
}

fn exact_robot_binding<'a>(
    bindings: &'a [TypedResourceBinding],
    principal_id: &str,
) -> Result<&'a TypedResourceBinding, RobotModuleError> {
    let mut matching = bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == ROBOT_RESOURCE_KIND);
    let binding = matching.next().ok_or_else(|| {
        RobotModuleError::not_bound(
            "Pair a robot and bind that device to this Agent before enabling Robot actions.",
        )
    })?;
    if matching.next().is_some() {
        return Err(RobotModuleError::not_bound(
            "More than one Robot resource is bound; choose one exact device.",
        ));
    }
    if binding.owner_id != principal_id {
        return Err(RobotModuleError::owner_mismatch(
            "Robot resource binding belongs to another principal",
        ));
    }
    Ok(binding)
}

pub(crate) fn bound_robot_resource<'a>(
    bindings: &'a [TypedResourceBinding],
    principal_id: &str,
) -> Result<&'a TypedResourceBinding, RobotModuleError> {
    exact_robot_binding(bindings, principal_id)
}

fn validate_provider_tool_name(name: &str) -> Result<(), RobotModuleError> {
    if name.is_empty()
        || name.len() > 64
        || !name.starts_with("robot_")
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(RobotModuleError::invalid(format!(
            "Robot tool {name:?} cannot be represented as a provider-safe exact tool name"
        )));
    }
    Ok(())
}

fn bounded_description(description: &str) -> String {
    let trimmed = description.trim();
    let bounded = if trimmed.len() <= MAX_DEVICE_TOOL_DESCRIPTION_BYTES {
        trimmed.to_owned()
    } else {
        trimmed
            .chars()
            .scan(0usize, |bytes, character| {
                let next = bytes.saturating_add(character.len_utf8());
                (next <= MAX_DEVICE_TOOL_DESCRIPTION_BYTES).then(|| {
                    *bytes = next;
                    character
                })
            })
            .collect()
    };
    if bounded.is_empty() {
        "Tool exposed by the robot bound to this AgentSession.".to_owned()
    } else {
        bounded
    }
}

fn strict_device_input_schema(
    mut schema: serde_json::Value,
) -> Result<StrictJsonValue, RobotModuleError> {
    if serde_json::to_vec(&schema)
        .map_err(|error| RobotModuleError::invalid(error.to_string()))?
        .len()
        > MAX_DEVICE_TOOL_SCHEMA_BYTES
    {
        return Err(RobotModuleError::invalid(format!(
            "Robot tool schema exceeds the {MAX_DEVICE_TOOL_SCHEMA_BYTES}-byte Session limit"
        )));
    }
    harden_object_schemas(&mut schema, "$input")?;
    let root = schema
        .as_object()
        .ok_or_else(|| RobotModuleError::invalid("Robot tool input schema must be a JSON object"))?;
    if root.get("type").and_then(serde_json::Value::as_str) != Some("object") {
        return Err(RobotModuleError::invalid(
            "Robot tool input schema root type must be object",
        ));
    }
    match root.get("properties") {
        Some(serde_json::Value::Object(_)) | None => {}
        _ => {
            return Err(RobotModuleError::invalid(
                "Robot tool input schema properties must be an object",
            ));
        }
    }
    Ok(StrictJsonValue(schema))
}

struct NoExternalDeviceSchema;

impl jsonschema::Retrieve for NoExternalDeviceSchema {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("Robot schemas cannot retrieve external resources".into())
    }
}

fn harden_object_schemas(
    value: &mut serde_json::Value,
    path: &str,
) -> Result<(), RobotModuleError> {
    match value {
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                harden_object_schemas(item, &format!("{path}[{index}]"))?;
            }
        }
        serde_json::Value::Object(object) => {
            if object.contains_key("$ref") {
                return Err(RobotModuleError::invalid(format!(
                    "Robot tool schema {path} contains an external schema reference"
                )));
            }
            let object_schema = object.get("type").and_then(serde_json::Value::as_str)
                == Some("object")
                || object.contains_key("properties");
            if object_schema {
                if let Some(properties) = object.get("properties")
                    && !properties.is_object()
                {
                    return Err(RobotModuleError::invalid(format!(
                        "Robot tool schema {path}.properties must be an object"
                    )));
                }
                object.insert(
                    "additionalProperties".to_owned(),
                    serde_json::Value::Bool(false),
                );
            }
            for (key, child) in object.iter_mut() {
                if key != "description" && key != "title" && key != "default" && key != "enum" {
                    harden_object_schemas(child, &format!("{path}.{key}"))?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn effect_ledger_error(error: anyhow::Error) -> RobotModuleError {
    let message = error.to_string();
    let code = if message.contains("reused with a different request") {
        "ROBOT_EFFECT_IDEMPOTENCY_CONFLICT"
    } else {
        ROBOT_EFFECT_RECEIPT_FAILED
    };
    RobotModuleError::new(code, message, false)
}

async fn settle_known_failure(
    ledger: &RobotEffectLedger,
    reservation: &nomifun_robot::effect_ledger::RobotEffectReservation,
    code: &str,
    message: &str,
) -> Result<(), RobotModuleError> {
    ledger
        .fail(
            reservation,
            code.to_owned(),
            message.to_owned(),
            now_ms(),
        )
        .await
        .map_err(effect_ledger_error)
}

fn bounded_tool_result(output: String) -> String {
    if output.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return output;
    }
    output.chars().take(MAX_TOOL_RESULT_CHARS).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn tool_output(
    robot_id: &str,
    tool_name: &str,
    result: String,
    replayed: bool,
) -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({
        "robot_id": robot_id,
        "tool_name": tool_name,
        "result": result,
        "replayed": replayed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        CorrelationId, ResourceBindingId, ResourceId, ResourceKind,
    };
    use nomifun_robot::link::Frame;
    use nomifun_robot::mcp_bridge::{RobotMcpClient, RobotToolDescriptor};
    use nomifun_robot::registry::{RobotPermissions, RobotReport};
    use tokio::sync::mpsc;

    struct Fixture {
        _dir: tempfile::TempDir,
        owner: Arc<RobotModuleOwner>,
        binding: TypedResourceBinding,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }

    async fn fixture(permissions: RobotPermissions) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (reported, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "robot-1".to_owned(),
                    client_id: "client-1".to_owned(),
                    board: "test".to_owned(),
                    firmware_version: "1".to_owned(),
                },
                1,
            )
            .await
            .unwrap();
        registry
            .claim(
                reported.activation_code.as_deref().unwrap(),
                "companion-1",
            )
            .await
            .unwrap();
        registry
            .set_permissions("robot-1", permissions)
            .await
            .unwrap();
        registry
            .connect("robot-1", "companion-1", "socket-1")
            .await
            .unwrap();
        let tools = Arc::new(RobotToolRegistry::default());
        let (tx, mut rx) = mpsc::channel(8);
        let client = Arc::new(RobotMcpClient::new(tx, "socket-1".to_owned()));
        let responder = Arc::clone(&client);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorded = Arc::clone(&calls);
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            while let Some(Frame::Text(frame)) = rx.recv().await {
                recorded.fetch_add(1, Ordering::SeqCst);
                let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
                captured.lock().unwrap().push(value["payload"]["params"].clone());
                responder
                    .handle_incoming(serde_json::json!({
                        "jsonrpc":"2.0",
                        "id":value["payload"]["id"],
                        "result":{"content":[{"type":"text","text":"ok"}],"isError":false}
                    }))
                    .await;
            }
        });
        tools
            .attach(
                "robot-1",
                client,
                vec![
                    RobotToolDescriptor {
                        device_name: "self.gimbal.look".to_owned(),
                        exposed_name: "robot_gimbal_look".to_owned(),
                        description: "Look in a direction".to_owned(),
                        input_schema: serde_json::json!({
                            "type":"object",
                            "properties":{"direction":{"type":"string"}},
                            "required":["direction"]
                        }),
                    },
                    RobotToolDescriptor {
                        device_name: "self.head.express".to_owned(),
                        exposed_name: "robot_head_express".to_owned(),
                        description: "Queue an expressive face and head gesture".to_owned(),
                        input_schema: serde_json::json!({"type":"object","properties":{
                            "emotion":{"type":"string","enum":["happy","sad","surprise","thinking","neutral"]},
                            "action":{"type":"string","enum":["none","nod","shake","left","right","up","down","center"]}
                        }}),
                    },
                    RobotToolDescriptor {
                        device_name: "self.display.text".to_owned(),
                        exposed_name: "robot_display_text".to_owned(),
                        description: "Show text".to_owned(),
                        input_schema: serde_json::json!({"type":"object"}),
                    },
                    RobotToolDescriptor {
                        device_name: "self.dog.action".to_owned(),
                        exposed_name: "robot_dog_action".to_owned(),
                        description: "Control the dog legs".to_owned(),
                        input_schema: serde_json::json!({
                            "type":"object", "properties":{"action":{"type":"string"}},
                            "required":["action"]
                        }),
                    },
                    RobotToolDescriptor {
                        device_name: "self.camera.take_photo".to_owned(),
                        exposed_name: "robot_camera_take_photo".to_owned(),
                        description: "Take a photo".to_owned(),
                        input_schema: serde_json::json!({"type":"object"}),
                    },
                    RobotToolDescriptor {
                        device_name: "self.extension.read".to_owned(),
                        exposed_name: "robot_extension_read".to_owned(),
                        description: "Read device extension status".to_owned(),
                        input_schema: serde_json::json!({"type":"object"}),
                    },
                    RobotToolDescriptor {
                        device_name: "vendor.arm.pose".to_owned(),
                        exposed_name: "robot_vendor_arm_pose".to_owned(),
                        description: "Vendor arm primitive".to_owned(),
                        input_schema: serde_json::json!({"type":"object","properties":{
                            "angle":{"type":"integer","minimum":-30,"maximum":30}
                        },"required":["angle"],"additionalProperties":false}),
                    },
                ],
            )
            .await;
        let owner = Arc::new(RobotModuleOwner::from_parts(
            Arc::from("owner"),
            Arc::clone(&registry),
            tools,
            Arc::new(RobotEffectLedger::load(dir.path()).await.unwrap()),
            Arc::new(RobotVisionObservationRegistry::default()),
        ));
        Fixture {
            _dir: dir,
            owner,
            binding: binding(["vision", "display", "motion", "device"]),
            calls,
            requests,
        }
    }

    fn binding(operations: impl IntoIterator<Item = &'static str>) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("robot-binding"),
            resource_kind: ResourceKind::from(ROBOT_RESOURCE_KIND),
            resource_id: ResourceId::from("robot-1"),
            owner_id: "owner".to_owned(),
            operations: operations.into_iter().map(str::to_owned).collect(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "companion_id".to_owned(),
                "companion-1".to_owned(),
            )]),
        }
    }

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        }
    }

    fn permissions() -> RobotPermissions {
        RobotPermissions {
            vision: true,
            motion: true,
            display: true,
            device_tools: true,
            proactive_speech: false,
            continuous_vision: false,
        }
    }

    fn invocation(name: &str, key: &str) -> NomiHostDynamicToolInvocation {
        NomiHostDynamicToolInvocation {
            capability_id: module_capability_id(),
            provider_name: name.to_owned(),
            operation_id: OperationId::from(format!("operation-{key}")),
            idempotency_key: IdempotencyKey::from(key),
            correlation_id: CorrelationId::from(format!("correlation-{key}")),
            arguments: StrictJsonValue(serde_json::json!({"direction":"left"})),
        }
    }

    #[tokio::test]
    async fn one_module_materializes_only_exact_granted_actions() {
        let fixture = fixture(permissions()).await;
        let tools = fixture
            .owner
            .resolve_session_tools(
                &principal(),
                &AgentSessionId::from("session-1"),
                &fixture.binding,
                &BTreeSet::from([
                    ActionId::from(nomifun_robot::capability::ROBOT_VISION_ACTION_ID),
                    ActionId::from(nomifun_robot::capability::ROBOT_MOTION_ACTION_ID),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(tools.descriptors.len(), 4);
        assert!(tools.descriptors.iter().all(|descriptor| matches!(
            descriptor.action_id.as_ref(),
            nomifun_robot::capability::ROBOT_VISION_ACTION_ID
                | nomifun_robot::capability::ROBOT_MOTION_ACTION_ID
        )));
        assert_eq!(module_capability_id().as_ref(), ROBOT_MODULE_ID);
    }

    #[tokio::test]
    async fn dog_motion_is_discovered_and_dispatched_without_device_tool_authority() {
        let mut permitted = permissions();
        permitted.device_tools = false;
        let fixture = fixture(permitted.clone()).await;
        let tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("dog-session"), &fixture.binding,
            &BTreeSet::from([ActionId::from(RobotAction::Motion.id())]),
        ).await.unwrap();
        let dog = tools.descriptors.iter()
            .find(|descriptor| descriptor.provider_name == "robot_dog_action").unwrap();
        assert_eq!(dog.action_id.as_ref(), RobotAction::Motion.id());
        let mut request = invocation("robot_dog_action", "dog-sit");
        request.arguments = StrictJsonValue(serde_json::json!({"action":"sitdown"}));
        tools.invoker.invoke(request).await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.requests.lock().unwrap()[0]["name"], "self.dog.action");

        permitted.motion = false;
        permitted.device_tools = true;
        fixture.owner.registry.set_permissions("robot-1", permitted).await.unwrap();
        let mut request = invocation("robot_dog_action", "dog-revoked");
        request.arguments = StrictJsonValue(serde_json::json!({"action":"sitdown"}));
        let denied = tools.invoker.invoke(request).await.unwrap_err();
        assert_eq!(denied.code.as_ref(), ROBOT_PERMISSION_DENIED);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);

        let device_tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("device-session"), &fixture.binding,
            &BTreeSet::from([ActionId::from(RobotAction::Device.id())]),
        ).await.unwrap();
        assert!(!device_tools.descriptors.iter()
            .any(|descriptor| descriptor.provider_name == "robot_dog_action"));
        let mut request = invocation("robot_dog_action", "dog-not-bound");
        request.arguments = StrictJsonValue(serde_json::json!({"action":"sitdown"}));
        let denied = device_tools.invoker.invoke(request).await.unwrap_err();
        assert_eq!(denied.code.as_ref(), "ROBOT_SESSION_TOOL_NOT_BOUND");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn expressive_tools_keep_original_schema_and_motion_authority() {
        let fixture = fixture(permissions()).await;
        let actions = BTreeSet::from([ActionId::from(RobotAction::Motion.id())]);
        let tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding, &actions,
        ).await.unwrap();
        let tool = tools.descriptors.iter().find(|tool| tool.provider_name == "robot_head_express").unwrap();
        assert_eq!(tool.action_id.as_ref(), RobotAction::Motion.id());
        assert_eq!(tool.description, "Queue an expressive face and head gesture");
        let mut request = invocation("robot_head_express", "express-happy");
        request.arguments = StrictJsonValue(serde_json::json!({"emotion":"happy","action":"nod"}));
        tools.invoker.invoke(request).await.unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        let mut invalid = invocation("robot_head_express", "invalid-emotion");
        invalid.arguments = StrictJsonValue(serde_json::json!({"emotion":"invented","action":"nod"}));
        assert_eq!(tools.invoker.invoke(invalid).await.unwrap_err().code.as_ref(), ROBOT_INVALID_PAYLOAD);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1, "invalid expression must not reach the device");

        let mut permitted = permissions();
        permitted.motion = false;
        fixture.owner.registry.set_permissions("robot-1", permitted).await.unwrap();
        let blocked = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding, &actions,
        ).await.unwrap();
        assert!(blocked.descriptors.is_empty());
        let mut expression = invocation("robot_head_express", "after-motion-revoked");
        expression.arguments = StrictJsonValue(serde_json::json!({"emotion":"happy","action":"nod"}));
        assert_eq!(tools.invoker.invoke(expression).await.unwrap_err().code.as_ref(), ROBOT_PERMISSION_DENIED);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn arbitrary_device_profiles_defaults_and_named_actions_use_the_same_effect_path() {
        use nomifun_robot::capability_profile::{RobotCapabilityProfile, RobotCapabilityPreset, RobotToolOverride};
        let fixture = fixture(permissions()).await;
        let actions = BTreeSet::from([ActionId::from(RobotAction::Motion.id())]);
        let old_tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding, &actions,
        ).await.unwrap();
        let mut profile = RobotCapabilityProfile::default();
        profile.tool_overrides.insert("vendor.arm.pose".into(), RobotToolOverride {
            action: Some(RobotAction::Motion), description: Some("Use a small arm gesture to greet the user".into()),
            default_arguments: serde_json::json!({"angle":10}).as_object().unwrap().clone(),
            ..Default::default()
        });
        profile.presets.push(RobotCapabilityPreset {
            id:"greeting".into(), name:"Greeting".into(), tool_name:"vendor.arm.pose".into(),
            description:"A brief greeting gesture".into(),
            arguments:serde_json::json!({"angle":15}).as_object().unwrap().clone(), enabled:true,
        });
        let saved = fixture.owner.registry.set_capability_profile(
            "robot-1", 0, profile, &fixture.owner.tools,
        ).await.unwrap();
        assert_eq!(saved.capability_revision, 1);
        let old = old_tools.invoker.invoke(invocation("robot_gimbal_look", "stale-profile")).await.unwrap_err();
        assert_eq!(old.code.as_ref(), "ROBOT_CAPABILITIES_CHANGED");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        let mut current_binding = fixture.binding.clone();
        current_binding.typed_parameters.insert("capability_revision".into(), "1".into());
        let current = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &current_binding, &actions,
        ).await.unwrap();
        let tool = current.descriptors.iter().find(|tool| tool.provider_name == "robot_vendor_arm_pose").unwrap();
        assert_eq!(tool.description, "Use a small arm gesture to greet the user");
        assert_eq!(tool.input_schema.0["properties"]["angle"]["default"], 10);
        for name in ["robot_vendor_arm_pose", "robot_preset_greeting"] {
            let mut request = invocation(name, name);
            request.arguments = StrictJsonValue(serde_json::json!({}));
            current.invoker.invoke(request).await.unwrap();
        }
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests[0]["name"], "vendor.arm.pose");
        assert_eq!(requests[0]["arguments"], serde_json::json!({"angle":10}));
        assert_eq!(requests[1]["name"], "vendor.arm.pose");
        assert_eq!(requests[1]["arguments"], serde_json::json!({"angle":15}));
        drop(requests);
        let mut invalid = invocation("robot_vendor_arm_pose", "out-of-range-profile-argument");
        invalid.arguments = StrictJsonValue(serde_json::json!({"angle":31}));
        assert_eq!(current.invoker.invoke(invalid).await.unwrap_err().code.as_ref(), ROBOT_INVALID_PAYLOAD);
        let mut override_fixed = invocation("robot_preset_greeting", "preset-override");
        override_fixed.arguments = StrictJsonValue(serde_json::json!({"angle":-30}));
        assert_eq!(current.invoker.invoke(override_fixed).await.unwrap_err().code.as_ref(), ROBOT_INVALID_PAYLOAD);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);

        let display_only = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &current_binding,
            &BTreeSet::from([ActionId::from(RobotAction::Display.id())]),
        ).await.unwrap();
        assert!(display_only.descriptors.iter().all(|tool| tool.provider_name != "robot_preset_greeting"));
    }

    #[tokio::test]
    async fn disabled_optional_vision_does_not_block_display_and_device_tools() {
        let mut permitted = permissions();
        permitted.vision = false;
        permitted.motion = false;
        let fixture = fixture(permitted).await;
        let actions = RobotAction::ALL.into_iter().map(|action| ActionId::from(action.id())).collect();
        assert!(fixture.owner.vision_context("owner", &fixture.binding, &actions).await.unwrap().is_none());
        let tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding, &actions,
        ).await.unwrap();
        let names = tools.descriptors.iter().map(|tool| tool.provider_name.as_str()).collect::<BTreeSet<_>>();
        assert_eq!(names, BTreeSet::from(["robot_display_text", "robot_extension_read", "robot_vendor_arm_pose"]));
        for (name, key) in [("robot_display_text", "display"), ("robot_extension_read", "device")] {
            let mut request = invocation(name, key);
            request.arguments = StrictJsonValue(serde_json::json!({}));
            tools.invoker.invoke(request).await.unwrap();
        }
        let denied = tools.invoker.invoke(invocation("robot_camera_take_photo", "camera")).await.unwrap_err();
        assert_eq!(denied.code.as_ref(), "ROBOT_SESSION_TOOL_NOT_BOUND");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn revoked_vision_omits_cached_context_but_still_denies_explicit_camera_dispatch() {
        let fixture = fixture(permissions()).await;
        let actions = BTreeSet::from([ActionId::from(RobotAction::Vision.id())]);
        let tools = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding, &actions,
        ).await.unwrap();
        fixture.owner.observations.record(nomifun_robot::vision::RobotVisionObservation {
            source: nomifun_robot::vision::RobotVisionSource {
                companion_id: "companion-1".into(), conversation_id: "session-1".into(),
                connection_id: "socket-1".into(), request_id: "observation-1".into(),
            },
            robot_id: "robot-1".into(), companion_id: "companion-1".into(),
            question: "What is visible?".into(), answer: "Private observation".into(), observed_at_ms: now_ms(),
        }).await;
        assert!(fixture.owner.vision_context("owner", &fixture.binding, &actions).await.unwrap().is_some());
        let mut permitted = permissions();
        permitted.vision = false;
        fixture.owner.registry.set_permissions("robot-1", permitted).await.unwrap();
        assert!(fixture.owner.vision_context("owner", &fixture.binding, &actions).await.unwrap().is_none());
        let mut request = invocation("robot_camera_take_photo", "after-revoke");
        request.arguments = StrictJsonValue(serde_json::json!({}));
        let denied = tools.invoker.invoke(request).await.unwrap_err();
        assert_eq!(denied.code.as_ref(), ROBOT_PERMISSION_DENIED);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disabled_vision_still_rejects_invalid_owner_companion_and_action_authority() {
        let mut permitted = permissions();
        permitted.vision = false;
        let fixture = fixture(permitted).await;
        let actions = BTreeSet::from([ActionId::from(RobotAction::Vision.id())]);
        assert!(fixture.owner.vision_context("other-owner", &fixture.binding, &actions).await.is_err());
        let mut forged = fixture.binding.clone();
        forged.owner_id = "other-owner".into();
        assert!(fixture.owner.vision_context("owner", &forged, &actions).await.is_err());
        let mut forged = fixture.binding.clone();
        forged.typed_parameters.insert("companion_id".into(), "other-companion".into());
        assert!(fixture.owner.vision_context("owner", &forged, &actions).await.is_err());
        let ungranted = fixture.owner.vision_context("owner", &fixture.binding, &BTreeSet::new()).await.unwrap_err();
        assert_eq!(ungranted.code.as_str(), ROBOT_ACTION_NOT_GRANTED);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn physical_dispatch_replays_receipt_and_revocation_is_terminal() {
        let fixture = fixture(permissions()).await;
        let tools = fixture
            .owner
            .resolve_session_tools(
                &principal(),
                &AgentSessionId::from("session-1"),
                &fixture.binding,
                &BTreeSet::from([ActionId::from(
                    nomifun_robot::capability::ROBOT_MOTION_ACTION_ID,
                )]),
            )
            .await
            .unwrap();
        let first = tools
            .invoker
            .invoke(invocation("robot_gimbal_look", "same"))
            .await
            .unwrap();
        assert_eq!(first.0["replayed"], false);
        let replay = tools
            .invoker
            .invoke(invocation("robot_gimbal_look", "same"))
            .await
            .unwrap();
        assert_eq!(replay.0["replayed"], true);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        tools.revoke();
        let error = tools
            .invoker
            .invoke(invocation("robot_gimbal_look", "after-close"))
            .await
            .unwrap_err();
        assert_eq!(error.code.as_ref(), "ROBOT_SESSION_REVOKED");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mutable_device_permission_rechecks_before_dispatch() {
        let fixture = fixture(permissions()).await;
        let tools = fixture
            .owner
            .resolve_session_tools(
                &principal(),
                &AgentSessionId::from("session-1"),
                &fixture.binding,
                &BTreeSet::from([ActionId::from(
                    nomifun_robot::capability::ROBOT_MOTION_ACTION_ID,
                )]),
            )
            .await
            .unwrap();
        let mut permissions = permissions();
        permissions.motion = false;
        fixture
            .owner
            .registry
            .set_permissions("robot-1", permissions)
            .await
            .unwrap();
        let error = tools
            .invoker
            .invoke(invocation("robot_gimbal_look", "revoked"))
            .await
            .unwrap_err();
        assert_eq!(error.code.as_ref(), ROBOT_PERMISSION_DENIED);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconnect_does_not_redirect_a_frozen_session_action() {
        let fixture = fixture(permissions()).await;
        let tools = fixture
            .owner
            .resolve_session_tools(
                &principal(),
                &AgentSessionId::from("session-1"),
                &fixture.binding,
                &BTreeSet::from([ActionId::from(
                    nomifun_robot::capability::ROBOT_MOTION_ACTION_ID,
                )]),
            )
            .await
            .unwrap();
        fixture
            .owner
            .registry
            .connect("robot-1", "companion-1", "socket-2")
            .await
            .unwrap();
        let error = tools
            .invoker
            .invoke(invocation("robot_gimbal_look", "reconnected"))
            .await
            .unwrap_err();
        assert_eq!(error.code.as_ref(), ROBOT_OFFLINE);
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);

        let descriptors = fixture.owner.tools.tools("robot-1").await;
        let (tx, mut rx) = mpsc::channel(8);
        let client = Arc::new(RobotMcpClient::new(tx, "socket-2".to_owned()));
        let responder = client.clone();
        let recorded = fixture.calls.clone();
        tokio::spawn(async move {
            while let Some(Frame::Text(frame)) = rx.recv().await {
                recorded.fetch_add(1, Ordering::SeqCst);
                let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
                responder.handle_incoming(serde_json::json!({
                    "id":value["payload"]["id"],
                    "result":{"content":[{"type":"text","text":"new connection"}],"isError":false}
                })).await;
            }
        });
        fixture.owner.tools.attach("robot-1", client, descriptors).await;
        let refreshed = fixture.owner.resolve_session_tools(
            &principal(), &AgentSessionId::from("session-1"), &fixture.binding,
            &BTreeSet::from([ActionId::from(RobotAction::Motion.id())]),
        ).await.unwrap();
        let old = tools.invoker.invoke(invocation("robot_gimbal_look", "still-old")).await.unwrap_err();
        assert_eq!(old.code.as_ref(), ROBOT_OFFLINE, "a fresh owner must not redirect the old invoker");
        let current = refreshed.invoker.invoke(invocation("robot_gimbal_look", "fresh-connection")).await.unwrap();
        assert_eq!(current.0["result"], "new connection");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn schemas_fail_closed_without_external_refs_or_extra_fields() {
        let hardened = strict_device_input_schema(serde_json::json!({
            "type":"object",
            "properties":{"direction":{"type":"string"}}
        }))
        .unwrap();
        assert_eq!(hardened.0["additionalProperties"], false);
        assert!(
            strict_device_input_schema(serde_json::json!({
                "type":"object",
                "properties":{"payload":{"$ref":"https://example.invalid/schema"}}
            }))
            .is_err()
        );
    }
}
