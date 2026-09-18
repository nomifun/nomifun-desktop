//! Nomi-core composition for the workspace-scoped Wave 2 capabilities.
//!
//! The current Nomi Session owns the physical workspace. Neither the model nor
//! a saved Agent binding may choose a native root. The Session provider
//! canonicalizes its server-persisted workspace, replaces any stale workspace
//! resource with [`session_workspace_binding`], and only then compiles the
//! Kernel invocation set. This module's action host accepts exactly that
//! Session-derived binding identity and keeps the existing Wave 2 owners
//! isolated per canonical root.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nomifun_agent_contracts::{
    AgentSessionId, CanonicalSchemaRef, CapabilityId, PrincipalRef,
    ResolvedCapability, ResourceBindingId, ResourceKind, StrictJsonValue,
    TypedResourceBinding,
};
use nomifun_agent_domain_wave2::{
    Wave2HostContext, Wave2HostPort, Wave2HostPortError, Wave2HostRequest,
    WorkspaceFileChangeKind, WorkspaceFileChangedEvent, WorkspaceFilesChangedBatch,
};
use nomifun_ai_agent::ContextContributor;
use nomifun_api_types::TypedResourceBindingDto;
use nomifun_common::AppError;
use nomifun_file::{
    AgentSessionWorkspaceBinding, WORKSPACE_DELETE_OPERATION,
    WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER,
    WORKSPACE_WRITE_OPERATION,
};
use notify::{event::ModifyKind, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;

use super::agent_wave2_host::{
    Wave2ApplicationHost, Wave2EffectAdmission, Wave2EffectCompletion,
    begin_wave2_exclusive_effect, finish_wave2_effect,
};

const WORKSPACE_FILES: &str = nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID;
const WORKSPACE_VCS: &str = nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID;
const WORKSPACE_PROCESS: &str = nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID;
const WORKSPACE_ARTIFACTS: &str = nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID;

/// Platform-owned workspace operations usable by the Coding host. Nomi's native
/// tool admission below remains unchanged, so tools are never registered twice.
pub(crate) fn coding_capability_ids() -> BTreeSet<CapabilityId> {
    [WORKSPACE_FILES, WORKSPACE_VCS, WORKSPACE_PROCESS, WORKSPACE_ARTIFACTS]
        .into_iter().map(CapabilityId::from).collect()
}

const MAX_WATCH_EVENTS: usize = 256;
const MAX_DEBOUNCE_IDENTITIES: usize = 1024;
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

pub(crate) fn tool_capability_ids() -> BTreeSet<CapabilityId> {
    let capabilities = [
        WORKSPACE_FILES,
        WORKSPACE_VCS,
        WORKSPACE_PROCESS,
        WORKSPACE_ARTIFACTS,
        nomifun_agent_domain_wave2::SSH_MODULE_ID,
    ]
        .into_iter()
        .map(CapabilityId::from)
        .collect::<BTreeSet<_>>();
    #[cfg(feature = "computer-use")]
    let capabilities = {
        let mut capabilities = capabilities;
        capabilities.insert(CapabilityId::from(
            nomifun_agent_domain_wave2::COMPUTER_MODULE_ID,
        ));
        capabilities
    };
    capabilities
}

pub(crate) fn event_capability_ids() -> BTreeSet<CapabilityId> {
    BTreeSet::from([CapabilityId::from(WORKSPACE_FILES)])
}

pub(crate) fn action_host_port(
    services: &crate::services::AppServices,
    effect_store: nomifun_agent_session::AgentSessionStore,
) -> Arc<NomiCoreWave2Host> {
    Arc::new(NomiCoreWave2Host {
        mcp: Some(super::nomi_core_mcp::NomiCoreMcpHost::for_services(services)),
        ssh: Some(nomifun_ssh::SshActionOwner::new(services.ssh_pool.clone())),
        ..Default::default()
    }
    .with_effect_store(effect_store))
}

pub(crate) fn schema_resolver(
) -> Arc<dyn nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver> {
    Arc::new(NomiCoreWave2SchemaResolver)
}

fn session_workspace_binding_id(session_id: &AgentSessionId) -> ResourceBindingId {
    ResourceBindingId::from(format!("nomi-session-workspace:{}", session_id.as_ref()))
}

/// Resolve the current Conversation's server-owned workspace to the one typed
/// binding consumed by Wave 2. Absolute root, owner and Session identity are
/// deliberately absent from every model-facing action schema.
pub(crate) fn session_workspace_binding(
    server_workspace: &str,
    owner: &PrincipalRef,
    session_id: &AgentSessionId,
    authority_binding: &TypedResourceBinding,
) -> Result<TypedResourceBinding, AppError> {
    if owner.principal_kind != "user"
        || owner.principal_id.trim().is_empty()
    {
        return Err(AppError::Conflict(
            "Nomi Wave 2 workspace binding requires the exact owned AgentSession"
                .to_owned(),
        ));
    }
    if authority_binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND
        || authority_binding.resource_id.as_ref().trim().is_empty()
        || authority_binding.owner_id != owner.principal_id
        || authority_binding.connection_config_ref.is_some()
    {
        return Err(AppError::Conflict(
            "Nomi Wave 2 AgentSession has no exact server-owned workspace resource"
                .to_owned(),
        ));
    }
    let configured = server_workspace.trim();
    if configured.is_empty() {
        return Err(
            AppError::Conflict(
                "Nomi Wave 2 AgentSession has no server-resolved workspace"
                    .to_owned(),
            )
        );
    }
    let canonical_root = canonical_workspace_root(Path::new(configured))?;
    let canonical_root = canonical_root.to_str().ok_or_else(|| {
        AppError::Conflict(
            "Nomi Wave 2 workspace root is not representable as UTF-8"
                .to_owned(),
        )
    })?;
    let binding_id = session_workspace_binding_id(session_id);
    let resource_id = authority_binding.resource_id.clone();
    let mut operations = authority_binding.operations.clone();
    // Canonical Wave 2 treats deletion as a workspace write grant. FileService
    // names its lower-level guard `delete`, so derive that internal permission
    // only from an already server-resolved write grant.
    if operations.contains(WORKSPACE_WRITE_OPERATION) {
        operations.insert(WORKSPACE_DELETE_OPERATION.to_owned());
    }
    let typed_parameters = BTreeMap::from([(
        WORKSPACE_ROOT_PARAMETER.to_owned(),
        canonical_root.to_owned(),
    )]);
    let binding = TypedResourceBinding {
        binding_id: binding_id.clone(),
        resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
        resource_id: resource_id.clone(),
        owner_id: owner.principal_id.clone(),
        operations: operations.clone(),
        connection_config_ref: None,
        typed_parameters: typed_parameters.clone(),
    };
    // Reuse the file owner's typed validation at the composition boundary.
    AgentSessionWorkspaceBinding::new(
        session_id.as_ref(),
        TypedResourceBindingDto {
            binding_id: binding_id.as_ref().to_owned(),
            resource_kind: WORKSPACE_RESOURCE_KIND.to_owned(),
            resource_id: resource_id.as_ref().to_owned(),
            owner_id: owner.principal_id.clone(),
            operations,
            connection_config_ref: None,
            typed_parameters,
        },
        PathBuf::from(canonical_root),
    )?;
    Ok(binding)
}

/// Replace only the workspace slot. Saved Agent bindings remain immutable;
/// the returned vector is the current Session target materialization.
pub(crate) fn with_session_workspace_binding(
    bindings: impl IntoIterator<Item = TypedResourceBinding>,
    workspace: TypedResourceBinding,
) -> Vec<TypedResourceBinding> {
    let mut resolved = bindings
        .into_iter()
        .filter(|binding| binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND)
        .collect::<Vec<_>>();
    resolved.push(workspace);
    resolved.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    resolved
}

pub(crate) fn canonical_workspace_root(root: &Path) -> Result<PathBuf, AppError> {
    if !root.is_absolute() {
        return Err(AppError::Conflict(
            "Nomi Wave 2 workspace root is not absolute".to_owned(),
        ));
    }
    let canonical = dunce::canonicalize(root).map_err(|error| {
        AppError::Conflict(format!(
            "Nomi Wave 2 workspace cannot be canonicalized: {error}"
        ))
    })?;
    if !canonical.is_dir() {
        return Err(AppError::Conflict(
            "Nomi Wave 2 workspace is not a directory".to_owned(),
        ));
    }
    Ok(canonical)
}

/// Multi-workspace Wave 2 host. Each canonical root gets its own existing
/// File/Snapshot/VCS owner. In particular, `VcsPushOwner`'s push lock and
/// outcome-unknown fence are cached by repository root rather than one global
/// `OnceLock` bound to the first Session.
#[derive(Default)]
pub(crate) struct NomiCoreWave2Host {
    mcp: Option<super::nomi_core_mcp::NomiCoreMcpHost>,
    ssh: Option<nomifun_ssh::SshActionOwner>,
    effect_store: Option<nomifun_agent_session::AgentSessionStore>,
    roots: Mutex<HashMap<PathBuf, Arc<Wave2ApplicationHost>>>,
    processes: Mutex<HashMap<(String, String, String), Arc<super::engine_process_host::EngineProcessScope>>>,
}

impl NomiCoreWave2Host {
    /// Inject the canonical durable Agent Session Effect owner before this
    /// host is published to the Kernel. Existing per-root owners cannot be
    /// retrofitted because that would split one workspace's admission epoch.
    pub(crate) fn with_effect_store(
        mut self,
        store: nomifun_agent_session::AgentSessionStore,
    ) -> Self {
        assert!(
            self.roots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "Agent Effect store must be injected before workspace owner creation"
        );
        self.effect_store = Some(store);
        self
    }

    pub(crate) async fn read_mcp_resource(&self, principal: PrincipalRef, session: AgentSessionId,
        operation_id: nomifun_agent_contracts::OperationId, resource: TypedResourceBinding,
        operation: nomifun_mcp::McpResourceOperation) -> Result<StrictJsonValue, AppError> {
        self.mcp.as_ref().ok_or_else(|| AppError::Conflict("MCP owner is unavailable".into()))?
            .resource(principal, session, operation_id, resource, operation).await
            .map_err(|error| AppError::Conflict(error.to_string()))
    }

    pub(crate) async fn ensure_workspace_git_evidence(&self, root: &Path) -> Result<(), AppError> {
        self.ensure_workspace_git_ready(root)?;
        Ok(())
    }

    fn retained_workspace_host(&self, root: &Path) -> Result<Option<Arc<Wave2ApplicationHost>>, AppError> {
        // A Session captures this canonical identity on admission. Cleanup must
        // not re-resolve a removed, renamed or replaced filesystem path.
        Ok(self.roots.lock().map_err(|_| AppError::Conflict("workspace owners poisoned".into()))?.get(root).cloned())
    }

    /// Current-process owner observation, not cross-boot effect reconciliation.
    pub(crate) fn ensure_workspace_git_ready(&self, root: &Path) -> Result<(), AppError> {
        if let Some(owner) = self.retained_workspace_host(root)? { owner.ensure_git_ready()?; }
        Ok(())
    }

    pub(crate) async fn settle_workspace_git(&self, root: &Path) -> Result<(), AppError> {
        if let Some(owner) = self.retained_workspace_host(root)? { owner.settle_git().await?; }
        Ok(())
    }

    pub(crate) async fn invoke_mcp_tool(&self, context: nomifun_agent_kernel::CapabilityInvocationContext, input: StrictJsonValue) -> Result<StrictJsonValue, Wave2HostPortError> {
        self.mcp.as_ref().ok_or_else(|| Wave2HostPortError::unavailable("no current-product MCP owner is configured"))?
            .invoke(context.into(), input).await
    }
    pub(crate) async fn ensure_mcp_settled(&self, user: &str, session: &str) -> Result<(), AppError> {
        match &self.mcp {
            Some(owner) => owner.ensure_settled(user, session).await,
            None => Ok(()),
        }
    }
    pub(crate) async fn ensure_mcp_source_replay_safe(&self, user: &str, session: &str, source: &str) -> Result<(), AppError> {
        match &self.mcp {
            Some(owner) => owner.ensure_source_replay_safe(user, session, source).await,
            None => Ok(()),
        }
    }
    pub(crate) async fn mcp_recovery_context(&self, user: &str, session: &str) -> Result<Option<String>, AppError> {
        match &self.mcp {
            Some(owner) => owner.recovery_context(user, session).await,
            None => Ok(None),
        }
    }
    pub(crate) async fn coding_processes_quiescent(&self, user: &str, session: &str) -> Result<bool, AppError> {
        let scopes = self.processes.lock().map_err(|_| AppError::Conflict("process scopes poisoned".into()))?
            .iter().filter(|((owner, id, _), _)| owner == user && id == session)
            .map(|(_, scope)| scope.clone()).collect::<Vec<_>>();
        for scope in scopes {
            if !scope.is_quiescent().await { return Ok(false); }
        }
        Ok(true)
    }

    pub(crate) fn open_coding_turn(&self, user: &str, session: &str, operation: &str, root: &str, journal: super::engine_journal::EngineTurnJournal) -> Result<(), AppError> {
        let key = (user.to_owned(), session.to_owned(), operation.to_owned());
        let root = canonical_workspace_root(Path::new(root))?;
        let mut scopes = self.processes.lock().map_err(|_| AppError::Conflict("process scopes poisoned".into()))?;
        if scopes.contains_key(&key) { return Err(AppError::Conflict("process turn already registered".into())); }
        let scope = super::engine_process_host::EngineProcessScope::new(&root, journal).map_err(|error| AppError::Conflict(error.to_string()))?;
        scopes.insert(key, Arc::new(scope));
        Ok(())
    }

    pub(crate) async fn cleanup_coding_session(&self, user: &str, session: &str) -> Result<(), AppError> {
        let scopes = self.processes.lock().map_err(|_| AppError::Conflict("process scopes poisoned".into()))?
            .iter().filter(|((owner, id, _), _)| owner == user && id == session)
            .map(|(key, scope)| (key.clone(), scope.clone())).collect::<Vec<_>>();
        // Close every scope before waiting; one failed cleanup must not leave
        // other scopes accepting launches or skip their cleanup attempts.
        for (_, scope) in &scopes { scope.close_admission(); }
        let mut failures = Vec::new();
        for (key, scope) in scopes {
            match scope.cleanup().await {
                Ok(()) => { self.processes.lock().map_err(|_| AppError::Conflict("process scopes poisoned".into()))?.remove(&key); }
                Err(error) => failures.push(error.to_string()),
            }
        }
        if failures.is_empty() { Ok(()) } else { Err(AppError::Conflict(failures.join("; "))) }
    }

    async fn invoke_ssh(
        &self,
        request: Wave2HostRequest,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let owner = self.ssh.as_ref().ok_or_else(|| {
            Wave2HostPortError::unavailable("the canonical SSH owner is unavailable")
        })?;
        let typed = request.into_typed()?;
        let input = match &typed.operation {
            nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshFsRead { input }
            | nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshFsWrite { input }
            | nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshExec { input }
            | nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshSudo { input } => {
                input.clone()
            }
            _ => {
                return Err(Wave2HostPortError::new(
                    "ACTION_OPERATION_MISMATCH",
                    "the SSH owner received a non-SSH operation",
                ));
            }
        };
        let [binding] = typed.context.resource_bindings.as_slice() else {
            return Err(Wave2HostPortError::resource_not_bound(
                "the SSH Module requires exactly one ssh_host binding",
            ));
        };
        if binding.resource_kind.as_ref() != nomifun_ssh::SSH_HOST_RESOURCE_KIND
            || binding.connection_config_ref.is_some()
            || binding
                .typed_parameters
                .keys()
                .any(|key| key != "remote_cwd")
        {
            return Err(Wave2HostPortError::invalid_payload(
                "the ssh_host binding contains non-canonical authority fields",
            ));
        }
        let ssh_host_id = nomifun_common::SshHostId::parse(binding.resource_id.as_ref())
            .map_err(|error| {
                Wave2HostPortError::invalid_payload(format!(
                    "the ssh_host resource identity is invalid: {error}"
                ))
            })?;
        let remote_cwd = binding
            .typed_parameters
            .get("remote_cwd")
            .cloned()
            .unwrap_or_else(|| ".".to_owned());
        let resource = nomifun_ssh::AgentSshHostResource::from_operation_names(
            binding.binding_id.as_ref(),
            binding.owner_id.clone(),
            ssh_host_id,
            remote_cwd,
            binding.operations.iter(),
        )
        .map_err(ssh_action_error)?;
        let authority = nomifun_ssh::AgentSshAuthority::bound(
            typed.context.principal.principal_id.clone(),
            resource,
        )
        .map_err(ssh_action_error)?;
        let action_context = nomifun_ssh::SshActionContext {
            principal_id: typed.context.principal.principal_id.clone(),
            agent_session_id: typed.context.agent_session_id.as_ref().to_owned(),
            operation_id: typed.context.operation_id.as_ref().to_owned(),
        };
        let dispatch = async {
            let value = match typed.operation {
                nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshFsRead { input } => {
                    let input = serde_json::from_value::<nomifun_ssh::SshFsReadInput>(input.0)
                        .map_err(|error| nomifun_ssh::SshActionError::InvalidInput(error.to_string()))?;
                    serde_json::to_value(owner.fs_read(&authority, &action_context, input).await?)
                }
                nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshFsWrite { input } => {
                    let input = serde_json::from_value::<nomifun_ssh::SshFsWriteInput>(input.0)
                        .map_err(|error| nomifun_ssh::SshActionError::InvalidInput(error.to_string()))?;
                    serde_json::to_value(owner.fs_write(&authority, &action_context, input).await?)
                }
                nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshExec { input } => {
                    let input = serde_json::from_value::<nomifun_ssh::SshExecInput>(input.0)
                        .map_err(|error| nomifun_ssh::SshActionError::InvalidInput(error.to_string()))?;
                    serde_json::to_value(owner.exec(&authority, &action_context, input).await?)
                }
                nomifun_agent_domain_wave2::Wave2TypedCapabilityOperation::SshSudo { input } => {
                    let input = serde_json::from_value::<nomifun_ssh::SshSudoInput>(input.0)
                        .map_err(|error| nomifun_ssh::SshActionError::InvalidInput(error.to_string()))?;
                    serde_json::to_value(owner.sudo(&authority, &action_context, input).await?)
                }
                _ => unreachable!("SSH operation checked above"),
            }
            .map_err(|error| nomifun_ssh::SshActionError::External(error.to_string()))?;
            Ok::<_, nomifun_ssh::SshActionError>(StrictJsonValue(value))
        };

        if typed.context.action_id.as_ref() == nomifun_ssh::SSH_FS_READ_ACTION_ID {
            return dispatch.await.map_err(ssh_action_error);
        }
        let store = self.effect_store.as_ref().ok_or_else(|| {
            Wave2HostPortError::unavailable(
                "canonical Agent Effect store is not mounted for SSH effects",
            )
        })?;
        match begin_wave2_exclusive_effect(
            store,
            &typed.context,
            binding,
            &input,
            nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
        )
        .await?
        {
            Wave2EffectAdmission::Replay(output) => Ok(output),
            Wave2EffectAdmission::Reserved(reservation) => match dispatch.await {
                Ok(output) => {
                    finish_wave2_effect(
                        &reservation,
                        Wave2EffectCompletion::Succeeded(&output),
                    )
                    .await?;
                    Ok(output)
                }
                Err(error) => {
                    let unknown = matches!(error, nomifun_ssh::SshActionError::OutcomeUnknown(_));
                    let error = ssh_action_error(error);
                    finish_wave2_effect(
                        &reservation,
                        if unknown {
                            Wave2EffectCompletion::Uncertain(&error)
                        } else {
                            Wave2EffectCompletion::Failed(&error)
                        },
                    )
                    .await?;
                    Err(error)
                }
            },
        }
    }

    fn host_for(
        &self,
        context: &Wave2HostContext,
    ) -> Result<Arc<Wave2ApplicationHost>, Wave2HostPortError> {
        if !coding_capability_ids().contains(&context.capability_id) {
            return Err(Wave2HostPortError::unavailable(format!(
                "{} is not owned by the Nomi-core Wave 2 workspace adapter",
                context.capability_id.as_ref()
            )));
        }
        let canonical_root = if context.capability_id.as_ref() == WORKSPACE_PROCESS {
            exact_session_process_root(context)?
        } else { exact_session_workspace_root(
            &context.agent_session_id,
            &context.principal.principal_id,
            &context.resource_bindings,
        )? };
        self.host_for_root(canonical_root)
    }

    fn host_for_root(
        &self,
        canonical_root: PathBuf,
    ) -> Result<Arc<Wave2ApplicationHost>, Wave2HostPortError> {
        let mut roots = self
            .roots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(owner) = roots.get(&canonical_root) {
            return Ok(Arc::clone(owner));
        }
        let mut owner = Wave2ApplicationHost::for_workspace_root(canonical_root.clone());
        if let Some(store) = &self.effect_store {
            owner = owner.with_effect_store(store.clone());
        }
        let owner = Arc::new(owner);
        roots.insert(canonical_root, Arc::clone(&owner));
        Ok(owner)
    }

    #[cfg(test)]
    fn cached_root_count(&self) -> usize {
        self.roots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

fn ssh_action_error(error: nomifun_ssh::SshActionError) -> Wave2HostPortError {
    use nomifun_ssh::SshActionError;
    match error {
        SshActionError::HostUnbound => Wave2HostPortError::resource_not_bound(error.to_string()),
        SshActionError::ResourceOwnerMismatch => {
            Wave2HostPortError::owner_mismatch(error.to_string())
        }
        SshActionError::ResourceOperationDenied { .. } => {
            Wave2HostPortError::resource_not_bound(error.to_string())
        }
        SshActionError::InvalidResource(_)
        | SshActionError::InvalidContext(_)
        | SshActionError::InvalidInput(_) => {
            Wave2HostPortError::invalid_payload(error.to_string())
        }
        SshActionError::OutcomeUnknown(_) => {
            Wave2HostPortError::new("EFFECT_OUTCOME_UNKNOWN", error.to_string())
        }
        SshActionError::External(_) => {
            Wave2HostPortError::new("SSH_ACTION_REJECTED", error.to_string())
        }
    }
}

impl Wave2HostPort for NomiCoreWave2Host {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<StrictJsonValue, Wave2HostPortError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if matches!(
                &request.operation,
                nomifun_agent_domain_wave2::Wave2CapabilityOperation::Ssh { .. }
            ) {
                return self.invoke_ssh(request).await;
            }
            let input = match &request.operation {
                nomifun_agent_domain_wave2::Wave2CapabilityOperation::WorkspaceExecution {
                    input,
                } => input,
                _ => {
                    return Err(Wave2HostPortError::unavailable(
                        "Nomi-core Wave 2 workspace owner received a non-workspace operation",
                    ));
                }
            };
            let root = if request.context.capability_id.as_ref() == WORKSPACE_PROCESS {
                exact_session_process_root(&request.context)?
            } else {
                exact_session_workspace_root(&request.context.agent_session_id,
                    &request.context.principal.principal_id, &request.context.resource_bindings)?
            };
            self.ensure_workspace_git_evidence(&root).await
                .map_err(|error| Wave2HostPortError::unavailable(error.to_string()))?;
            if request.context.capability_id.as_ref() == WORKSPACE_PROCESS {
                nomifun_agent_domain_wave2::validate_module_action_input(
                    request.context.capability_id.as_ref(),
                    request.context.action_id.as_ref(),
                    input,
                )
                    .map_err(|error| Wave2HostPortError::new("INVALID_PAYLOAD", error))?;
                let root = exact_session_process_root(&request.context)?;
                let key = (request.context.principal.principal_id.clone(), request.context.agent_session_id.as_ref().to_owned(), request.context.correlation_id.as_ref().to_owned());
                let scope = self.processes.lock().map_err(|_| Wave2HostPortError::unavailable("process scopes poisoned"))?.get(&key).cloned()
                    .ok_or_else(|| Wave2HostPortError::unavailable("no admitted Coding process turn for this exact authority"))?;
                // The process runtime uses native canonical paths (including
                // Windows extended prefixes); compare like representations.
                let native_root = std::fs::canonicalize(&root).map_err(|error| Wave2HostPortError::unavailable(error.to_string()))?;
                if scope.workspace_root() != native_root.as_path() { return Err(Wave2HostPortError::unavailable("process workspace authority changed")); }
                let owner_input = process_owner_input(request.context.action_id.as_ref(), input)?;
                if request.context.action_id.as_ref() == "workspace.process/poll" {
                    return scope
                        .invoke(owner_input, request.context.operation_id.as_ref())
                        .await;
                }
                let [binding] = request.context.resource_bindings.as_slice() else {
                    return Err(Wave2HostPortError::unavailable(
                        "process execution requires one Session process binding",
                    ));
                };
                let effect_owner = self.host_for(&request.context)?;
                let operation_id = request.context.operation_id.as_ref().to_owned();
                return effect_owner
                    .invoke_managed_effect(&request.context, binding, input, move || async move {
                        scope.invoke(owner_input, &operation_id).await
                    })
                    .await;
            }
            let owner = dispatch_validated_action_input(
                request.context.capability_id.as_ref(),
                request.context.action_id.as_ref(),
                input,
                || self.host_for(&request.context),
            )??;
            owner.invoke(request).await
        })
    }
}

fn dispatch_validated_action_input<T>(
    capability_id: &str,
    action_id: &str,
    input: &StrictJsonValue,
    dispatch: impl FnOnce() -> T,
) -> Result<T, Wave2HostPortError> {
    nomifun_agent_domain_wave2::validate_module_action_input(capability_id, action_id, input)
        .map_err(|error| Wave2HostPortError::new("INVALID_PAYLOAD", error))?;
    Ok(dispatch())
}

fn process_owner_input(
    action_id: &str,
    input: &StrictJsonValue,
) -> Result<StrictJsonValue, Wave2HostPortError> {
    let action = action_id
        .strip_prefix("workspace.process/")
        .ok_or_else(|| {
            Wave2HostPortError::new(
                "ACTION_NOT_DECLARED",
                format!("{action_id} is not a workspace.process Action"),
            )
        })?;
    let operation = if action == "input" { "stdin" } else { action };
    let mut object = input.0.as_object().cloned().ok_or_else(|| {
        Wave2HostPortError::invalid_payload("workspace.process input must be an object")
    })?;
    if object.contains_key("operation") {
        return Err(Wave2HostPortError::invalid_payload(
            "workspace.process operation is selected by the Action ID, not the payload",
        ));
    }
    object.insert("operation".to_owned(), json!(operation));
    Ok(StrictJsonValue(serde_json::Value::Object(object)))
}

/// A process grant never implies a file grant. Only the host may substitute
/// the current Session root into an already authorized process resource.
pub(crate) fn session_process_binding(
    server_workspace: &str,
    owner: &PrincipalRef,
    session_id: &AgentSessionId,
    authority: &TypedResourceBinding,
) -> Result<TypedResourceBinding, AppError> {
    if owner.principal_kind != "user" || owner.principal_id.is_empty()
        || authority.resource_kind.as_ref() != "process_session"
        || authority.owner_id != owner.principal_id
        || authority.resource_id.as_ref().is_empty()
        || authority.connection_config_ref.is_some()
        || !authority.operations.contains("execute")
        || server_workspace.trim().is_empty()
    {
        return Err(AppError::Conflict("Coding requires an owned process execute grant".into()));
    }
    let root = canonical_workspace_root(Path::new(server_workspace))?;
    let root = root.to_str().ok_or_else(|| AppError::Conflict("process root is not UTF-8".into()))?;
    let mut binding = authority.clone();
    binding.binding_id = ResourceBindingId::from(format!("coding-session-process:{}", session_id.as_ref()));
    binding.typed_parameters = BTreeMap::from([(WORKSPACE_ROOT_PARAMETER.into(), root.into())]);
    Ok(binding)
}

fn exact_session_process_root(context: &Wave2HostContext) -> Result<PathBuf, Wave2HostPortError> {
    let [binding] = context.resource_bindings.as_slice() else {
        return Err(Wave2HostPortError::unavailable("process execution requires one Session process binding"));
    };
    if binding.binding_id.as_ref() != format!("coding-session-process:{}", context.agent_session_id.as_ref())
        || binding.resource_kind.as_ref() != "process_session"
        || binding.owner_id != context.principal.principal_id
        || context.principal.principal_kind != "user"
        || binding.resource_id.as_ref().is_empty()
        || !binding.operations.contains("execute")
        || binding.connection_config_ref.is_some()
        || binding.typed_parameters.len() != 1
    {
        return Err(Wave2HostPortError::unavailable("process execution has no exact Session authority"));
    }
    let raw = binding.typed_parameters.get(WORKSPACE_ROOT_PARAMETER)
        .ok_or_else(|| Wave2HostPortError::unavailable("process root is missing"))?;
    let root = canonical_workspace_root(Path::new(raw))
        .map_err(|error| Wave2HostPortError::unavailable(error.to_string()))?;
    if root != PathBuf::from(raw) {
        return Err(Wave2HostPortError::unavailable("process root is not canonical"));
    }
    Ok(root)
}

fn exact_session_workspace_root(
    agent_session_id: &AgentSessionId,
    principal_id: &str,
    resource_bindings: &[TypedResourceBinding],
) -> Result<PathBuf, Wave2HostPortError> {
    let [binding] = resource_bindings else {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Nomi-core Wave 2 action requires one exact Session workspace binding",
        ));
    };
    if binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND
        || binding.binding_id != session_workspace_binding_id(agent_session_id)
        || binding.resource_id.as_ref().trim().is_empty()
    {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Nomi-core Wave 2 action received a non-Session workspace binding",
        ));
    }
    if binding.owner_id != principal_id {
        return Err(Wave2HostPortError::new(
            "RESOURCE_OWNER_MISMATCH",
            "Nomi-core Wave 2 workspace belongs to a different principal",
        ));
    }
    if binding.connection_config_ref.is_some()
        || binding.typed_parameters.len() != 1
    {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Nomi-core Session workspace binding contains unsupported authority fields",
        ));
    }
    let raw_root = binding
        .typed_parameters
        .get(WORKSPACE_ROOT_PARAMETER)
        .ok_or_else(|| {
            Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Nomi-core Session workspace binding has no server-resolved root",
            )
        })?;
    let supplied = PathBuf::from(raw_root);
    let canonical = canonical_workspace_root(&supplied).map_err(|error| {
        Wave2HostPortError::new("PRESET_RESOURCE_NOT_BOUND", error.to_string())
    })?;
    if supplied != canonical {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Nomi-core Session workspace binding is not canonical",
        ));
    }
    Ok(canonical)
}

#[derive(Default)]
struct WatchQueue {
    events: VecDeque<WorkspaceFileChangedEvent>,
    debounce: HashMap<String, Instant>,
    dropped: u64,
}

impl WatchQueue {
    fn push(&mut self, event: WorkspaceFileChangedEvent) {
        let now = Instant::now();
        let debounce_key = format!("{:?}\0{}", event.kind, event.path);
        if self
            .debounce
            .get(&debounce_key)
            .is_some_and(|previous| now.duration_since(*previous) < WATCH_DEBOUNCE)
        {
            return;
        }
        if self.debounce.len() >= MAX_DEBOUNCE_IDENTITIES {
            self.debounce
                .retain(|_, previous| now.duration_since(*previous) < WATCH_DEBOUNCE);
        }
        if self.debounce.len() < MAX_DEBOUNCE_IDENTITIES {
            self.debounce.insert(debounce_key, now);
        }
        if self.events.len() == MAX_WATCH_EVENTS {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }

    fn drain(&mut self) -> (Vec<WorkspaceFileChangedEvent>, u64) {
        let events = self.events.drain(..).collect();
        let dropped = std::mem::take(&mut self.dropped);
        (events, dropped)
    }
}

/// Live `workspace.files` changed Event projected into per-turn system context.
/// It owns no public watch ID; dropping the Session-held contributor drops the
/// OS watcher and cancels the subscription.
pub(crate) struct NomiWorkspaceWatchContext {
    root: PathBuf,
    watcher: Mutex<RecommendedWatcher>,
    queue: Arc<Mutex<WatchQueue>>,
}

impl NomiWorkspaceWatchContext {
    pub(crate) fn start(root: impl AsRef<Path>) -> Result<Arc<Self>, AppError> {
        let root = canonical_workspace_root(root.as_ref())?;
        let queue = Arc::new(Mutex::new(WatchQueue::default()));
        let callback_queue = Arc::clone(&queue);
        let callback_root = root.clone();
        let mut watcher = notify::recommended_watcher(
            move |result: Result<notify::Event, notify::Error>| {
                let Ok(event) = result else {
                    let mut queue = callback_queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    queue.dropped = queue.dropped.saturating_add(1);
                    return;
                };
                let Some(kind) = watch_operation(&event.kind) else {
                    return;
                };
                let mut queue = callback_queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for path in event.paths {
                    let Ok(relative) = path.strip_prefix(&callback_root) else {
                        continue;
                    };
                    if relative.components().next().is_some_and(|component| {
                        nomifun_file::is_workspace_owner_component(component.as_os_str())
                    }) {
                        continue;
                    }
                    let path = relative.to_string_lossy().replace('\\', "/");
                    if path.is_empty() {
                        continue;
                    }
                    queue.push(WorkspaceFileChangedEvent { path, kind });
                }
            },
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "workspace.files could not create a workspace watcher: {error}"
            ))
        })?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| {
                AppError::Internal(format!(
                    "workspace.files could not subscribe to the Session workspace: {error}"
                ))
            })?;
        Ok(Arc::new(Self {
            root,
            watcher: Mutex::new(watcher),
            queue,
        }))
    }

    #[cfg(test)]
    fn push_for_test(&self, path: &str, kind: WorkspaceFileChangeKind) {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(WorkspaceFileChangedEvent {
                path: path.to_owned(),
                kind,
            });
    }
}

impl Drop for NomiWorkspaceWatchContext {
    fn drop(&mut self) {
        if let Ok(mut watcher) = self.watcher.lock() {
            let _ = watcher.unwatch(&self.root);
        }
    }
}

#[async_trait::async_trait]
impl ContextContributor for NomiWorkspaceWatchContext {
    async fn pre_turn_context(&self) -> Option<String> {
        let (events, dropped) = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain();
        if events.is_empty() && dropped == 0 {
            return None;
        }
        let batch = WorkspaceFilesChangedBatch::new(events, dropped);
        batch.validate().ok()?;
        serde_json::to_string(&batch)
        .ok()
        .map(|payload| {
            format!(
                "<nomifun_workspace_events format=\"canonical-json\">\n{payload}\n</nomifun_workspace_events>"
            )
        })
    }

    fn label(&self) -> &str {
        WORKSPACE_FILES
    }
}

fn watch_operation(kind: &EventKind) -> Option<WorkspaceFileChangeKind> {
    match kind {
        EventKind::Create(_) => Some(WorkspaceFileChangeKind::Created),
        EventKind::Modify(ModifyKind::Name(_)) => Some(WorkspaceFileChangeKind::Renamed),
        EventKind::Modify(_) => Some(WorkspaceFileChangeKind::Modified),
        EventKind::Remove(_) => Some(WorkspaceFileChangeKind::Removed),
        EventKind::Any | EventKind::Other => Some(WorkspaceFileChangeKind::Other),
        EventKind::Access(_) => None,
    }
}

/// Exact input schema source for every admitted workspace Module Action.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NomiCoreWave2SchemaResolver;

#[async_trait::async_trait]
impl nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver
    for NomiCoreWave2SchemaResolver
{
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        if !tool_capability_ids().contains(&capability.capability.id) {
            return Err(format!(
                "{} is not an admitted Nomi-core Wave 2 Tool",
                capability.capability.id.as_ref()
            ));
        }
        nomifun_agent_domain_wave2::resolve_action_schema(
            capability.capability.id.as_ref(),
            reference,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::ResourceId;
    use nomifun_file::WORKSPACE_READ_OPERATION;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn session_id() -> AgentSessionId {
        AgentSessionId::from(nomifun_common::generate_id())
    }

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: nomifun_common::generate_id(),
        }
    }

    fn authority(principal: &PrincipalRef) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("workspace:default-workspace"),
            resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
            resource_id: ResourceId::from("default-workspace"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from([
                WORKSPACE_READ_OPERATION.to_owned(),
                WORKSPACE_WRITE_OPERATION.to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                WORKSPACE_ROOT_PARAMETER.to_owned(),
                "server-selection-root-is-replaced".to_owned(),
            )]),
        }
    }

    #[test]
    fn session_binding_replaces_saved_workspace_and_hides_authority_from_actions() {
        let root = tempfile::tempdir().unwrap();
        let session_id = session_id();
        let principal = principal();
        let binding = session_workspace_binding(
            &root.path().to_string_lossy(),
            &principal,
            &session_id,
            &authority(&principal),
        )
        .unwrap();
        assert_eq!(binding.owner_id, principal.principal_id);
        assert_eq!(binding.typed_parameters.len(), 1);
        assert_eq!(
            Path::new(&binding.typed_parameters[WORKSPACE_ROOT_PARAMETER]),
            dunce::canonicalize(root.path()).unwrap()
        );

        let stale = TypedResourceBinding {
            binding_id: ResourceBindingId::from("stale"),
            resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
            resource_id: ResourceId::from("stale"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let replaced = with_session_workspace_binding([stale], binding.clone());
        assert_eq!(replaced, vec![binding]);
    }

    #[tokio::test]
    async fn watch_context_is_bounded_relative_and_drained_per_turn() {
        let root = tempfile::tempdir().unwrap();
        let watch = NomiWorkspaceWatchContext::start(root.path()).unwrap();
        for index in 0..=MAX_WATCH_EVENTS {
            watch.push_for_test(
                &format!("src/{index}.rs"),
                WorkspaceFileChangeKind::Modified,
            );
        }
        let context = watch.pre_turn_context().await.unwrap();
        assert!(!context.contains(&root.path().to_string_lossy().into_owned()));
        let json = context
            .lines()
            .nth(1)
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .unwrap();
        assert_eq!(json["capability_id"], WORKSPACE_FILES);
        assert_eq!(json["event_schema"], "workspace.files/changed");
        assert_eq!(json["events"].as_array().unwrap().len(), MAX_WATCH_EVENTS);
        assert_eq!(json["events"][0]["kind"], "modified");
        assert_eq!(json["dropped_event_count"], 1);
        let batch: WorkspaceFilesChangedBatch = serde_json::from_value(json).unwrap();
        batch.validate().unwrap();
        assert!(watch.pre_turn_context().await.is_none());
    }

    #[test]
    fn watch_debounce_identity_map_has_a_hard_bound() {
        let mut queue = WatchQueue::default();
        let total = MAX_DEBOUNCE_IDENTITIES + 128;
        for index in 0..total {
            queue.push(WorkspaceFileChangedEvent {
                path: format!("src/{index}.rs"),
                kind: WorkspaceFileChangeKind::Modified,
            });
        }
        assert!(queue.debounce.len() <= MAX_DEBOUNCE_IDENTITIES);
        assert_eq!(queue.events.len(), MAX_WATCH_EVENTS);
        assert_eq!(queue.dropped, (total - MAX_WATCH_EVENTS) as u64);
    }

    #[tokio::test]
    async fn watch_context_receives_real_recursive_filesystem_events() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let watch = NomiWorkspaceWatchContext::start(root.path()).unwrap();
        std::fs::write(nested.join("live.txt"), b"first").unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let observed = loop {
            if let Some(context) = watch.pre_turn_context().await
                && context.contains("nested/live.txt")
            {
                break context;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "recursive watcher did not publish the real file event"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(!observed.contains(&root.path().to_string_lossy().into_owned()));
    }

    #[test]
    fn watch_context_rejects_a_missing_workspace_before_activation() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("missing-workspace");
        let error = NomiWorkspaceWatchContext::start(&missing)
            .err()
            .expect("a missing workspace must not produce an active watcher");
        assert!(error.to_string().contains("workspace"));
    }

    #[test]
    fn action_owner_cache_is_keyed_by_canonical_workspace() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let host = NomiCoreWave2Host::default();
        for root in [first.path(), first.path(), second.path()] {
            let session_id = session_id();
            let principal = principal();
            let binding = session_workspace_binding(
                &root.to_string_lossy(),
                &principal,
                &session_id,
                &authority(&principal),
            )
            .unwrap();
            let canonical = exact_session_workspace_root(
                &session_id,
                &principal.principal_id,
                &[binding],
            )
            .unwrap();
            host.host_for_root(canonical).unwrap();
        }
        assert_eq!(host.cached_root_count(), 2);
    }

    #[test]
    fn action_owner_rejects_model_style_root_or_session_spoofing() {
        let root = tempfile::tempdir().unwrap();
        let session_id = session_id();
        let principal = principal();
        let mut binding = session_workspace_binding(
            &root.path().to_string_lossy(),
            &principal,
            &session_id,
            &authority(&principal),
        )
        .unwrap();
        binding.binding_id = ResourceBindingId::from("model-supplied");
        let error = exact_session_workspace_root(
            &session_id,
            &principal.principal_id,
            &[binding],
        )
        .expect_err("spoofed binding must fail closed");
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[test]
    fn invalid_model_payloads_never_dispatch_to_a_workspace_owner() {
        let dispatches = AtomicUsize::new(0);
        for (capability_id, action_id, input) in [
            (
                WORKSPACE_FILES,
                "workspace.files/delete",
                json!({"path": "ok", "workspace_root": "C:/spoof"}),
            ),
            (
                WORKSPACE_ARTIFACTS,
                "workspace.artifacts/publish",
                json!({"path": ""}),
            ),
            (
                WORKSPACE_VCS,
                "workspace.vcs/push",
                json!({
                    "remote": "origin",
                    "refspec": "HEAD:refs/heads/main",
                    "force": true
                }),
            ),
        ] {
            let error = dispatch_validated_action_input(
                capability_id,
                action_id,
                &StrictJsonValue(input),
                || dispatches.fetch_add(1, Ordering::AcqRel),
            )
            .expect_err("invalid model payload must be rejected before dispatch");
            assert_eq!(error.code, "INVALID_PAYLOAD");
        }
        assert_eq!(dispatches.load(Ordering::Acquire), 0);
    }

    #[test]
    fn process_action_identity_is_projected_only_inside_the_owner_boundary() {
        let input = StrictJsonValue(json!({"process_id": "p", "input": "hello"}));
        let projected = process_owner_input("workspace.process/input", &input).unwrap();
        assert_eq!(projected.0["operation"], "stdin");
        assert_eq!(projected.0["process_id"], "p");
        assert!(process_owner_input(
            "workspace.process/input",
            &StrictJsonValue(json!({
                "operation": "cancel",
                "process_id": "p",
                "input": "hello"
            })),
        )
        .is_err());
    }
}
