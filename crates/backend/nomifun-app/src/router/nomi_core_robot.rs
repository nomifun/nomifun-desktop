//! Target-bound Nomi-core owner for the six bundled Robot capabilities.
//!
//! The owner reuses the production Robot registry, authenticated device MCP
//! links, ASR/TTS/vision pipeline, and durable physical-effect ledger. It never
//! selects a default robot and never accepts a device/tool name outside the
//! exact Session resource and capability ceiling.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;
use nomifun_agent_contracts::{
    AgentSessionId, CapabilityId, CorrelationId, EffectClass, IdempotencyKey,
    OperationId, PrincipalRef, StrictJsonValue, TypedResourceBinding, digest_payload,
};
use nomifun_ai_agent::{
    NomiHostDynamicToolDescriptor, NomiHostDynamicToolError, NomiHostDynamicToolInvocation,
    NomiHostDynamicToolInvoker,
};
use nomifun_agent_domain_wave4::{
    ROBOT_AUDIO, ROBOT_DEVICE_TOOLS, ROBOT_DISPLAY, ROBOT_LINK, ROBOT_MOTION,
    ROBOT_RESOURCE_KIND, ROBOT_VISION, Wave4CapabilityOperation,
    Wave4ContextHostPort, Wave4ContextHostRequest, Wave4HostPort,
    Wave4HostPortError, Wave4HostRequest,
};
use nomifun_robot::effect_ledger::{
    RobotEffectAdmission, RobotEffectKey, RobotEffectLedger, RobotEffectRequest,
};
use nomifun_robot::mcp_bridge::ToolCallError;
use nomifun_robot::registry::{RobotRecord, RobotRegistry};
use nomifun_robot::status::RobotStatusRegistry;
use nomifun_robot::tool_registry::{RobotToolCapability, RobotToolRegistry};
use nomifun_robot::vision::RobotVisionObservationRegistry;

const ROBOT_NOT_FOUND: &str = "ROBOT_NOT_FOUND";
const ROBOT_NOT_PAIRED: &str = "ROBOT_NOT_PAIRED";
const ROBOT_OFFLINE: &str = "ROBOT_OFFLINE";
const ROBOT_DEVICE_REJECTED: &str = "ROBOT_DEVICE_REJECTED";
const ROBOT_EFFECT_FAILED: &str = "ROBOT_EFFECT_FAILED";
const ROBOT_EFFECT_OUTCOME_UNKNOWN: &str = "ROBOT_EFFECT_OUTCOME_UNKNOWN";
const ROBOT_EFFECT_RECEIPT_FAILED: &str = "ROBOT_EFFECT_RECEIPT_FAILED";
const MAX_TOOL_NAME_BYTES: usize = 512;
const MAX_TOOL_RESULT_CHARS: usize = 65_536;
const MAX_DEVICE_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
const MAX_DEVICE_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;

#[derive(Clone)]
struct FrozenRobotDeviceTool {
    capability: RobotToolCapability,
    device_name: String,
}

struct BoundRobotSessionToolInvoker {
    owner: Arc<NomiCoreRobotWave4Owner>,
    principal: PrincipalRef,
    agent_session_id: AgentSessionId,
    binding: TypedResourceBinding,
    robot_id: String,
    tools: BTreeMap<(CapabilityId, String), FrozenRobotDeviceTool>,
}

#[derive(Clone, Debug)]
struct RobotLifecycleActivation {
    principal_id: String,
    agent_session_id: AgentSessionId,
    capability_id: CapabilityId,
    binding_id: String,
    robot_id: String,
    companion_id: String,
    cancelled: Arc<AtomicBool>,
}

struct RobotLifecycleLease {
    key: String,
    activation: RobotLifecycleActivation,
    activations: Arc<DashMap<String, RobotLifecycleActivation>>,
    registry: Arc<RobotRegistry>,
    status: Arc<RobotStatusRegistry>,
}

pub(crate) fn tool_capability_ids() -> BTreeSet<CapabilityId> {
    [ROBOT_DISPLAY, ROBOT_MOTION, ROBOT_DEVICE_TOOLS]
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

pub(crate) fn context_capability_ids() -> BTreeSet<CapabilityId> {
    BTreeSet::from([CapabilityId::from(ROBOT_VISION)])
}

pub(crate) fn lifecycle_capability_ids() -> BTreeSet<CapabilityId> {
    [ROBOT_LINK, ROBOT_AUDIO]
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RobotToolInput {
    tool_name: String,
    arguments: serde_json::Value,
}

/// Real owner used by the bundled Robot registration and Nomi lifecycle lane.
pub(crate) struct NomiCoreRobotWave4Owner {
    authoritative_user_id: Arc<str>,
    registry: Arc<RobotRegistry>,
    status: Arc<RobotStatusRegistry>,
    tools: Arc<RobotToolRegistry>,
    effects: Arc<RobotEffectLedger>,
    observations: Arc<RobotVisionObservationRegistry>,
    lifecycle_activations: Arc<DashMap<String, RobotLifecycleActivation>>,
}

impl NomiCoreRobotWave4Owner {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        robot: Arc<crate::robot_wiring::RobotServices>,
    ) -> Self {
        Self {
            authoritative_user_id,
            registry: Arc::clone(&robot.registry),
            status: Arc::clone(&robot.status),
            tools: Arc::clone(&robot.tools),
            effects: Arc::clone(&robot.effect_ledger),
            observations: Arc::clone(&robot.vision_observations),
            lifecycle_activations: Arc::new(DashMap::new()),
        }
    }

    #[cfg(test)]
    fn from_parts(
        authoritative_user_id: Arc<str>,
        registry: Arc<RobotRegistry>,
        status: Arc<RobotStatusRegistry>,
        tools: Arc<RobotToolRegistry>,
        effects: Arc<RobotEffectLedger>,
        observations: Arc<RobotVisionObservationRegistry>,
    ) -> Self {
        Self {
            authoritative_user_id,
            registry,
            status,
            tools,
            effects,
            observations,
            lifecycle_activations: Arc::new(DashMap::new()),
        }
    }

    /// Registration helper used by central bundled-owner composition.
    pub(crate) fn registration(
        owner: Arc<Self>,
    ) -> Result<nomifun_agent_kernel::PluginRegistration, String> {
        let action = Arc::clone(&owner) as Arc<dyn Wave4HostPort>;
        let context = owner as Arc<dyn Wave4ContextHostPort>;
        nomifun_agent_domain_wave4::robot_registration_with_host_ports(action, context)
    }

    /// Resolve exact live device tools for one already-authenticated and
    /// persisted AgentSession binding.
    ///
    /// Capability placement comes only from the compiled Snapshot. Resource
    /// identity and the device-side name are frozen here and retained by the
    /// returned invoker; neither is accepted from model arguments.
    pub(crate) async fn resolve_session_tools(
        self: &Arc<Self>,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        binding: &TypedResourceBinding,
        initial_capability_ids: &BTreeSet<CapabilityId>,
        deferred_capability_ids: &BTreeSet<CapabilityId>,
    ) -> Result<
        (
            Vec<NomiHostDynamicToolDescriptor>,
            Arc<dyn NomiHostDynamicToolInvoker>,
        ),
        String,
    > {
        self.ensure_installation_owner(&principal.principal_id)
            .map_err(|error| error.to_string())?;
        let robot = self
            .load_bound_robot(binding)
            .await
            .map_err(|error| error.to_string())?;

        let mut descriptors = Vec::new();
        let mut frozen = BTreeMap::new();
        for (capability_id, capability, required_operation) in [
            (ROBOT_DISPLAY, RobotToolCapability::Display, "display"),
            (ROBOT_MOTION, RobotToolCapability::Motion, "motion"),
            (ROBOT_DEVICE_TOOLS, RobotToolCapability::DeviceTools, "link"),
        ] {
            let capability_ref = CapabilityId::from(capability_id);
            let deferred = deferred_capability_ids.contains(&capability_ref);
            if !deferred && !initial_capability_ids.contains(&capability_ref) {
                continue;
            }
            if !binding.operations.contains(required_operation) {
                return Err(format!(
                    "Robot binding does not grant {required_operation} for {capability_id}"
                ));
            }
            for tool in self
                .tools
                .tools_for_capability(&robot.robot_id, capability)
                .await
            {
                validate_provider_tool_name(&tool.exposed_name)?;
                let input_schema = strict_device_input_schema(tool.input_schema)?;
                let description = bounded_description(&tool.description);
                let key = (capability_ref.clone(), tool.exposed_name.clone());
                if frozen
                    .insert(
                        key,
                        FrozenRobotDeviceTool {
                            capability,
                            device_name: tool.device_name,
                        },
                    )
                    .is_some()
                {
                    return Err(format!(
                        "Robot published duplicate tool {} for {capability_id}",
                        tool.exposed_name
                    ));
                }
                if descriptors.iter().any(|descriptor: &NomiHostDynamicToolDescriptor| {
                    descriptor.provider_name == tool.exposed_name
                }) {
                    return Err(format!(
                        "Robot tool names collide after provider-safe projection: {}",
                        tool.exposed_name
                    ));
                }
                descriptors.push(NomiHostDynamicToolDescriptor {
                    capability_id: capability_ref.clone(),
                    provider_name: tool.exposed_name,
                    description,
                    input_schema,
                    effect_class: EffectClass::Physical,
                    deferred,
                });
            }
        }
        descriptors.sort_by(|left, right| {
            (&left.capability_id, &left.provider_name)
                .cmp(&(&right.capability_id, &right.provider_name))
        });
        let invoker: Arc<dyn NomiHostDynamicToolInvoker> = Arc::new(
            BoundRobotSessionToolInvoker {
                owner: Arc::clone(self),
                principal: principal.clone(),
                agent_session_id: agent_session_id.clone(),
                binding: binding.clone(),
                robot_id: robot.robot_id,
                tools: frozen,
            },
        );
        Ok((descriptors, invoker))
    }

    /// Activate the ResourceProvider/BackgroundService contributions for one
    /// exact compiled Nomi Session.
    pub(crate) async fn activate_lifecycle(
        &self,
        request: nomifun_ai_agent::NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<StrictJsonValue, String> {
        let capability_id = request.capability.capability.id;
        self.activate_lifecycle_for(
            &capability_id,
            &request.principal.principal_id,
            &request.agent_session_id,
            &request.resource_bindings,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn activate_lifecycle_for(
        &self,
        capability_id: &CapabilityId,
        principal_id: &str,
        agent_session_id: &AgentSessionId,
        resource_bindings: &[TypedResourceBinding],
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        if !lifecycle_capability_ids().contains(capability_id) {
            return Err(Wave4HostPortError::action_operation_mismatch(format!(
                "WAVE4_ACTION_OPERATION_MISMATCH: {} is not a Robot lifecycle capability",
                capability_id.as_ref()
            )));
        }
        self.ensure_installation_owner(principal_id)?;
        let binding = exact_robot_binding(resource_bindings, principal_id)?;
        let robot = self.load_bound_robot(binding).await?;
        let online = self
            .status
            .snapshot()
            .await
            .into_iter()
            .any(|status| status.robot_id == robot.robot_id && status.phase != "offline");
        if !online {
            return Err(Wave4HostPortError::new(
                ROBOT_OFFLINE,
                "the bound robot is not connected",
            ));
        }
        let companion_id = robot
            .companion_id
            .clone()
            .expect("load_bound_robot requires a paired Companion");
        let key = lifecycle_key(principal_id, agent_session_id, capability_id);
        let activation = RobotLifecycleActivation {
            principal_id: principal_id.to_owned(),
            agent_session_id: agent_session_id.clone(),
            capability_id: capability_id.clone(),
            binding_id: binding.binding_id.as_ref().to_owned(),
            robot_id: robot.robot_id.clone(),
            companion_id,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        if let Some(existing) = self.lifecycle_activations.get(&key) {
            if existing.binding_id != activation.binding_id
                || existing.robot_id != activation.robot_id
                || existing.companion_id != activation.companion_id
                || existing.cancelled.load(Ordering::Acquire)
            {
                return Err(Wave4HostPortError::resource_owner_mismatch(
                    "Robot lifecycle activation no longer matches this Session binding",
                ));
            }
        } else {
            self.lifecycle_activations.insert(key, activation);
        }
        Ok(StrictJsonValue(serde_json::json!({
            "kind": capability_id.as_ref(),
            "robot_id": robot.robot_id,
            "companion_id": robot.companion_id,
            "state": "active",
        })))
    }

    /// Return the Session-retained lease for an already activated Robot
    /// lifecycle capability. The central Nomi lifecycle assembly stores this
    /// as a context contributor so `Drop` is the runtime disposal boundary.
    pub(crate) async fn lifecycle_context_contributor(
        &self,
        request: &nomifun_ai_agent::NomiPlatformBuiltinLifecycleInvocation,
    ) -> Result<Option<Arc<dyn nomifun_ai_agent::ContextContributor>>, String> {
        self.lifecycle_context_contributor_for(
            &request.capability.capability.id,
            &request.principal,
            &request.agent_session_id,
            &request.resource_bindings,
        )
        .await
    }

    async fn lifecycle_context_contributor_for(
        &self,
        capability_id: &CapabilityId,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        resource_bindings: &[TypedResourceBinding],
    ) -> Result<Option<Arc<dyn nomifun_ai_agent::ContextContributor>>, String> {
        if !lifecycle_capability_ids().contains(capability_id) {
            return Ok(None);
        }
        self.ensure_installation_owner(&principal.principal_id)
            .map_err(|error| error.to_string())?;
        let binding = exact_robot_binding(resource_bindings, &principal.principal_id)
            .map_err(|error| error.to_string())?;
        let robot = self
            .load_bound_robot(binding)
            .await
            .map_err(|error| error.to_string())?;
        let key = lifecycle_key(
            &principal.principal_id,
            agent_session_id,
            capability_id,
        );
        let activation = self
            .lifecycle_activations
            .get(&key)
            .map(|entry| entry.clone())
            .ok_or_else(|| {
                format!(
                    "Robot lifecycle {} must be activated before its Session lease is retained",
                    capability_id.as_ref()
                )
            })?;
        if activation.binding_id != binding.binding_id.as_ref()
            || activation.robot_id != robot.robot_id
            || activation.companion_id != robot.companion_id.as_deref().unwrap_or_default()
            || activation.cancelled.load(Ordering::Acquire)
        {
            return Err(
                "Robot lifecycle activation does not match the current owner/binding relationship"
                    .to_owned(),
            );
        }
        Ok(Some(Arc::new(RobotLifecycleLease {
            key,
            activation,
            activations: Arc::clone(&self.lifecycle_activations),
            registry: Arc::clone(&self.registry),
            status: Arc::clone(&self.status),
        })))
    }

    async fn ensure_lifecycle_active(
        &self,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        capability_id: &str,
        binding: &TypedResourceBinding,
        robot_id: &str,
    ) -> Result<(), Wave4HostPortError> {
        let capability_id = CapabilityId::from(capability_id);
        let key = lifecycle_key(&principal.principal_id, agent_session_id, &capability_id);
        let activation = self.lifecycle_activations.get(&key).ok_or_else(|| {
            Wave4HostPortError::new(
                ROBOT_OFFLINE,
                format!("{} must be active before a Robot device tool is used", capability_id.as_ref()),
            )
        })?;
        if activation.cancelled.load(Ordering::Acquire)
            || activation.binding_id != binding.binding_id.as_ref()
            || activation.robot_id != robot_id
            || activation.principal_id != principal.principal_id
            || activation.agent_session_id.as_ref() != agent_session_id.as_ref()
            || activation.capability_id != capability_id
        {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Robot lifecycle lease does not match the current Session binding",
            ));
        }
        // Re-check the mutable pairing boundary. A Session compiled before a
        // robot was rebound must lose use authority immediately.
        self.load_bound_robot(binding).await?;
        Ok(())
    }

    fn ensure_installation_owner(&self, principal_id: &str) -> Result<(), Wave4HostPortError> {
        if principal_id != self.authoritative_user_id.as_ref() {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Robot request principal is not the installation owner",
            ));
        }
        Ok(())
    }

    async fn load_bound_robot(
        &self,
        binding: &TypedResourceBinding,
    ) -> Result<RobotRecord, Wave4HostPortError> {
        let robot = self
            .registry
            .get(binding.resource_id.as_ref())
            .await
            .ok_or_else(|| {
                Wave4HostPortError::new(
                    ROBOT_NOT_FOUND,
                    "the bound robot resource does not exist in this installation",
                )
            })?;
        let companion_id = robot.companion_id.as_deref().ok_or_else(|| {
            Wave4HostPortError::new(
                ROBOT_NOT_PAIRED,
                "the bound robot is not paired with a Companion",
            )
        })?;
        let bound_companion = binding
            .typed_parameters
            .get("companion_id")
            .ok_or_else(|| {
                Wave4HostPortError::resource_not_bound(
                    "the Robot resource binding has no server-resolved companion_id",
                )
            })?;
        if companion_id != bound_companion {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "the Robot resource is no longer paired with the bound Companion",
            ));
        }
        Ok(robot)
    }

    async fn execute_tool(
        &self,
        request: &Wave4HostRequest,
        input: &StrictJsonValue,
        capability: RobotToolCapability,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        let parsed: RobotToolInput = serde_json::from_value(input.0.clone())
            .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))?;
        if parsed.tool_name.trim().is_empty()
            || parsed.tool_name != parsed.tool_name.trim()
            || parsed.tool_name.len() > MAX_TOOL_NAME_BYTES
            || !parsed.arguments.is_object()
        {
            return Err(Wave4HostPortError::invalid_request(
                "tool_name must be trimmed and non-empty, and arguments must be an object",
            ));
        }
        let binding = exact_robot_binding(
            &request.context.resource_bindings,
            &request.context.principal.principal_id,
        )?;
        self.execute_bound_device_tool(
            &request.context.principal,
            &request.context.agent_session_id,
            &request.context.operation_id,
            &request.context.idempotency_key,
            &request.context.correlation_id,
            binding,
            capability,
            &parsed.tool_name,
            None,
            parsed.arguments,
        )
        .await
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
        capability: RobotToolCapability,
        exposed_name: &str,
        expected_device_name: Option<&str>,
        arguments: serde_json::Value,
    ) -> Result<StrictJsonValue, Wave4HostPortError> {
        if !arguments.is_object() {
            return Err(Wave4HostPortError::invalid_request(
                "Robot tool arguments must be an object",
            ));
        }
        self.ensure_installation_owner(&principal.principal_id)?;
        if binding.owner_id != principal.principal_id
            || binding.resource_kind.as_ref() != ROBOT_RESOURCE_KIND
        {
            return Err(Wave4HostPortError::resource_owner_mismatch(
                "Robot Session tool binding does not belong to the authenticated principal",
            ));
        }
        let robot = self.load_bound_robot(binding).await?;
        if !self.tools.is_attached(&robot.robot_id).await {
            return Err(Wave4HostPortError::new(
                ROBOT_OFFLINE,
                "the bound robot is not connected",
            ));
        }

        let canonical_input = StrictJsonValue(serde_json::json!({
            "tool_name": exposed_name,
            "arguments": arguments,
        }));
        let input_digest = digest_payload(&canonical_input)
            .map_err(|error| Wave4HostPortError::invalid_request(error.to_string()))?;
        let effect_request = RobotEffectRequest {
            key: RobotEffectKey {
                principal_id: principal.principal_id.clone(),
                agent_session_id: agent_session_id.as_ref().to_owned(),
                capability_id: capability.capability_id().to_owned(),
                idempotency_key: idempotency_key.as_ref().to_owned(),
            },
            input_digest: input_digest.as_ref().to_owned(),
            robot_id: robot.robot_id.clone(),
            tool_name: exposed_name.to_owned(),
            reserved_at_ms: now_ms(),
        };
        let reservation = match self.effects.admit(effect_request).await.map_err(effect_ledger_error)? {
            RobotEffectAdmission::Dispatch(reservation) => reservation,
            RobotEffectAdmission::Completed { output } => {
                return Ok(tool_output(&robot.robot_id, exposed_name, output, true));
            }
            RobotEffectAdmission::Failed { code, message } => {
                return Err(Wave4HostPortError::new(code, message));
            }
            RobotEffectAdmission::OutcomeUnknown { reason } => {
                return Err(Wave4HostPortError::new(ROBOT_EFFECT_OUTCOME_UNKNOWN, reason));
            }
        };

        let result = self
            .tools
            .call_exact_for_capability(
                &robot.robot_id,
                capability,
                exposed_name,
                expected_device_name,
                canonical_input.0["arguments"].clone(),
            )
            .await;
        match result {
            Ok(output) => {
                let output = bounded_tool_result(output);
                self.effects
                    .complete(
                        &reservation,
                        output.clone(),
                        now_ms(),
                    )
                    .await
                    .map_err(|error| {
                        Wave4HostPortError::new(
                            ROBOT_EFFECT_RECEIPT_FAILED,
                            format!("physical effect completed but its receipt could not be persisted: {error}"),
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
                Err(Wave4HostPortError::new(ROBOT_DEVICE_REJECTED, message))
            }
            Err(ToolCallError::Failed(message)) => {
                settle_known_failure(
                    &self.effects,
                    &reservation,
                    ROBOT_EFFECT_FAILED,
                    &message,
                )
                .await?;
                Err(Wave4HostPortError::new(ROBOT_EFFECT_FAILED, message))
            }
            Err(error @ (ToolCallError::Offline | ToolCallError::Timeout)) => {
                let reason = error.to_string();
                self.effects
                    .mark_outcome_unknown(
                        &reservation,
                        reason.clone(),
                        now_ms(),
                    )
                    .await
                    .map_err(effect_ledger_error)?;
                Err(Wave4HostPortError::new(
                    ROBOT_EFFECT_OUTCOME_UNKNOWN,
                    reason,
                ))
            }
        }
    }
}

#[async_trait::async_trait]
impl NomiHostDynamicToolInvoker for BoundRobotSessionToolInvoker {
    async fn invoke(
        &self,
        request: NomiHostDynamicToolInvocation,
    ) -> Result<StrictJsonValue, NomiHostDynamicToolError> {
        let key = (
            request.capability_id.clone(),
            request.provider_name.clone(),
        );
        let frozen = self.tools.get(&key).ok_or_else(|| {
            NomiHostDynamicToolError::new(
                "ROBOT_SESSION_TOOL_NOT_BOUND",
                format!(
                    "Robot Session tool {}/{} was not frozen into this AgentSession",
                    request.capability_id.as_ref(),
                    request.provider_name
                ),
                false,
            )
        })?;
        if !request.arguments.0.is_object() {
            return Err(NomiHostDynamicToolError::new(
                "INVALID_PAYLOAD",
                "Robot Session tool arguments must be an object",
                false,
            ));
        }
        self.owner
            .ensure_lifecycle_active(
                &self.principal,
                &self.agent_session_id,
                ROBOT_LINK,
                &self.binding,
                &self.robot_id,
            )
            .await
            .map_err(dynamic_tool_error)?;
        if frozen.device_name.starts_with("self.audio")
            || frozen.device_name.starts_with("audio")
        {
            self.owner
                .ensure_lifecycle_active(
                    &self.principal,
                    &self.agent_session_id,
                    ROBOT_AUDIO,
                    &self.binding,
                    &self.robot_id,
                )
                .await
                .map_err(dynamic_tool_error)?;
        }
        self.owner
            .execute_bound_device_tool(
                &self.principal,
                &self.agent_session_id,
                &request.operation_id,
                &request.idempotency_key,
                &request.correlation_id,
                &self.binding,
                frozen.capability,
                &request.provider_name,
                Some(&frozen.device_name),
                request.arguments.0,
            )
            .await
            .map_err(dynamic_tool_error)
    }
}

fn dynamic_tool_error(error: Wave4HostPortError) -> NomiHostDynamicToolError {
    let canonical_code = match error.code.as_str() {
        nomifun_agent_domain_wave4::WAVE4_INVALID_REQUEST => "INVALID_PAYLOAD",
        nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH => {
            "RESOURCE_OWNER_MISMATCH"
        }
        nomifun_agent_domain_wave4::WAVE4_RESOURCE_NOT_BOUND
        | nomifun_agent_domain_wave4::WAVE4_RESOURCE_BINDING_INVALID => {
            "PRESET_RESOURCE_NOT_BOUND"
        }
        other => other,
    };
    // Only an offline check performed before dispatch is safe to repeat. Every
    // ambiguous or post-dispatch physical outcome stays explicitly unsafe.
    let retry_safe = canonical_code == ROBOT_OFFLINE;
    NomiHostDynamicToolError::new(canonical_code, error.message, retry_safe)
}

#[async_trait::async_trait]
impl nomifun_ai_agent::ContextContributor for RobotLifecycleLease {
    async fn pre_turn_context(&self) -> Option<String> {
        if self.activation.cancelled.load(Ordering::Acquire) {
            return None;
        }
        let robot = self.registry.get(&self.activation.robot_id).await?;
        if robot.companion_id.as_deref() != Some(self.activation.companion_id.as_str()) {
            self.activation.cancelled.store(true, Ordering::Release);
            return None;
        }
        let online = self.status.snapshot().await.into_iter().any(|status| {
            status.robot_id == self.activation.robot_id && status.phase != "offline"
        });
        if !online {
            return None;
        }
        // ResourceProvider/BackgroundService leases own authority and cleanup,
        // not prompt data. Their health is enforced by the dynamic tool owner.
        None
    }

    fn label(&self) -> &str {
        "nomifun_robot_session_lifecycle"
    }
}

impl Drop for RobotLifecycleLease {
    fn drop(&mut self) {
        self.activation.cancelled.store(true, Ordering::Release);
        if let Some(current) = self.activations.get(&self.key) {
            let same_activation = Arc::ptr_eq(
                &current.cancelled,
                &self.activation.cancelled,
            );
            drop(current);
            if same_activation {
                self.activations.remove(&self.key);
            }
        }
    }
}

fn lifecycle_key(
    principal_id: &str,
    agent_session_id: &AgentSessionId,
    capability_id: &CapabilityId,
) -> String {
    format!(
        "{principal_id}\n{}\n{}",
        agent_session_id.as_ref(),
        capability_id.as_ref()
    )
}

fn validate_provider_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name.starts_with("robot_")
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(format!(
            "Robot tool {name:?} cannot be represented as a provider-safe exact tool name"
        ));
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

fn strict_device_input_schema(mut schema: serde_json::Value) -> Result<StrictJsonValue, String> {
    if serde_json::to_vec(&schema)
        .map_err(|error| format!("Robot tool schema cannot be encoded: {error}"))?
        .len()
        > MAX_DEVICE_TOOL_SCHEMA_BYTES
    {
        return Err(format!(
            "Robot tool schema exceeds the {MAX_DEVICE_TOOL_SCHEMA_BYTES}-byte Session limit"
        ));
    }
    harden_object_schemas(&mut schema, "$input")?;
    let root = schema
        .as_object()
        .ok_or_else(|| "Robot tool input schema must be a JSON object".to_owned())?;
    if root.get("type").and_then(serde_json::Value::as_str) != Some("object") {
        return Err("Robot tool input schema root type must be object".to_owned());
    }
    match root.get("properties") {
        Some(serde_json::Value::Object(_)) | None => {}
        _ => return Err("Robot tool input schema properties must be an object".to_owned()),
    }
    Ok(StrictJsonValue(schema))
}

/// Preserve the device-declared argument fields and types while ensuring no
/// object layer accepts undeclared fields. Device schema prose is not an
/// authority expansion for the host's physical-effect boundary.
fn harden_object_schemas(value: &mut serde_json::Value, path: &str) -> Result<(), String> {
    match value {
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                harden_object_schemas(item, &format!("{path}[{index}]"))?;
            }
        }
        serde_json::Value::Object(object) => {
            if object.contains_key("$ref") {
                return Err(format!(
                    "Robot tool schema {path} contains an external schema reference"
                ));
            }
            let object_schema = object.get("type").and_then(serde_json::Value::as_str)
                == Some("object")
                || object.contains_key("properties");
            if object_schema {
                if let Some(properties) = object.get("properties") {
                    if !properties.is_object() {
                        return Err(format!(
                            "Robot tool schema {path}.properties must be an object"
                        ));
                    }
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

impl Wave4HostPort for NomiCoreRobotWave4Owner {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>> + Send + 'a>> {
        Box::pin(async move {
            request.validate()?;
            match &request.operation {
                Wave4CapabilityOperation::RobotDisplay { input } => {
                    self.execute_tool(&request, input, RobotToolCapability::Display)
                        .await
                }
                Wave4CapabilityOperation::RobotMotion { input } => {
                    self.execute_tool(&request, input, RobotToolCapability::Motion)
                        .await
                }
                Wave4CapabilityOperation::RobotDeviceTools { input } => {
                    self.execute_tool(&request, input, RobotToolCapability::DeviceTools)
                        .await
                }
                _ => Err(Wave4HostPortError::action_operation_mismatch(
                    "Robot owner received a non-Robot action",
                )),
            }
        })
    }
}

impl Wave4ContextHostPort for NomiCoreRobotWave4Owner {
    fn contribute<'a>(
        &'a self,
        request: Wave4ContextHostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<StrictJsonValue>, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            request.validate()?;
            if request.capability_id.as_ref() != ROBOT_VISION {
                return Err(Wave4HostPortError::action_operation_mismatch(
                    "Robot Context owner received a non-vision contribution",
                ));
            }
            self.ensure_installation_owner(&request.principal.principal_id)?;
            let binding = exact_robot_binding(
                &request.resource_bindings,
                &request.principal.principal_id,
            )?;
            let robot = self.load_bound_robot(binding).await?;
            let companion_id = robot
                .companion_id
                .as_deref()
                .expect("load_bound_robot requires pairing");
            Ok(self
                .observations
                .latest_recent(
                    &robot.robot_id,
                    companion_id,
                    now_ms(),
                )
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
        })
    }
}

fn exact_robot_binding<'a>(
    bindings: &'a [TypedResourceBinding],
    principal_id: &str,
) -> Result<&'a TypedResourceBinding, Wave4HostPortError> {
    let mut matching = bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == ROBOT_RESOURCE_KIND);
    let binding = matching.next().ok_or_else(|| {
        Wave4HostPortError::resource_not_bound("target Robot resource is not bound")
    })?;
    if matching.next().is_some() {
        return Err(Wave4HostPortError::resource_binding_invalid(
            "more than one Robot resource is bound",
        ));
    }
    if binding.owner_id != principal_id {
        return Err(Wave4HostPortError::resource_owner_mismatch(
            "Robot resource binding belongs to another principal",
        ));
    }
    Ok(binding)
}

fn effect_ledger_error(error: anyhow::Error) -> Wave4HostPortError {
    let message = error.to_string();
    let code = if message.contains("reused with a different request") {
        "ROBOT_EFFECT_IDEMPOTENCY_CONFLICT"
    } else {
        ROBOT_EFFECT_RECEIPT_FAILED
    };
    Wave4HostPortError::new(code, message)
}

async fn settle_known_failure(
    ledger: &RobotEffectLedger,
    reservation: &nomifun_robot::effect_ledger::RobotEffectReservation,
    code: &str,
    message: &str,
) -> Result<(), Wave4HostPortError> {
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
        ActionId, AgentSessionId, CanonicalSchemaRef, CorrelationId,
        IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, ScopeKey,
    };
    use nomifun_robot::link::Frame;
    use nomifun_robot::mcp_bridge::{RobotMcpClient, RobotToolDescriptor};
    use nomifun_robot::registry::RobotReport;
    use nomifun_robot::status::{RobotPhase, RobotStatusRegistry};
    use nomifun_robot::vision::RobotVisionObservation;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    async fn fixture() -> (
        Arc<NomiCoreRobotWave4Owner>,
        tempfile::TempDir,
        Arc<AtomicUsize>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (reported, _) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "robot-1".to_owned(),
                    client_id: "client-1".to_owned(),
                    board: "fake-board".to_owned(),
                    firmware_version: "1.0.0".to_owned(),
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

        let calls = Arc::new(AtomicUsize::new(0));
        let responder_calls = Arc::clone(&calls);
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let client = Arc::new(RobotMcpClient::new(tx, "device-session".to_owned()));
        let responder = Arc::clone(&client);
        tokio::spawn(async move {
            while let Some(Frame::Text(raw)) = rx.recv().await {
                let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
                let payload = &envelope["payload"];
                responder_calls.fetch_add(1, Ordering::SeqCst);
                responder
                    .handle_incoming(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": payload["id"],
                        "result": {
                            "content": [{ "type": "text", "text": "device-ok" }],
                            "isError": false
                        }
                    }))
                    .await;
            }
        });
        let tools = Arc::new(RobotToolRegistry::default());
        tools
            .attach(
                "robot-1",
                client,
                vec![
                    descriptor("self.emoji.set_expression"),
                    descriptor("self.head.look"),
                    descriptor("self.get_device_status"),
                ],
            )
            .await;
        let status = Arc::new(RobotStatusRegistry::new(
            nomifun_robot::events::RobotEventEmitter::new(Arc::new(NullSink)),
            "owner".to_owned(),
        ));
        status
            .publish("robot-1", Some("companion-1"), RobotPhase::Idle, 1)
            .await;
        let effects = Arc::new(RobotEffectLedger::load(dir.path()).await.unwrap());
        let observations = Arc::new(RobotVisionObservationRegistry::default());
        observations
            .record(RobotVisionObservation {
                robot_id: "robot-1".to_owned(),
                companion_id: "companion-1".to_owned(),
                question: "what?".to_owned(),
                answer: "a cup".to_owned(),
                observed_at_ms: now_ms(),
            })
            .await;
        (
            Arc::new(NomiCoreRobotWave4Owner::from_parts(
                Arc::from("owner"),
                registry,
                status,
                tools,
                effects,
                observations,
            )),
            dir,
            calls,
        )
    }

    struct NullSink;

    impl nomifun_realtime::UserEventSink for NullSink {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    fn descriptor(device_name: &str) -> RobotToolDescriptor {
        RobotToolDescriptor {
            device_name: device_name.to_owned(),
            exposed_name: nomifun_robot::mcp_bridge::exposed_tool_name(device_name),
            description: device_name.to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn binding(operation: &str) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("robot:robot-1"),
            resource_kind: ResourceKind::from(ROBOT_RESOURCE_KIND),
            resource_id: ResourceId::from("robot-1"),
            owner_id: "owner".to_owned(),
            operations: BTreeSet::from([operation.to_owned()]),
            connection_config_ref: None,
            typed_parameters: [("companion_id".to_owned(), "companion-1".to_owned())]
                .into_iter()
                .collect(),
        }
    }

    fn all_tool_binding() -> TypedResourceBinding {
        let mut binding = binding("link");
        binding.operations.extend([
            "audio".to_owned(),
            "display".to_owned(),
            "motion".to_owned(),
        ]);
        binding
    }

    fn action_request(
        capability_id: &str,
        operation: &str,
        tool_name: &str,
        idempotency_key: &str,
    ) -> Wave4HostRequest {
        let input = StrictJsonValue(serde_json::json!({
            "tool_name": tool_name,
            "arguments": {},
        }));
        let operation_value = match capability_id {
            ROBOT_DISPLAY => Wave4CapabilityOperation::RobotDisplay { input },
            ROBOT_MOTION => Wave4CapabilityOperation::RobotMotion { input },
            ROBOT_DEVICE_TOOLS => Wave4CapabilityOperation::RobotDeviceTools { input },
            _ => unreachable!(),
        };
        Wave4HostRequest {
            context: nomifun_agent_domain_wave4::Wave4HostContext {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "owner".to_owned(),
                },
                agent_session_id: AgentSessionId::from("session"),
                operation_id: OperationId::from("operation"),
                idempotency_key: IdempotencyKey::from(idempotency_key),
                correlation_id: CorrelationId::from("correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 1,
                capability_id: CapabilityId::from(capability_id),
                action_id: ActionId::from(format!("{capability_id}.invoke")),
                state_scope_key: ScopeKey::from("session:session"),
                resource_bindings: vec![binding(operation)],
            },
            operation: operation_value,
        }
    }

    fn context_schema() -> CanonicalSchemaRef {
        nomifun_agent_domain_wave4::robot_registration()
            .unwrap()
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .into_iter()
            .find(|capability| capability.id.as_ref() == ROBOT_VISION)
            .unwrap()
            .contributions
            .context_schema_refs
            .into_iter()
            .next()
            .unwrap()
    }

    #[tokio::test]
    async fn physical_action_dispatches_once_and_replays_the_durable_receipt() {
        let (owner, _dir, calls) = fixture().await;
        let request = action_request(
            ROBOT_MOTION,
            "motion",
            "robot_head_look",
            "same-key",
        );
        let first = owner.invoke(request.clone()).await.unwrap();
        assert_eq!(first.0["result"], "device-ok");
        assert_eq!(first.0["replayed"], false);
        let replay = owner.invoke(request).await.unwrap();
        assert_eq!(replay.0["replayed"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn device_tool_cannot_escape_its_selected_capability() {
        let (owner, _dir, calls) = fixture().await;
        let error = owner
            .invoke(action_request(
                ROBOT_DISPLAY,
                "display",
                "robot_head_look",
                "display-key",
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, ROBOT_DEVICE_REJECTED);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn vision_context_comes_from_the_real_bounded_observation_registry() {
        let (owner, _dir, _calls) = fixture().await;
        let value = owner
            .contribute(Wave4ContextHostRequest {
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: "owner".to_owned(),
                },
                agent_session_id: AgentSessionId::from("session"),
                operation_id: OperationId::from("context-operation"),
                correlation_id: CorrelationId::from("context-correlation"),
                resolved_snapshot_ref: ResolvedSnapshotRef {
                    snapshot_id: "snapshot".into(),
                    snapshot_digest: "digest".into(),
                },
                registry_generation: 1,
                registry_digest: nomifun_agent_contracts::DigestHex::from("registry"),
                capability_id: CapabilityId::from(ROBOT_VISION),
                state_scope_key: ScopeKey::from("session:session"),
                resource_bindings: vec![binding("vision")],
                schema_ref: context_schema(),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value.0["kind"], "robot_vision");
        assert_eq!(value.0["answer"], "a cup");
    }

    #[tokio::test]
    async fn link_and_audio_lifecycle_use_the_live_robot_session_not_the_mcp_toolset() {
        let (owner, _dir, _calls) = fixture().await;
        for capability_id in [ROBOT_LINK, ROBOT_AUDIO] {
            let value = owner
                .activate_lifecycle_for(
                    &CapabilityId::from(capability_id),
                    "owner",
                    &AgentSessionId::from("session"),
                    &[binding(if capability_id == ROBOT_LINK {
                        "link"
                    } else {
                        "audio"
                    })],
                )
                .await
                .unwrap();
            assert_eq!(value.0["kind"], capability_id);
            assert_eq!(value.0["state"], "active");
        }
    }

    #[tokio::test]
    async fn exact_dynamic_tools_require_a_live_session_lease_and_drop_revokes_it() {
        let (owner, _dir, calls) = fixture().await;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let session_id = AgentSessionId::from("dynamic-session");
        let binding = all_tool_binding();
        let initial = BTreeSet::from([CapabilityId::from(ROBOT_DISPLAY)]);
        let deferred = BTreeSet::from([
            CapabilityId::from(ROBOT_MOTION),
            CapabilityId::from(ROBOT_DEVICE_TOOLS),
        ]);
        let resolved = owner
            .resolve_session_tools(
                &principal,
                &session_id,
                &binding,
                &initial,
                &deferred,
            )
            .await
            .unwrap();
        assert_eq!(resolved.0.len(), 3);
        let display = resolved
            .0
            .iter()
            .find(|descriptor| descriptor.provider_name == "robot_emoji_set_expression")
            .unwrap();
        assert_eq!(display.capability_id.as_ref(), ROBOT_DISPLAY);
        assert!(!display.deferred);
        assert_eq!(display.input_schema.0["type"], "object");
        assert_eq!(display.input_schema.0["additionalProperties"], false);
        assert!(resolved
            .0
            .iter()
            .find(|descriptor| descriptor.provider_name == "robot_head_look")
            .unwrap()
            .deferred);

        let invocation = |key: &str| NomiHostDynamicToolInvocation {
            capability_id: CapabilityId::from(ROBOT_DISPLAY),
            provider_name: "robot_emoji_set_expression".to_owned(),
            operation_id: OperationId::from(format!("operation-{key}")),
            idempotency_key: IdempotencyKey::from(key.to_owned()),
            correlation_id: CorrelationId::from(format!("correlation-{key}")),
            arguments: StrictJsonValue(serde_json::json!({})),
        };
        let before_activation = resolved
            .1
            .invoke(invocation("before"))
            .await
            .unwrap_err();
        assert!(before_activation.internal_message.contains("robot.link"));
        assert!(before_activation.retry_safe);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        owner
            .activate_lifecycle_for(
                &CapabilityId::from(ROBOT_LINK),
                "owner",
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap();
        let lease = owner
            .lifecycle_context_contributor_for(
                &CapabilityId::from(ROBOT_LINK),
                &principal,
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap()
            .unwrap();
        let output = resolved
            .1
            .invoke(invocation("active"))
            .await
            .unwrap();
        assert_eq!(output.0["tool_name"], "robot_emoji_set_expression");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        drop(lease);
        let after_drop = resolved
            .1
            .invoke(invocation("after-drop"))
            .await
            .unwrap_err();
        assert!(after_drop.internal_message.contains("robot.link"));
        assert!(after_drop.retry_safe);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dynamic_tool_rechecks_the_robot_companion_binding() {
        let (owner, _dir, calls) = fixture().await;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let session_id = AgentSessionId::from("binding-session");
        let binding = all_tool_binding();
        let resolved = owner
            .resolve_session_tools(
                &principal,
                &session_id,
                &binding,
                &BTreeSet::from([CapabilityId::from(ROBOT_DISPLAY)]),
                &BTreeSet::new(),
            )
            .await
            .unwrap();
        owner
            .activate_lifecycle_for(
                &CapabilityId::from(ROBOT_LINK),
                "owner",
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap();
        let _lease = owner
            .lifecycle_context_contributor_for(
                &CapabilityId::from(ROBOT_LINK),
                &principal,
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap()
            .unwrap();
        owner
            .registry
            .patch(
                "robot-1",
                None,
                Some(Some("different-companion".to_owned())),
            )
            .await
            .unwrap();

        let error = resolved
            .1
            .invoke(NomiHostDynamicToolInvocation {
                capability_id: CapabilityId::from(ROBOT_DISPLAY),
                provider_name: "robot_emoji_set_expression".to_owned(),
                operation_id: OperationId::from("binding-operation"),
                idempotency_key: IdempotencyKey::from("binding-key"),
                correlation_id: CorrelationId::from("binding-correlation"),
                arguments: StrictJsonValue(serde_json::json!({})),
            })
            .await
            .unwrap_err();
        assert!(error.internal_message.contains("no longer paired"));
        assert!(!error.retry_safe);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dynamic_tool_preserves_outcome_unknown_as_not_retry_safe() {
        let (owner, _dir, _calls) = fixture().await;
        let (tx, rx) = mpsc::channel::<Frame>(1);
        drop(rx);
        owner
            .tools
            .attach(
                "robot-1",
                Arc::new(RobotMcpClient::new(tx, "closed-device-session".to_owned())),
                vec![descriptor("self.emoji.set_expression")],
            )
            .await;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "owner".to_owned(),
        };
        let session_id = AgentSessionId::from("unknown-session");
        let binding = all_tool_binding();
        let resolved = owner
            .resolve_session_tools(
                &principal,
                &session_id,
                &binding,
                &BTreeSet::from([CapabilityId::from(ROBOT_DISPLAY)]),
                &BTreeSet::new(),
            )
            .await
            .unwrap();
        owner
            .activate_lifecycle_for(
                &CapabilityId::from(ROBOT_LINK),
                "owner",
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap();
        let _lease = owner
            .lifecycle_context_contributor_for(
                &CapabilityId::from(ROBOT_LINK),
                &principal,
                &session_id,
                std::slice::from_ref(&binding),
            )
            .await
            .unwrap()
            .unwrap();

        let error = resolved
            .1
            .invoke(NomiHostDynamicToolInvocation {
                capability_id: CapabilityId::from(ROBOT_DISPLAY),
                provider_name: "robot_emoji_set_expression".to_owned(),
                operation_id: OperationId::from("unknown-operation"),
                idempotency_key: IdempotencyKey::from("unknown-key"),
                correlation_id: CorrelationId::from("unknown-correlation"),
                arguments: StrictJsonValue(serde_json::json!({})),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code.as_ref(), ROBOT_EFFECT_OUTCOME_UNKNOWN);
        assert!(!error.retry_safe);
        assert!(error.internal_message.contains("offline"));
    }
}
