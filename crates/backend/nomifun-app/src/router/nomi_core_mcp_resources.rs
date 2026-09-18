//! Exact Nomi `mcp_server` binding admission into the shared platform MCP owner.
//! Nomi's execution context differs from the Runtime's Broker context; neither is
//! fabricated here. The retained Nomi effect scope owns calls and turn closure.
use async_trait::async_trait;
use nomifun_agent_contracts::{
    AgentSessionId, CapabilityId, ContributionSourceKind, PluginSourceKind, PrincipalRef,
    digest_payload,
};
use nomifun_agent_kernel::{
    ActiveCapabilitySetSnapshot, CompiledSnapshot, KernelRegistry, SessionCapabilityState,
};
use nomifun_ai_agent::nomi_resources::{
    NomiMcpResourceInvoker, NomiMcpResources, NomiResourceImageAuthority,
};
use nomifun_common::AppError;
use nomifun_engine_core::{EngineResourceImageRead, EngineResourceRead, EngineToolResult};
use serde_json::Value;
use std::sync::Arc;

use super::{
    engine_kernel_session::{
        mcp_resource_operation, project_resource_page, resource_server_ids, select_resource_server,
        validate_resource_owner_result, validate_resource_page,
    },
    nomi_core_wave2::NomiCoreWave2Host,
};

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi MCP resource: {message}"))
}

pub(crate) fn adapter(
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    host: Arc<NomiCoreWave2Host>,
    principal: PrincipalRef,
    session: AgentSessionId,
    primary_image_input: bool,
    constraints: nomifun_api_types::ExecutionConstraints,
) -> Result<NomiMcpResources, AppError> {
    validate(&kernel, &compiled, &principal)?;
    frozen_state(&compiled, &active)?;
    let identity = digest_payload(&(
        "nomi-mcp-resource-port-v1",
        compiled.snapshot_ref(),
        compiled.resource_bindings(),
    ))
    .map_err(failure)?;
    let server_ids = resource_server_ids(&compiled);
    let image_authority = Arc::new(NomiResourceImageAuthority::default());
    NomiMcpResources::new(
        Arc::new(ResourceOwner {
            kernel,
            compiled,
            active,
            host,
            principal,
            session,
            primary_image_input,
            constraints,
            image_authority: image_authority.clone(),
            serial: tokio::sync::Mutex::new(()),
        }),
        identity.as_ref().to_owned(),
        false,
        server_ids,
    )?
    .with_image_authority(image_authority)
}

fn validate(
    kernel: &KernelRegistry,
    compiled: &CompiledSnapshot,
    principal: &PrincipalRef,
) -> Result<(), AppError> {
    let registry = kernel.snapshot().map_err(failure)?;
    if compiled.registry_generation != registry.generation
        || compiled.registry_digest != registry.registry_digest
    {
        return Err(failure("resource binding differs from its frozen registry"));
    }
    super::nomi_core_mcp_catalog::validate_resources(compiled, &registry, principal)
}

fn frozen_state(
    compiled: &CompiledSnapshot,
    state: &SessionCapabilityState,
) -> Result<ActiveCapabilitySetSnapshot, AppError> {
    let active = state.snapshot().map_err(failure)?;
    if active.resolved_snapshot_ref != *compiled.snapshot_ref() {
        return Err(failure("resource binding belongs to another frozen Snapshot"));
    }
    Ok(active)
}

struct ResourceOwner {
    kernel: Arc<KernelRegistry>,
    compiled: Arc<CompiledSnapshot>,
    active: Arc<SessionCapabilityState>,
    host: Arc<NomiCoreWave2Host>,
    principal: PrincipalRef,
    session: AgentSessionId,
    primary_image_input: bool,
    constraints: nomifun_api_types::ExecutionConstraints,
    image_authority: Arc<NomiResourceImageAuthority>,
    serial: tokio::sync::Mutex<()>,
}

#[async_trait]
impl NomiMcpResourceInvoker for ResourceOwner {
    async fn read(
        &self,
        operation_id: String,
        activation_proven: bool,
        request: EngineResourceRead,
    ) -> Result<Value, AppError> {
        let _serial = self.serial.lock().await;
        let (value, request, _) = self
            .observe(operation_id, activation_proven, request)
            .await?;
        project_resource_page(&value, &request)
    }

    async fn read_image(
        &self,
        operation_id: String,
        activation_proven: bool,
        request: EngineResourceImageRead,
    ) -> Result<EngineToolResult, AppError> {
        super::engine_mcp_media::validate_image(&request)?;
        let _serial = self.serial.lock().await;
        self.admit_image(None)?;
        let read = EngineResourceRead {
            server_id: request.server_id.clone(),
            query: request.query.clone(),
            offset: 0,
            limit: 8192,
            expected_sha256: None,
        };
        let (value, read, generation) = self
            .observe(operation_id.clone(), activation_proven, read)
            .await?;
        validate_resource_owner_result(&value, &read)?;
        self.admit_image(Some(generation))?;
        // Nomi's EngineEffectScope retains this complete future through remote
        // cleanup, receipts AND decoding. Never fabricate a Runtime/Broker turn.
        let result = super::engine_mcp_media::image(value, request, operation_id).await?;
        self.admit_image(Some(generation))?;
        Ok(result)
    }
}

impl ResourceOwner {
    fn admit_image(&self, generation: Option<u64>) -> Result<(), AppError> {
        self.image_authority.ensure_active()?;
        let active = frozen_state(&self.compiled, &self.active)?;
        let selected = self
            .compiled
            .content()
            .enabled_capabilities
            .iter()
            .find(|entry| entry.capability.id.as_ref() == "llm.vision");
        if !self.primary_image_input
            || !self.constraints.allows_capability("llm.vision")
            || !active.active.contains(&CapabilityId::from("llm.vision"))
            || generation.is_some_and(|generation| active.generation != generation)
            || !selected.is_some_and(|entry| {
                entry.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
                    && entry.resolved_source.source_kind == PluginSourceKind::Bundled
            })
        {
            return Err(failure(
                "image input differs from frozen vision selection, primary route or capability generation",
            ));
        }
        // Host-bound image support and the frozen vision selection are both
        // required. ToolSearch presentation evidence never grants vision.
        Ok(())
    }

    /// Caller keeps the serial guard until all projection/decoding has ended.
    async fn observe(
        &self,
        operation_id: String,
        activation_proven: bool,
        request: EngineResourceRead,
    ) -> Result<(Value, EngineResourceRead, u64), AppError> {
        validate_resource_page(&request)?;
        if self.constraints.restricted() {
            return Err(failure(
                "resource read exceeds the frozen execution ceiling",
            ));
        }
        if !activation_proven
            || !operation_id
                .strip_prefix("nomi-resource:tool-call-v1-")
                .is_some_and(|suffix| {
                    suffix.len() == 64
                        && suffix
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        {
            return Err(failure("resource operation has no valid engine presentation evidence or operation identity"));
        }
        validate(&self.kernel, &self.compiled, &self.principal)?;
        // Presentation evidence never grants a resource. The exact typed
        // binding remains the sole server/read authority.
        let active = frozen_state(&self.compiled, &self.active)?;
        // Reject an ambiguous/unknown target before entering the shared owner.
        let binding = select_resource_server(
            &self.compiled,
            &self.principal,
            request.server_id.as_deref(),
        )?;
        let mut request = request;
        request.server_id = Some(binding.resource_id.as_ref().to_owned());
        self.host
            .ensure_mcp_settled(&self.principal.principal_id, self.session.as_ref())
            .await?;
        let generation = active.generation;
        let operation = mcp_resource_operation(&request.query);
        // Shared owner reserves the current live Conversation epoch BEFORE
        // initialize/OAuth/stdio start and settles only after protocol cleanup.
        let result = self
            .host
            .read_mcp_resource(
                self.principal.clone(),
                self.session.clone(),
                operation_id.into(),
                binding.clone(),
                operation,
            )
            .await?;
        Ok((result.0, request, generation))
    }
}
