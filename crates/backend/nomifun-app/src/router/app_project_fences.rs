//! Transient coordination for the exact canonical workspace shared by canonical
//! Sessions. This uses the existing admission lock registry, never a resource,
//! checkpoint, membership, authorization, or effect ledger.
use super::*;

#[derive(Debug)]
pub(crate) struct AppProjectWorkspaceFence {
    _directory: tokio::sync::OwnedRwLockWriteGuard<()>,
    _sessions: Vec<tokio::sync::OwnedRwLockWriteGuard<()>>,
    pub(super) session_ids: Vec<AgentSessionId>,
    _root: std::path::PathBuf,
}

#[derive(Debug)]
pub(crate) struct AppProjectDeletionFence {
    _project: tokio::sync::OwnedRwLockWriteGuard<()>,
    _workspace: AppProjectWorkspaceFence,
    app_session_ids: Vec<AgentSessionId>,
}

#[derive(Debug)]
pub(crate) struct AgentAppProjectImportFence {
    _directory: tokio::sync::OwnedRwLockWriteGuard<()>,
    _sessions: Vec<tokio::sync::OwnedRwLockWriteGuard<()>>,
    _mutation: super::super::nomi_core_wave2::WorkspaceMutationGuard,
}

/// A borrowed view of the Kernel invocation. This has no persisted identity or
/// permissions of its own; the receipt and current canonical facts must match.
pub(super) struct AppImportAuthority<'a> {
    pub principal: &'a PrincipalRef,
    pub session: &'a AgentSessionId,
    pub turn: &'a OperationId,
    pub operation: &'a OperationId,
    pub snapshot: &'a nomifun_agent_contracts::ResolvedSnapshotRef,
}

pub(crate) async fn guard_agent_app_project_import(owner: &NomiCoreSessionOwner,
    workspace_owner: &super::super::nomi_core_wave2::NomiCoreWave2Host,
    context: &nomifun_agent_kernel::CapabilityInvocationContext,
    receipt: &nomifun_agent_session::EffectEventRequest, app: &nomifun_app_hosting::AppRecord)
    -> Result<AgentAppProjectImportFence, AppError> {
    if context.capability_id.as_ref() != "app.hosting" || context.action_id.as_ref() != "app.hosting/import" {
        return Err(AppError::Conflict("Project import requires the exact admitted application import action".into()));
    }
    guard_agent_import_exact(owner, workspace_owner, AppImportAuthority { principal: &context.principal,
        session: &context.agent_session_id, turn: &context.turn_id, operation: &context.operation_id,
        snapshot: &context.resolved_snapshot_ref }, receipt, app).await
}

pub(super) async fn guard_agent_import_exact(owner: &NomiCoreSessionOwner,
    workspace_owner: &super::super::nomi_core_wave2::NomiCoreWave2Host, authority: AppImportAuthority<'_>,
    receipt: &nomifun_agent_session::EffectEventRequest, app: &nomifun_app_hosting::AppRecord)
    -> Result<AgentAppProjectImportFence, AppError> {
    let blocked = || AppError::Conflict("Application project import is fenced by active work or unsettled effects".into());
    let root = canonical_existing_workspace_directory(&app.directory, WorkspaceDirectoryCheck::Create)?;
    let directory = directory_lock(owner, &authority.principal.principal_id, &root).try_write_owned().map_err(|_| blocked())?;
    let store = owner.canonical().store();
    let workspace = root.to_str().ok_or_else(blocked)?;
    let mut ids = project_sessions(store, authority.principal, workspace).await.map_err(|error| AppError::Conflict(error.message))?;
    ids.push(authority.session.clone());
    ids.sort_by(|left, right| left.as_ref().cmp(right.as_ref()));
    ids.dedup();
    let mut sessions = Vec::with_capacity(ids.len());
    for id in &ids {
        sessions.push(owner.session_operation_lock(id.as_ref()).try_write_owned().map_err(|_| blocked())?);
        let current = store.get_live_session(id).await.map_err(agent_session_store_error)?;
        if current.owner_ref != *authority.principal { return Err(blocked()); }
        let head = store.head(id).await.map_err(agent_session_store_error)?;
        let native = store.inspect_latest_native_execution(authority.principal, id).await.map_err(agent_session_store_error)?;
        if id == authority.session {
            if authority.principal.principal_kind != "user"
                || current.agent_binding.resolved_snapshot_ref != *authority.snapshot
                || head.active_turn_id.as_deref() != Some(authority.turn.as_ref())
                || receipt.agent_session_id != *authority.session || receipt.turn_id != *authority.turn
                || receipt.operation_id != *authority.operation || receipt.owner_domain != "app_hosting"
                || receipt.capability_module.as_ref() != "app.hosting" || receipt.action_id.as_ref() != "app.hosting/import"
                || receipt.strategy != nomifun_agent_session::EffectStrategy::ManagedEffect { return Err(blocked()); }
            let target = super::super::hosted_effect_receipts::application_resource_key(&authority.principal.principal_id,
                "app.hosting/import", &json!({"app_id":app.id})).await?;
            if receipt.resource_key.as_deref() != Some(target.as_str()) { return Err(blocked()); }
            let actual = store.read_effect(id, &receipt.effect_id).await.map_err(agent_session_store_error)?.ok_or_else(blocked)?;
            if actual.agent_session_id != receipt.agent_session_id || actual.turn_id != receipt.turn_id
                || actual.operation_id != receipt.operation_id || actual.owner_domain != receipt.owner_domain
                || actual.capability_module != receipt.capability_module || actual.action_id != receipt.action_id
                || actual.resource_key != receipt.resource_key || actual.input_digest != receipt.input_digest
                || actual.started_event_id != receipt.event_id || actual.strategy != receipt.strategy
                || actual.state != nomifun_agent_session::AgentEffectState::Pending { return Err(blocked()); }
            let causation = store.effect_causation_event_id(id, authority.turn, authority.operation,
                &receipt.capability_module, &receipt.action_id).await.map_err(agent_session_store_error)?;
            if receipt.causation_event_id.as_ref() != Some(&causation) { return Err(blocked()); }
            // Only this retained invocation's pending receipt is expected.
            // Do not clear or ignore another effect, even within this Turn.
            if store.list_effects(id).await.map_err(agent_session_store_error)?.iter().any(|effect|
                matches!(effect.state, nomifun_agent_session::AgentEffectState::Pending | nomifun_agent_session::AgentEffectState::Unknown)
                    && effect.effect_id != receipt.effect_id) { return Err(blocked()); }
            if native.is_some_and(|native| native.operation_id != *authority.turn || native.state != "running"
                || native.unknown_effects > 0 || native.pending_effects != 1 || native.recovery_blocked
                || native.pause.is_some() || native.pause_requested) { return Err(blocked()); }
        } else if head.active_turn_id.is_some() || store.has_unsettled_effects(id).await.map_err(agent_session_store_error)?
            || native.is_some_and(|native| native.pending_effects > 0 || native.unknown_effects > 0 || native.producer_lease_live) {
            return Err(blocked());
        }
    }
    // The existing native mutex closes the caller's own parallel-file-tool
    // window; a different root is unaffected. No broad tool lock is added.
    let mutation = workspace_owner.try_guard_workspace_mutation(&root)?;
    Ok(AgentAppProjectImportFence { _directory: directory, _sessions: sessions, _mutation: mutation })
}

pub(super) fn directory_lock(owner: &NomiCoreSessionOwner, owner_id: &str, root: &std::path::Path)
    -> Arc<tokio::sync::RwLock<()>> {
    // The path is canonical, or the same verified genuinely missing frozen
    // anchor. Both spellings retain the original directory lock identity.
    // Delimit and hash both fields so arbitrary path text cannot collide with
    // a Session identity or another authenticated owner's admission key.
    let identity = json!({"owner":owner_id,"root":root.to_string_lossy()});
    let digest = digest_payload(&identity).expect("plain JSON identity is canonicalizable");
    owner.session_operation_lock(&format!("workspace:{}", digest.as_ref()))
}

fn project_lock(owner: &NomiCoreSessionOwner, owner_id: &str, app_id: &str) -> Arc<tokio::sync::RwLock<()>> {
    let identity = json!({"owner":owner_id,"app":app_id});
    let digest = digest_payload(&identity).expect("plain JSON identity is canonicalizable");
    owner.session_operation_lock(&format!("app-project:{}", digest.as_ref()))
}

fn busy() -> NomiCoreApiError {
    NomiCoreApiError::new(StatusCode::CONFLICT, "APP_DEVELOPMENT_BUSY",
        "Wait for the application project's active work and resolve unsettled effects before changing its files")
}

pub(super) async fn project_sessions(store: &nomifun_agent_session::AgentSessionStore, principal: &PrincipalRef, workspace: &str)
    -> Result<Vec<AgentSessionId>, NomiCoreApiError> {
    // A deleting Session is absent from ordinary history, but can still own
    // an unconfirmed file outcome or native cleanup. Keep its exact current
    // selected directory protected until its normal cleanup completes.
    if store.list_deleting_sessions().await.map_err(agent_session_store_error)?.iter().any(|session|
        session.owner_ref == *principal && has_selected_workspace(&session.agent_binding)
            && session.agent_binding.typed_resource_bindings.iter().any(|binding|
                binding.resource_kind.as_ref() == "workspace" && binding.owner_id == principal.principal_id
                    && binding.resource_id.as_ref().starts_with(SELECTED_WORKSPACE_RESOURCE_PREFIX)
                    && binding.typed_parameters.get("workspace_root").is_some_and(|root| root == workspace))) {
        return Err(busy());
    }
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = store.list_live_sessions_for_workspace(principal, cursor.as_deref(), 1000, workspace).await
            .map_err(agent_session_store_error)?;
        ids.extend(page.items.into_iter().map(|item| item.session.agent_session_id));
        if !page.has_more { break; }
        let next = page.next_cursor.ok_or_else(|| AppError::Conflict("Project Session history cursor is missing".into()))?;
        if cursor.as_ref() == Some(&next) { return Err(AppError::Conflict("Project Session history cursor did not advance".into()).into()); }
        cursor = Some(next);
    }
    ids.sort_by(|left, right| left.as_ref().cmp(right.as_ref()));
    ids.dedup();
    Ok(ids)
}

pub(crate) fn guard_app_project_directory_read(state: &NomiCoreAgentApiState, owner_id: &str,
    directory: &std::path::Path) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, NomiCoreApiError> {
    if owner_id != state.authoritative_user_id.as_ref() {
        return Err(AppError::Forbidden("Application project owner mismatch".into()).into());
    }
    let root = canonical_existing_workspace_directory(directory, WorkspaceDirectoryCheck::Create)?;
    directory_lock(&state.session_owner, owner_id, &root).try_read_owned().map_err(|_| busy())
}

pub(crate) fn guard_app_project_scope_read(state: &NomiCoreAgentApiState, owner_id: &str, app_id: &str)
    -> Result<tokio::sync::OwnedRwLockReadGuard<()>, NomiCoreApiError> {
    if owner_id != state.authoritative_user_id.as_ref() {
        return Err(AppError::Forbidden("Application project owner mismatch".into()).into());
    }
    state.session_owner.guard_app_project_scope_read(owner_id, app_id)
}

impl NomiCoreSessionOwner {
    /// Scope admission is independent of a Session's present workspace. This
    /// also fences current-generation scoped history whose old root changed.
    pub(super) fn guard_app_project_scope_read(&self, owner_id: &str, app_id: &str)
        -> Result<tokio::sync::OwnedRwLockReadGuard<()>, NomiCoreApiError> {
        project_lock(self, owner_id, app_id).try_read_owned().map_err(|_| busy())
    }
    /// Called while the caller already protects the exact Session binding.
    /// A try-read avoids a directory-write/Session-write waiting cycle.
    pub(super) fn guard_binding_workspace_read(&self, owner_id: &str, binding: &AgentBindingValue)
        -> Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>, AppError> {
        // The default catalog freezes the installation resource, not an
        // existing project directory. Its exact Session-owned child is still
        // validated and created by the ordinary owned materializer below.
        if uses_managed_session_workspace(binding) { return Ok(None); }
        let resources = binding.typed_resource_bindings.iter().filter(|resource|
            resource.resource_kind.as_ref() == "workspace" && resource.owner_id == owner_id);
        let mut root = None;
        for resource in resources {
            if let Some(path) = resource.typed_parameters.get("workspace_root") {
                let canonical = nomifun_common::paths::simplified(
                    &nomifun_knowledge::resolve_unbound_workspace_anchor(std::path::Path::new(path))?);
                if root.as_ref().is_some_and(|previous| previous != &canonical) {
                    return Err(AppError::Conflict("A Session cannot admit work in ambiguous project directories".into()));
                }
                root = Some(canonical);
            }
        }
        root.map(|root| directory_lock(self, owner_id, &root).try_read_owned()
            .map_err(|_| AppError::Conflict("Application project files are being changed; retry after that operation finishes".into())))
            .transpose()
    }
}

pub(super) async fn workspace_change(state: &NomiCoreAgentApiState, owner_id: &str,
    app: &nomifun_app_hosting::AppRecord, wait_for_completion_release: bool)
    -> Result<AppProjectWorkspaceFence, NomiCoreApiError> {
    if owner_id != state.authoritative_user_id.as_ref() {
        return Err(AppError::Forbidden("Application project owner mismatch".into()).into());
    }
    let root = canonical_existing_workspace_directory(&app.directory, WorkspaceDirectoryCheck::Create)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let lock = directory_lock(&state.session_owner, owner_id, &root);
    let directory = if wait_for_completion_release {
        tokio::time::timeout_at(deadline, lock.write_owned()).await.map_err(|_| busy())?
    } else { lock.try_write_owned().map_err(|_| busy())? };
    let principal = authenticated_principal(&AuthenticatedOwner(UserId::from(owner_id.to_owned())));
    let store = state.session_owner.canonical().store();
    let workspace = root.to_str().ok_or_else(|| AppError::Conflict("Application project directory is not UTF-8".into()))?;
    let ids = project_sessions(store, &principal, workspace).await?;
    let mut sessions = Vec::with_capacity(ids.len());
    // No new Session, root-changing binding or accepted Turn can cross the
    // directory write fence. These Session locks protect existing exact facts.
    for id in &ids {
        let lock = state.session_owner.session_operation_lock(id.as_ref());
        let fence = if wait_for_completion_release {
            tokio::time::timeout_at(deadline, lock.write_owned()).await.map_err(|_| busy())?
        } else { lock.try_write_owned().map_err(|_| busy())? };
        sessions.push(fence);
        // The typed Store page already verified owner, purpose and current
        // workspace. File coordination reads its live head, not a transcript.
        let head = store.head(id).await.map_err(agent_session_store_error)?;
        let native = store.inspect_latest_native_execution(&principal, id).await.map_err(agent_session_store_error)?;
        if head.active_turn_id.is_some() || store.has_unsettled_effects(id).await.map_err(agent_session_store_error)?
            || native.is_some_and(|execution| execution.pending_effects > 0 || execution.unknown_effects > 0 || execution.producer_lease_live) {
            return Err(busy());
        }
    }
    Ok(AppProjectWorkspaceFence { _directory: directory, _sessions: sessions, session_ids: ids, _root: root })
}

/// Directory safety still covers every real same-root owner. Session deletion
/// follows immutable app scope, including app Sessions already fenced for
/// cleanup, and never infers ownership from the current directory or Agent.
pub(super) async fn deletion(state: &NomiCoreAgentApiState, owner_id: &str,
    app: &nomifun_app_hosting::AppRecord) -> Result<AppProjectDeletionFence, NomiCoreApiError> {
    if owner_id != state.authoritative_user_id.as_ref() {
        return Err(AppError::Forbidden("Application project owner mismatch".into()).into());
    }
    let project = project_lock(&state.session_owner, owner_id, &app.id).try_write_owned().map_err(|_| busy())?;
    let root = canonical_existing_workspace_directory(&app.directory, WorkspaceDirectoryCheck::Create)?;
    let directory = directory_lock(&state.session_owner, owner_id, &root).try_write_owned().map_err(|_| busy())?;
    let principal = authenticated_principal(&AuthenticatedOwner(UserId::from(owner_id.to_owned())));
    let store = state.session_owner.canonical().store();
    let workspace = root.to_str().ok_or_else(|| AppError::Conflict("Application project directory is not UTF-8".into()))?;
    let app_session_ids = store.list_app_project_session_members(&principal, &app.id).await.map_err(agent_session_store_error)?;
    let mut ids = app_session_ids.clone();
    let mut cursor = None;
    loop {
        let page = store.list_live_sessions_for_workspace(&principal, cursor.as_deref(), 1000, workspace).await.map_err(agent_session_store_error)?;
        ids.extend(page.items.into_iter().map(|item| item.session.agent_session_id));
        if !page.has_more { break; }
        let next = page.next_cursor.ok_or_else(|| AppError::Conflict("Project Session history cursor is missing".into()))?;
        if cursor.as_ref() == Some(&next) { return Err(busy()); }
        cursor = Some(next);
    }
    for session in store.list_deleting_sessions().await.map_err(agent_session_store_error)? {
        if session.owner_ref == principal && has_selected_workspace(&session.agent_binding)
            && session.agent_binding.typed_resource_bindings.iter().any(|binding|
                binding.resource_kind.as_ref() == "workspace" && binding.owner_id == owner_id
                    && binding.resource_id.as_ref().starts_with(SELECTED_WORKSPACE_RESOURCE_PREFIX)
                    && binding.typed_parameters.get("workspace_root").is_some_and(|selected| selected == workspace)) {
            // An unrelated deleting Session owns its own cleanup and must not
            // be swept into this application's lifecycle.
            if !app_session_ids.contains(&session.agent_session_id) { return Err(busy()); }
            ids.push(session.agent_session_id);
        }
    }
    ids.sort_by(|left, right| left.as_ref().cmp(right.as_ref()));
    ids.dedup();
    let mut sessions = Vec::with_capacity(ids.len());
    for id in &ids {
        sessions.push(state.session_owner.session_operation_lock(id.as_ref()).try_write_owned().map_err(|_| busy())?);
        match store.get_live_session(id).await {
            Ok(_) => {}
            Err(nomifun_agent_session::SessionStoreError::Deleted(_)) => {
                let deleting = store.get_deleting_session(id).await.map_err(agent_session_store_error)?;
                if deleting.owner_ref != principal || deleting.metadata.app_project_id.as_deref() != Some(app.id.as_str()) {
                    return Err(busy());
                }
                let blockers = store.delete_blockers(id).await.map_err(agent_session_store_error)?;
                if delete_cleanup_requires_reconciliation(&blockers) {
                    return Err(agent_session_delete_blocked(id.as_ref(), &blockers));
                }
                let native = store.inspect_latest_native_execution_for_delete(&principal, id).await.map_err(agent_session_store_error)?;
                if native.is_some_and(|execution| execution.turn_state == "running" || execution.producer_lease_live
                    || execution.pending_effects > 0 || execution.unknown_effects > 0) {
                    return Err(busy());
                }
                // The existing delete owner proves quiescence and resumes its
                // exact cleanup. Live-only history/head reads are invalid here.
                continue;
            }
            Err(error) => return Err(agent_session_store_error(error).into()),
        }
        let head = store.head(id).await.map_err(agent_session_store_error)?;
        let native = store.inspect_latest_native_execution(&principal, id).await.map_err(agent_session_store_error)?;
        if head.active_turn_id.is_some() || store.has_unsettled_effects(id).await.map_err(agent_session_store_error)?
            || native.is_some_and(|execution| execution.pending_effects > 0 || execution.unknown_effects > 0 || execution.producer_lease_live) {
            return Err(busy());
        }
    }
    Ok(AppProjectDeletionFence { _project: project, _workspace: AppProjectWorkspaceFence {
        _directory: directory, _sessions: sessions, session_ids: ids, _root: root }, app_session_ids })
}

pub(super) async fn delete_scoped_sessions(state: &NomiCoreAgentApiState, owner_id: &str,
    app: &nomifun_app_hosting::AppRecord, fence: &AppProjectDeletionFence) -> Result<(), NomiCoreApiError> {
    let owner = AuthenticatedOwner(UserId::from(owner_id.to_owned()));
    let principal = authenticated_principal(&owner);
    let store = state.session_owner.canonical().store();
    for id in &fence.app_session_ids {
        if store.app_project_id(&principal, id).await.map_err(agent_session_store_error)?.as_deref() != Some(app.id.as_str()) {
            return Err(AppError::Conflict("Application project Session scope changed during deletion".into()).into());
        }
    }
    release_scoped_runtime(state, &fence.app_session_ids, &fence._workspace._root).await?;
    for id in &fence.app_session_ids {
        execute_nomi_core_agent_session_delete_held(state.clone(), owner.clone(), id.clone(),
            format!("app-project-delete:{}:{}", app.id, id.as_ref())).await?;
    }
    Ok(())
}

/// Focused tests exercise the production teardown phase before canonical purge.
#[cfg(test)]
pub(super) async fn release_workspace_runtime(state: &NomiCoreAgentApiState, app: &nomifun_app_hosting::AppRecord)
    -> Result<(), NomiCoreApiError> {
    let owner = AuthenticatedOwner(UserId::from(state.authoritative_user_id.as_ref().to_owned()));
    let principal = authenticated_principal(&owner);
    let root = canonical_existing_workspace_directory(&app.directory, WorkspaceDirectoryCheck::Create)?;
    let workspace = root.to_str().ok_or_else(|| AppError::Conflict("Application project directory is not UTF-8".into()))?;
    let store = state.session_owner.canonical().store();
    let owned = store.list_app_project_session_members(&principal, &app.id).await.map_err(agent_session_store_error)?;
    // Keep the existing directory/effect proof even though teardown selection
    // now comes from immutable application ownership.
    project_sessions(store, &principal, workspace).await?;
    release_scoped_runtime(state, &owned, &root).await
}

async fn release_scoped_runtime(state: &NomiCoreAgentApiState, owned: &[AgentSessionId], root: &std::path::Path)
    -> Result<(), NomiCoreApiError> {
    // Tear down only scoped hosts before releasing the shared workspace. An
    // ordinary same-root host remains an explicit blocker and is preserved.
    for id in owned {
        if state.session_owner.runtime_sessions.active_turn_generation(id.as_ref()).is_some() { return Err(busy()); }
    }
    for id in owned {
        state.session_owner.runtime_sessions.terminate_and_wait_result(id.as_ref(), None).await
            .map_err(|error| NomiCoreApiError::new(StatusCode::CONFLICT, "APP_DEVELOPMENT_RESOURCE_CLEANUP_FAILED",
                format!("Application project Runtime resources could not be released; files and history are preserved: {error}")))?;
    }
    release_workspace_host(state, root)
}

fn release_workspace_host(state: &NomiCoreAgentApiState, root: &std::path::Path) -> Result<(), NomiCoreApiError> {
    let engines = state.session_owner.native_engines.get().and_then(std::sync::Weak::upgrade)
        .ok_or_else(|| NomiCoreApiError::new(StatusCode::CONFLICT, "APP_DEVELOPMENT_RESOURCE_CLEANUP_FAILED",
            "Application workspace resource owner is unavailable"))?;
    engines.release_workspace_host_after_project_teardown(&root)
        .map_err(|error| NomiCoreApiError::new(StatusCode::CONFLICT, "APP_DEVELOPMENT_RESOURCE_CLEANUP_FAILED",
            format!("Application project resources could not be released; files and history are preserved: {error}")))
}
