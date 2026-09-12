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
};
use nomifun_ai_agent::ContextContributor;
use nomifun_api_types::TypedResourceBindingDto;
use nomifun_common::AppError;
use nomifun_file::{
    AgentSessionWorkspaceBinding, WORKSPACE_DELETE_OPERATION,
    WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER,
    WORKSPACE_WRITE_OPERATION,
};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::json;

use super::agent_wave2_host::Wave2ApplicationHost;

pub(crate) const FS_DELETE: &str = "fs.delete";
pub(crate) const FS_WATCH: &str = "fs.watch";
pub(crate) const FS_SNAPSHOT: &str = "fs.snapshot";
pub(crate) const VCS_PUSH: &str = "vcs.push";

const MAX_WATCH_EVENTS: usize = 256;
const MAX_DEBOUNCE_IDENTITIES: usize = 1024;
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

pub(crate) fn tool_capability_ids() -> BTreeSet<CapabilityId> {
    [FS_DELETE, FS_SNAPSHOT, VCS_PUSH]
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

pub(crate) fn event_capability_ids() -> BTreeSet<CapabilityId> {
    BTreeSet::from([CapabilityId::from(FS_WATCH)])
}

pub(crate) fn action_host_port() -> Arc<dyn Wave2HostPort> {
    Arc::new(NomiCoreWave2Host::default())
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

fn canonical_workspace_root(root: &Path) -> Result<PathBuf, AppError> {
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
    roots: Mutex<HashMap<PathBuf, Arc<Wave2ApplicationHost>>>,
}

impl NomiCoreWave2Host {
    fn host_for(
        &self,
        context: &Wave2HostContext,
    ) -> Result<Arc<Wave2ApplicationHost>, Wave2HostPortError> {
        if !matches!(
            context.capability_id.as_ref(),
            FS_DELETE | FS_SNAPSHOT | VCS_PUSH
        ) {
            return Err(Wave2HostPortError::unavailable(format!(
                "{} is not owned by the Nomi-core Wave 2 workspace adapter",
                context.capability_id.as_ref()
            )));
        }
        let canonical_root = exact_session_workspace_root(
            &context.agent_session_id,
            &context.principal.principal_id,
            &context.resource_bindings,
        )?;
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
        let owner = Arc::new(Wave2ApplicationHost::for_workspace_root(
            canonical_root.clone(),
        ));
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
            let owner = dispatch_validated_action_input(
                request.context.capability_id.as_ref(),
                input,
                || self.host_for(&request.context),
            )??;
            owner.invoke(request).await
        })
    }
}

fn dispatch_validated_action_input<T>(
    capability_id: &str,
    input: &StrictJsonValue,
    dispatch: impl FnOnce() -> T,
) -> Result<T, Wave2HostPortError> {
    nomifun_agent_domain_wave2::validate_action_input(capability_id, input)
        .map_err(|error| Wave2HostPortError::new("INVALID_PAYLOAD", error))?;
    Ok(dispatch())
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct WorkspaceWatchEvent {
    path: String,
    operation: &'static str,
}

#[derive(Default)]
struct WatchQueue {
    events: VecDeque<WorkspaceWatchEvent>,
    debounce: HashMap<String, Instant>,
    dropped: u64,
}

impl WatchQueue {
    fn push(&mut self, event: WorkspaceWatchEvent) {
        let now = Instant::now();
        let debounce_key = format!("{}\0{}", event.operation, event.path);
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
        self.debounce.insert(debounce_key, now);
        if self.events.len() == MAX_WATCH_EVENTS {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }

    fn drain(&mut self) -> (Vec<WorkspaceWatchEvent>, u64) {
        let events = self.events.drain(..).collect();
        let dropped = std::mem::take(&mut self.dropped);
        (events, dropped)
    }
}

/// Live `fs.watch` EventSource projected into Nomi's per-turn system context.
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
                let Some(operation) = watch_operation(&event.kind) else {
                    return;
                };
                let mut queue = callback_queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for path in event.paths {
                    let Ok(relative) = path.strip_prefix(&callback_root) else {
                        continue;
                    };
                    let path = relative.to_string_lossy().replace('\\', "/");
                    queue.push(WorkspaceWatchEvent { path, operation });
                }
            },
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Nomi fs.watch could not create a workspace watcher: {error}"
            ))
        })?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| {
                AppError::Internal(format!(
                    "Nomi fs.watch could not subscribe to the Session workspace: {error}"
                ))
            })?;
        Ok(Arc::new(Self {
            root,
            watcher: Mutex::new(watcher),
            queue,
        }))
    }

    #[cfg(test)]
    fn push_for_test(&self, path: &str, operation: &'static str) {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(WorkspaceWatchEvent {
                path: path.to_owned(),
                operation,
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
        serde_json::to_string(&json!({
            "capability_id": FS_WATCH,
            "events": events,
            "dropped_event_count": dropped,
        }))
        .ok()
        .map(|payload| {
            format!(
                "<nomifun_workspace_events format=\"canonical-json\">\n{payload}\n</nomifun_workspace_events>"
            )
        })
    }

    fn label(&self) -> &str {
        FS_WATCH
    }
}

fn watch_operation(kind: &EventKind) -> Option<&'static str> {
    match kind {
        EventKind::Create(_) => Some("create"),
        EventKind::Modify(_) => Some("modify"),
        EventKind::Remove(_) => Some("remove"),
        EventKind::Any | EventKind::Other => Some("change"),
        EventKind::Access(_) => None,
    }
}

/// Exact Wave 2 input schema source for the three admitted FunctionTools.
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
            watch.push_for_test(&format!("src/{index}.rs"), "modify");
        }
        let context = watch.pre_turn_context().await.unwrap();
        assert!(!context.contains(&root.path().to_string_lossy().into_owned()));
        let json = context
            .lines()
            .nth(1)
            .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .unwrap();
        assert_eq!(json["events"].as_array().unwrap().len(), MAX_WATCH_EVENTS);
        assert_eq!(json["dropped_event_count"], 1);
        assert!(watch.pre_turn_context().await.is_none());
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
        for (capability_id, input) in [
            (FS_DELETE, json!({"path": "ok", "workspace_root": "C:/spoof"})),
            (FS_SNAPSHOT, json!({"operation": "baseline"})),
            (
                VCS_PUSH,
                json!({
                    "remote": "origin",
                    "refspec": "HEAD:refs/heads/main",
                    "force": true
                }),
            ),
        ] {
            let error = dispatch_validated_action_input(
                capability_id,
                &StrictJsonValue(input),
                || dispatches.fetch_add(1, Ordering::AcqRel),
            )
            .expect_err("invalid model payload must be rejected before dispatch");
            assert_eq!(error.code, "INVALID_PAYLOAD");
        }
        assert_eq!(dispatches.load(Ordering::Acquire), 0);
    }
}
