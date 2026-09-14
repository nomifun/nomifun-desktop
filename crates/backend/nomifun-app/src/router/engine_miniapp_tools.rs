//! Frozen Plugin Product function tools shared by source-integrated engines.
//! Service execution, authority and receipts remain owned by the application.
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, ContributionSourceKind, OperationId, PluginBridgeCallId, PrincipalRef,
    ResolvedCapability, StrictJsonValue, ToolPresentationKind,
};
use nomifun_agent_kernel::{CompiledSnapshot, SessionCapabilityState};
use nomifun_common::AppError;
use nomifun_engine_core::{
    EngineToolError, EngineToolExposure, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult,
};
use nomifun_plugin_platform::runtime::{
    PluginRuntimeAgentCapabilityInvocation, PluginRuntimeAgentCapabilityPort,
    PluginRuntimeApplicationError, PluginRuntimeApplicationService,
};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::hosted_effect_receipts::{Domain, HostedEffectReceipts};

#[derive(Clone)]
pub(crate) struct MiniAppOwner {
    pub application: Arc<PluginRuntimeApplicationService>,
    pub receipts: HostedEffectReceipts,
}

pub(crate) enum MiniAppCallError {
    Rejected(String),
    Unknown(String),
}

impl MiniAppOwner {
    /// Caller supplies only its host-frozen capability. The real Service owner
    /// rechecks release/catalog/action authority immediately before dispatch.
    pub(crate) async fn invoke(
        &self,
        user: &str,
        session: &str,
        capability: &ResolvedCapability,
        action: &nomifun_agent_contracts::ActionId,
        operation: OperationId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, MiniAppCallError> {
        validate_capability(capability)
            .map_err(|error| MiniAppCallError::Rejected(error.to_string()))?;
        // Unified capabilities carry optional release fields. Never default to
        // another product/release or panic, even for a caller-supplied snapshot.
        let (Some(product), Some(release), Some(epoch), Some(catalog)) = (
            capability.plugin_product_id.as_ref(),
            capability.active_release.as_ref(),
            capability.active_release_epoch,
            capability.catalog_digest.as_ref(),
        ) else {
            return Err(MiniAppCallError::Rejected(
                "Plugin Product capability is missing frozen release authority".into(),
            ));
        };
        let receipt = self
            .receipts
            .begin(
                user,
                session,
                operation.as_ref(),
                capability.capability.id.as_ref(),
                action.as_ref(),
                &input.0,
                Domain::MiniApp,
            )
            .await
            .map_err(|error| MiniAppCallError::Unknown(error.to_string()))?;
        let result = self
            .application
            .invoke_agent_capability(PluginRuntimeAgentCapabilityInvocation {
                owner_user_id: user.to_owned(),
                plugin_product_id: product.clone(),
                capability: capability.capability.clone(),
                action_id: action.clone(),
                action_allowlist: capability.action_allowlist.clone(),
                active_release: release.clone(),
                active_release_epoch: epoch,
                catalog_digest: catalog.clone(),
                // Retain the historical call identity alongside the receipt codec.
                call_id: PluginBridgeCallId::from(format!(
                    "platform-miniapp:{}",
                    operation.as_ref()
                )),
                operation_id: operation,
                payload: input,
            })
            .await;
        let written = match &result {
            Ok(output) => self.receipts.returned(receipt, &output.0).await,
            // These exact variants currently precede Service invocation in the
            // application owner. Other failures MUST keep the receipt pending.
            Err(
                PluginRuntimeApplicationError::Invalid(_)
                | PluginRuntimeApplicationError::NotFound,
            ) => {
                self.receipts
                    .rejected(receipt, "MINIAPP_REJECTED_BEFORE_DISPATCH")
                    .await
            }
            Err(_) => Ok(()),
        };
        written.map_err(|error| MiniAppCallError::Unknown(error.to_string()))?;
        result.map_err(|error| match error {
            PluginRuntimeApplicationError::Invalid(_)
            | PluginRuntimeApplicationError::NotFound => {
                MiniAppCallError::Rejected(error.to_string())
            }
            _ => MiniAppCallError::Unknown(error.to_string()),
        })
    }

    /// Only schemas from the exact selected Active Release are read. No
    /// execution, arbitrary catalog discovery, or latest-release fallback.
    pub(crate) async fn exposures(
        &self,
        user: &str,
        snapshot: &CompiledSnapshot,
    ) -> Result<Vec<EngineToolExposure>, AppError> {
        let mut exposures = Vec::new();
        let content = snapshot.content();
        let capabilities = content.enabled_capabilities.iter().filter(|capability| {
            capability.contribution_lock.source_kind
                == ContributionSourceKind::PluginProductActiveRelease
        });
        if capabilities.clone().count() > 128 {
            return Err(failure("too many MiniApp capabilities"));
        }
        for capability in capabilities {
            validate_capability(capability)?;
            let policy = snapshot
                .policy(&capability.capability.id)
                .ok_or_else(|| failure("missing MiniApp authority policy"))?;
            let before = exposures.len();
            for action in &capability.actions {
                if action.presentation != ToolPresentationKind::FunctionTool
                    || !policy.allowed_actions.contains(&action.action_id)
                    || (!capability.action_allowlist.is_empty()
                        && !capability.action_allowlist.contains(&action.action_id))
                {
                    continue;
                }
                if exposures.len() >= 128 {
                    return Err(failure("MiniApp tool surface exceeds 128 actions"));
                }
                let schema = self
                    .application
                    .resolve_agent_capability_schema(user, capability, &action.input_schema)
                    .await
                    .map_err(|_| failure("frozen MiniApp schema unavailable"))?;
                if nomifun_agent_contracts::canonical_json_bytes(&schema)
                    .map_err(failure)?
                    .len()
                    > 64 * 1024
                {
                    return Err(failure("MiniApp schema exceeds its context budget"));
                }
                let identity = nomifun_agent_contracts::canonical_json_bytes(&(capability, action))
                    .map_err(failure)?;
                let name = format!("miniapp_{:x}", Sha256::digest(&identity));
                exposures.push(EngineToolExposure {
                    definition: nomifun_chat_model_broker::ChatToolDefinition {
                        name: name[..63].to_owned(),
                        description: format!("{}: {}. MiniApp Service action {}; uses the frozen platform grant. A reply does not imply remote effects are reversible.",
                            capability.display_name.as_deref().unwrap_or(capability.capability.id.as_ref()),
                            capability.description.as_deref().unwrap_or(""), action.action_id.as_ref()),
                        input_schema: schema, deferred: false,
                    },
                    capability_id: capability.capability.id.clone(), action_id: action.action_id.clone(),
                });
            }
            if exposures.len() == before {
                return Err(failure("MiniApp has no admitted function action"));
            }
        }
        Ok(exposures)
    }
}

pub(crate) fn validate_capability(capability: &ResolvedCapability) -> Result<(), AppError> {
    if capability.contribution_lock.source_kind
        != ContributionSourceKind::PluginProductActiveRelease
    {
        return Err(failure("capability is not a Plugin Product Active Release"));
    }
    capability
        .validate()
        .map_err(|_| failure("invalid frozen MiniApp capability"))?;
    let mut actions = std::collections::BTreeSet::new();
    if capability.display_name.as_ref().is_some_and(|name| name.len() > 256)
        || capability.description.as_ref().is_some_and(|description| description.len() > 4096)
        || capability.capability.id.as_ref().len() > 1024
        || !capability.required_resource_kinds.is_empty()
        || capability.actions.len() > 128
        || capability.actions.iter().any(|action| {
            action.action_id.as_ref().len() > 1024 || !actions.insert(&action.action_id)
        })
        || nomifun_agent_contracts::canonical_json_bytes(capability)
            .map_err(failure)?
            .len()
            > 256 * 1024
        || !capability.actions.iter().any(|action| {
            action.presentation == ToolPresentationKind::FunctionTool
                && (capability.action_allowlist.is_empty()
                    || capability.action_allowlist.contains(&action.action_id))
        })
    {
        return Err(failure(
            "MiniApp requires bounded, unique function actions without unsupported resource grants",
        ));
    }
    Ok(())
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine MiniApp tools: {message}"))
}
fn rejected() -> EngineToolError {
    EngineToolError::ToolInvocation(
        "MiniApp invocation differs from frozen Session authority".into(),
    )
}

/// Installed inside the one retained EngineToolHost. Never expose this raw
/// invoker as a caller-owned future: the owner receipt must survive cancellation.
pub(crate) struct SessionTools {
    pub owner: MiniAppOwner,
    pub inner: Arc<dyn EngineToolInvoker>,
    pub snapshot: Arc<CompiledSnapshot>,
    pub active: Arc<SessionCapabilityState>,
    pub principal: PrincipalRef,
    pub session: AgentSessionId,
    pub plan: EngineToolPlan,
}

#[async_trait]
impl EngineToolInvoker for SessionTools {
    async fn invoke(
        &self,
        invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if cancellation.is_cancelled() {
            return Err(EngineToolError::Cancelled);
        }
        // Covers all lanes, not only MiniApp: an earlier unknown hosted effect
        // cannot be followed by a different tool while cleanup is unproven.
        self.owner
            .receipts
            .ensure_settled(&self.principal.principal_id, self.session.as_ref())
            .await
            .map_err(|_| EngineToolError::ToolInvocation("HOSTED_EFFECT_UNPROVEN".into()))?;
        let Some(capability) = self
            .snapshot
            .resolved_capability(&invocation.binding.capability_id)
            .filter(|capability| {
                capability.contribution_lock.source_kind
                    == ContributionSourceKind::PluginProductActiveRelease
            })
        else {
            return self.inner.invoke(invocation, cancellation).await;
        };
        let active = self.active.snapshot().map_err(|_| rejected())?;
        if self.principal.principal_kind != "user"
            || invocation.principal != self.principal
            || invocation.agent_session_id != self.session
            || invocation.resolved_snapshot_ref != *self.snapshot.snapshot_ref()
            || active.resolved_snapshot_ref != invocation.resolved_snapshot_ref
            || active.generation != invocation.active_set_generation
            || !active.active.contains(&capability.capability.id)
            || invocation.call.name != invocation.binding.model_name
            || self.plan.binding(&invocation.call.name) != Some(&invocation.binding)
        {
            return Err(rejected());
        }
        invocation.binding.validate()?;
        invocation.call.validate().map_err(|_| rejected())?;
        nomifun_engine_core::parse_completed_arguments(&invocation.call)?;
        let result = self
            .owner
            .invoke(
                &self.principal.principal_id,
                self.session.as_ref(),
                capability,
                &invocation.binding.action_id,
                invocation.operation_id,
                invocation.call.arguments,
            )
            .await;
        match result {
            Ok(output) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                serde_json::to_string(&output.0).map_err(|_| {
                    EngineToolError::ToolInvocation("MiniApp result could not be encoded".into())
                })?,
                false,
            )),
            Err(MiniAppCallError::Rejected(_)) => Ok(EngineToolResult::text(
                invocation.call.call_id,
                "MINIAPP_REJECTED_BEFORE_DISPATCH: frozen authority could not be admitted; no Service call was dispatched.",
                true,
            )),
            Err(MiniAppCallError::Unknown(_)) => Err(EngineToolError::ToolInvocation(
                "HOSTED_EFFECT_UNPROVEN: stop and inspect owner state; do not retry".into(),
            )),
        }
    }
}
