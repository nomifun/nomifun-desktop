//! Browser Module host for the unified Agent Runtime.
//!
//! The Kernel freezes Action, Role Provider and Resource authority. This owner
//! binds that exact authority to the application Browser Resource and retains
//! native input only while an interactive Browser invocation and its cleanup run.
//! It is deliberately not a Tool
//! facade: model names and schemas stay owned by the Kernel/Runtime compiler.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::FutureExt;

use nomifun_agent_contracts::{
    AgentSessionId, CapabilityId, ExactRoleProviderRef, OperationId, PrincipalRef,
    ResolvedSnapshotRef, StrictJsonValue, TypedResourceBinding,
};
use nomifun_agent_domain_wave2::{
    Wave2HostPort, Wave2HostPortError, Wave2HostRequest, Wave2TypedCapabilityOperation,
    Wave2TypedHostRequest,
};
use nomifun_agent_kernel::CompiledSnapshot;
use nomifun_browser_platform::{
    attached_browser::{
        AttachedBrowserCommand, AttachedBrowserRuntimeError, AuthorizedAttachedBrowserTurn,
    },
    bound_resource::BoundBrowserProviderResource,
    downloads::BrowserDownloadScope,
    product::{
        BROWSER_MODULE_ID, BROWSER_RESOURCE_KIND, BrowserCapabilityAction,
        BrowserProviderKind, BrowserSessionAuthority,
    },
    run_guard::{BrowserRunGuard, RunAdmissionError},
    runtime::{
        BrowserAction, BrowserDialogReply, BrowserElementRef, BrowserEvaluation,
        BrowserMouseButton, BrowserRuntimeSnapshot, BrowserTabCommand, BrowserTabTarget,
        WorkspaceError,
    },
    uploads::BrowserUploadScope,
    workspace::{BrowserResource, BrowserResourceService, BrowserWorkspace},
};
use nomifun_common::AppError;
use serde::Deserialize;

use crate::{
    AttachedChromeProviderService,
    browser_workspace_provider,
    headless_render::HeadlessRenderRuntime,
};

use super::agent_wave2_host::{
    Wave2EffectAdmission, begin_wave2_exclusive_effect, finish_wave2_failed_effect,
    finish_wave2_succeeded_effect, finish_wave2_uncertain_effect,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TurnKey {
    principal_id: String,
    agent_session_id: String,
    turn_id: String,
}

impl TurnKey {
    fn new(principal_id: &str, agent_session_id: &str, turn_id: &str) -> Self {
        Self {
            principal_id: principal_id.to_owned(),
            agent_session_id: agent_session_id.to_owned(),
            turn_id: turn_id.to_owned(),
        }
    }
}

#[derive(Clone)]
enum ActiveBrowserRun {
    Managed {
        resource: Arc<BrowserResource>,
        guard: BrowserRunGuard,
    },
    AttachedChrome(Arc<AuthorizedAttachedBrowserTurn>),
}

impl ActiveBrowserRun {
    fn cancel(&self) {
        match self {
            Self::Managed { guard, .. } => guard.cancel(),
            Self::AttachedChrome(turn) => turn.cancel(),
        }
    }
}

struct BrowserTurnAdmission {
    key: TurnKey,
    principal: PrincipalRef,
    snapshot: ResolvedSnapshotRef,
    registry_generation: u64,
    provider: ExactRoleProviderRef,
    binding: TypedResourceBinding,
    authority: BrowserSessionAuthority,
    upload_scope: Option<Arc<BrowserUploadScope>>,
    download_scope: Option<Arc<BrowserDownloadScope>>,
    run: Mutex<Option<ActiveBrowserRun>>,
    operation: tokio::sync::Mutex<()>,
    attached_target: Mutex<Option<String>>,
    attached_observations: Mutex<BTreeMap<(String, u64), String>>,
    observations_started: AtomicBool,
    cancelled: AtomicBool,
}

impl BrowserTurnAdmission {
    fn validate(&self, request: &Wave2TypedHostRequest) -> Result<(), BrowserHostFailure> {
        let context = &request.context;
        if context.capability_id.as_ref() != BROWSER_MODULE_ID
            || context.principal != self.principal
            || context.agent_session_id.as_ref() != self.key.agent_session_id
            || context.turn_id.as_ref() != self.key.turn_id
            || context.resolved_snapshot_ref != self.snapshot
            || context.registry_generation != self.registry_generation
        {
            return Err(BrowserHostFailure::AuthorityChanged);
        }
        if context.role_provider.as_ref() != Some(&self.provider) {
            return Err(BrowserHostFailure::ProviderChanged);
        }
        if context.resource_bindings.as_slice() != [self.binding.clone()] {
            return Err(BrowserHostFailure::ResourceChanged);
        }
        let action = browser_action(&request.operation)?;
        if context.action_id.as_ref() != action.action_id() {
            return Err(BrowserHostFailure::AuthorityChanged);
        }
        self.authority.authorize(action)?;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserHostFailure::Cancelled);
        }
        Ok(())
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(run) = self.run.lock().unwrap_or_else(|error| error.into_inner()).as_ref() {
            run.cancel();
        }
    }

    fn active_run(&self) -> Result<Option<ActiveBrowserRun>, BrowserHostFailure> {
        self.run.lock().map(|run| run.clone())
            .map_err(|_| BrowserHostFailure::Unavailable("Browser run state is poisoned".into()))
    }
}

struct ManagedSessionTurn {
    // The retained worker owns input-gate initialization even if its caller exits.
    ready: futures_util::future::Shared<futures_util::future::BoxFuture<'static, Result<BrowserRunGuard, WorkspaceError>>>,
    cancelled: tokio_util::sync::CancellationToken,
    workspace: Arc<BrowserWorkspace>,
    settled: AtomicBool,
}

/// Shared owner mounted both into the Browser Role handler and into the
/// EngineKernelSession lifecycle. The latter is what proves native input is
/// settled before the turn cleanup witness is published.
#[derive(Clone)]
pub(crate) struct BrowserRoleOwner {
    resources: Option<Arc<BrowserResourceService>>,
    attached_chrome: Option<Arc<AttachedChromeProviderService>>,
    data_dir: PathBuf,
    headless_render: Option<Arc<HeadlessRenderRuntime>>,
    effect_store: nomifun_agent_session::AgentSessionStore,
    turns: Arc<Mutex<BTreeMap<TurnKey, Arc<BrowserTurnAdmission>>>>,
    managed_turns: Arc<Mutex<BTreeMap<TurnKey, Arc<ManagedSessionTurn>>>>,
}

impl BrowserRoleOwner {
    pub(crate) fn new(
        resources: Option<Arc<BrowserResourceService>>,
        attached_chrome: Option<Arc<AttachedChromeProviderService>>,
        data_dir: PathBuf,
        headless_render: Option<Arc<HeadlessRenderRuntime>>,
        effect_store: nomifun_agent_session::AgentSessionStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            resources,
            attached_chrome,
            data_dir,
            headless_render,
            effect_store,
            turns: Arc::new(Mutex::new(BTreeMap::new())),
            managed_turns: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    pub(crate) fn ensure_previous_turn_finished(&self, principal_id: &str, agent_session_id: &str) -> Result<(), AppError> {
        if self.managed_turns.lock().map_err(|_| browser_lifecycle_error("managed Browser turn registry is poisoned"))?.keys()
            .chain(self.turns.lock().map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?.keys())
            .any(|key| key.principal_id == principal_id && key.agent_session_id == agent_session_id) {
            return Err(browser_lifecycle_error("A previous Browser turn has not proven terminal release"));
        }
        Ok(())
    }

    async fn begin_managed_turn(&self, key: &TurnKey, workspace: Arc<BrowserWorkspace>, preserve_observations: bool) -> Result<(), AppError> {
        let cancelled = tokio_util::sync::CancellationToken::new();
        let worker_cancelled = cancelled.clone();
        let worker_workspace = workspace.clone();
        let use_owner = {
            let mut uses = self.managed_turns.lock().map_err(|_| browser_lifecycle_error("managed Browser use registry is poisoned"))?;
            if uses.keys().any(|active| active.principal_id == key.principal_id && active.agent_session_id == key.agent_session_id) {
                return Err(browser_lifecycle_error("A previous managed Browser use has not proven cleanup"));
            }
            let worker_uses = self.managed_turns.clone();
            let worker_key = key.clone();
            let worker = tokio::spawn(async move {
                let result = if preserve_observations {
                    worker_workspace.begin_interaction().await
                } else {
                    worker_workspace.begin_run().await
                };
                if matches!(result, Err(WorkspaceError::WorkspaceClosed)) {
                    // The parent holds this registry through insertion. Even a
                    // dropped caller cannot leave a known-closed generation
                    // pinned after the retained preparation worker exits.
                    let mut uses = worker_uses.lock().map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?;
                    if uses.get(&worker_key).is_some_and(|current| Arc::ptr_eq(&current.workspace, &worker_workspace)) { uses.remove(&worker_key); }
                }
                let guard = result?;
                guard.require_explicit_finish();
                if worker_cancelled.is_cancelled() { guard.cancel(); }
                Ok(guard)
            });
            let use_owner = Arc::new(ManagedSessionTurn {
                ready: async move { worker.await.map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))? }.boxed().shared(),
                cancelled,
                workspace,
                settled: AtomicBool::new(false),
            });
            uses.insert(key.clone(), use_owner.clone());
            use_owner
        };
        use_owner.ready.clone().await.map_err(browser_lifecycle_error)?;
        Ok(())
    }

    pub(crate) async fn open_turn(
        &self,
        principal: &PrincipalRef,
        agent_session_id: &AgentSessionId,
        turn_id: &OperationId,
        workspace: &str,
        compiled: &CompiledSnapshot,
    ) -> Result<(), AppError> {
        let capability_id = CapabilityId::from(BROWSER_MODULE_ID);
        let Some(capability) = compiled.resolved_capability(&capability_id) else {
            return Ok(());
        };
        if !compiled
            .capability_resources_bound(&capability_id)
            .map_err(|error| browser_lifecycle_error(error.to_string()))?
        {
            // Browser is an enhancement surface. Keep its frozen capability
            // grant, but do not open lifecycle authority until this Session has
            // one concrete, server-resolved Browser resource.
            return Ok(());
        }
        let policy = compiled.policy(&capability_id).ok_or_else(|| {
            browser_lifecycle_error("Browser Module has no compiled authority policy")
        })?;
        if policy.allowed_actions.is_empty() {
            return Ok(());
        }
        if capability.action_allowlist != policy.allowed_actions {
            return Err(browser_lifecycle_error(
                "Browser Module Action authority differs from the frozen Snapshot",
            ));
        }
        let provider = compiled
            .content()
            .resolved_role_providers
            .get(&nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID.into())
            .map(|lock| lock.provider.clone())
            .ok_or_else(|| browser_lifecycle_error("Browser Module has no exact Role Provider"))?;
        let bindings = policy
            .resource_binding_ids
            .iter()
            .filter_map(|binding_id| compiled.binding(binding_id))
            .filter(|binding| binding.resource_kind.as_ref() == BROWSER_RESOURCE_KIND)
            .cloned()
            .collect::<Vec<_>>();
        let [binding] = bindings.as_slice() else {
            return Err(browser_lifecycle_error(
                "Browser Module requires exactly one frozen Browser Resource",
            ));
        };
        let descriptor = browser_workspace_provider::provider_descriptor(&provider, binding)?;
        let resource = browser_workspace_provider::browser_resource_binding(
            binding.clone(),
            descriptor,
        )?;
        let authority = BrowserSessionAuthority::from_action_ids(
            &principal.principal_id,
            agent_session_id.as_ref(),
            policy.allowed_actions.iter().map(|action| action.as_ref()),
            resource,
        )
        .map_err(browser_lifecycle_error)?;
        let workspace = Path::new(workspace);
        let upload_scope = policy
            .allowed_actions
            .iter()
            .any(|action| action.as_ref() == "browser/upload")
            .then(|| BrowserUploadScope::open(workspace).map(Arc::new))
            .transpose()
            .map_err(browser_lifecycle_error)?;
        let download_scope = policy
            .allowed_actions
            .iter()
            .any(|action| action.as_ref() == "browser/download")
            .then(|| BrowserDownloadScope::open(workspace).map(Arc::new))
            .transpose()
            .map_err(browser_lifecycle_error)?;
        let key = TurnKey::new(
            &principal.principal_id,
            agent_session_id.as_ref(),
            turn_id.as_ref(),
        );
        let admission = Arc::new(BrowserTurnAdmission {
            key: key.clone(),
            principal: principal.clone(),
            snapshot: compiled.snapshot_ref().clone(),
            registry_generation: compiled.registry_generation,
            provider,
            binding: binding.clone(),
            authority,
            upload_scope,
            download_scope,
            run: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            attached_target: Mutex::new(None),
            attached_observations: Mutex::new(BTreeMap::new()),
            observations_started: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        });
        let mut turns = self
            .turns
            .lock()
            .map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?;
        if turns.keys().any(|active| {
            active.principal_id == key.principal_id
                && active.agent_session_id == key.agent_session_id
        }) {
            return Err(browser_lifecycle_error(
                "A previous Browser turn has not proven cleanup",
            ));
        }
        turns.insert(key, admission);
        Ok(())
    }

    pub(crate) fn cancel_turn(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        turn_id: &str,
    ) -> Result<(), AppError> {
        let key = TurnKey::new(principal_id, agent_session_id, turn_id);
        if let Some(turn) = self.managed_turns.lock().map_err(|_| browser_lifecycle_error("managed Browser turn registry is poisoned"))?.get(&key) {
            turn.cancelled.cancel();
            if let Some(Ok(guard)) = turn.ready.peek() { guard.cancel(); }
        }

        if let Some(admission) = self
            .turns
            .lock()
            .map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?
            .get(&key)
            .cloned()
        {
            admission.cancel();
        }
        Ok(())
    }

    pub(crate) async fn settle_turn(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        turn_id: &str,
    ) -> Result<(), AppError> {
        let key = TurnKey::new(principal_id, agent_session_id, turn_id);
        self.cancel_turn(principal_id, agent_session_id, turn_id)?;
        let admission = self
            .turns
            .lock()
            .map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?
            .get(&key)
            .cloned();
        let _operation = match &admission {
            Some(admission) => Some(admission.operation.lock().await),
            None => None,
        };
        if let Some(admission) = &admission {
            if let Some(ActiveBrowserRun::AttachedChrome(run)) = admission.active_run().map_err(browser_lifecycle_error)? {
                run.settle().await.map_err(browser_lifecycle_error)?;
            }
        }
        let managed = self.managed_turns.lock()
            .map_err(|_| browser_lifecycle_error("managed Browser turn registry is poisoned"))?
            .get(&key).cloned();
        if let Some(turn) = managed {
            turn.cancelled.cancel();
            match turn.ready.clone().await {
                Ok(guard) => {
                    guard.cancel();
                    turn.workspace.settle_run(&guard).await.map_err(browser_lifecycle_error)?;
                }
                Err(_) => turn.workspace.settle_failed_start().await.map_err(browser_lifecycle_error)?,
            }
            turn.settled.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// A parked model gives the human the Browser while the same Turn retains
    /// its other owners. Settlement precedes unlock, and old guards/observations
    /// are retired. A later invocation needs a newly opened lazy admission.
    pub(crate) async fn yield_to_user(&self, principal_id: &str, agent_session_id: &str, turn_id: &str) -> Result<(), AppError> {
        self.require_parked_turn(principal_id, agent_session_id, turn_id).await?;
        self.settle_turn(principal_id, agent_session_id, turn_id).await?;
        self.require_parked_turn(principal_id, agent_session_id, turn_id).await?;
        self.finish_turn(principal_id, agent_session_id, turn_id).await
    }

    async fn require_parked_turn(&self, principal_id: &str, agent_session_id: &str, turn_id: &str) -> Result<(), AppError> {
        let session=AgentSessionId::from(agent_session_id);
        let observed=self.effect_store.observe(&session,None,1).await.map_err(browser_lifecycle_error)?;
        if observed.session.owner_ref.principal_kind != "user" || observed.session.owner_ref.principal_id != principal_id
            || observed.head.status != "running" || observed.head.active_turn_id.as_deref() != Some(turn_id)
            || !matches!(observed.active_turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Waiting {..})) {
            return Err(browser_lifecycle_error("Browser user handoff requires the exact canonical parked Turn"));
        }
        if self.effect_store.has_unsettled_effects(&session).await.map_err(browser_lifecycle_error)? {
            return Err(browser_lifecycle_error("Browser user handoff cannot release an unsettled effect"));
        }
        Ok(())
    }

    /// Called after the trusted owner commits a terminal or enters an exact
    /// parked boundary. Failed settlement retains the native input lock.
    pub(crate) async fn finish_turn(&self, principal_id: &str, agent_session_id: &str, turn_id: &str) -> Result<(), AppError> {
        let key = TurnKey::new(principal_id, agent_session_id, turn_id);
        let admission = self.turns.lock().map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?.get(&key).cloned();
        if let Some(admission) = &admission {
            if let Some(ActiveBrowserRun::AttachedChrome(run)) = admission.active_run().map_err(browser_lifecycle_error)? {
                run.finish().await.map_err(browser_lifecycle_error)?;
            }
        }
        let managed = self.managed_turns.lock().map_err(|_| browser_lifecycle_error("managed Browser turn registry is poisoned"))?.get(&key).cloned();
        if let Some(turn) = managed {
            if !turn.settled.load(Ordering::Acquire) { return Err(browser_lifecycle_error("managed Browser input has not proven settlement")); }
            match turn.ready.clone().await {
                Ok(guard) => turn.workspace.finish_run(&guard).await.map_err(browser_lifecycle_error)?,
                Err(_) => turn.workspace.recover_failed_start().await.map_err(browser_lifecycle_error)?,
            }
            self.managed_turns.lock().map_err(|_| browser_lifecycle_error("managed Browser turn registry is poisoned"))?.remove(&key);
        }
        if let Some(admission) = admission {
            let mut turns = self.turns.lock().map_err(|_| browser_lifecycle_error("Browser turn registry is poisoned"))?;
            if turns.get(&key).is_some_and(|current| Arc::ptr_eq(current, &admission)) { turns.remove(&key); }
        }
        Ok(())
    }

    fn admission(
        &self,
        request: &Wave2TypedHostRequest,
    ) -> Result<Arc<BrowserTurnAdmission>, BrowserHostFailure> {
        let key = TurnKey::new(
            &request.context.principal.principal_id,
            request.context.agent_session_id.as_ref(),
            request.context.turn_id.as_ref(),
        );
        let admission = self
            .turns
            .lock()
            .map_err(|_| BrowserHostFailure::Unavailable("turn registry poisoned".into()))?
            .get(&key)
            .cloned()
            .ok_or(BrowserHostFailure::NoActiveTurn)?;
        admission.validate(request)?;
        Ok(admission)
    }

    async fn run(&self, admission: &Arc<BrowserTurnAdmission>) -> Result<ActiveBrowserRun, BrowserHostFailure> {
        if admission.cancelled.load(Ordering::Acquire) { return Err(BrowserHostFailure::Cancelled); }
        let run = match admission.active_run()? {
            Some(run) => run,
            None => {
                // Cache only a successfully admitted run. A human close may win
                // before first use; that failed binding must not pin a dead runtime.
                let bound = browser_workspace_provider::bind_authorized_resource(
                    self.resources.clone(), self.attached_chrome.clone(), &self.data_dir,
                    admission.authority.clone(),
                ).await.map_err(|error| BrowserHostFailure::Unavailable(error.to_string()))?;
                let run = match bound {
                    BoundBrowserProviderResource::Managed(resource) => {
                        let current = self.managed_turns.lock()
                            .map_err(|_| BrowserHostFailure::Unavailable("managed Browser turn registry is poisoned".into()))?
                            .get(&admission.key).cloned();
                        if current.is_none() {
                            // A new Turn cannot reuse old element authority. Only
                            // later calls in this Turn preserve valid observations.
                            let preserve = admission.observations_started.load(Ordering::Acquire);
                            self.begin_managed_turn(&admission.key, resource.workspace().clone(), preserve)
                                .await.map_err(|error| BrowserHostFailure::Unavailable(error.to_string()))?;
                            admission.observations_started.store(true, Ordering::Release);
                        }
                        let turn = self.managed_turns.lock()
                            .map_err(|_| BrowserHostFailure::Unavailable("managed Browser turn registry is poisoned".into()))?
                            .get(&admission.key).cloned().ok_or(BrowserHostFailure::NoActiveTurn)?;
                        let guard = turn.ready.clone().await.map_err(BrowserHostFailure::from)?;
                        if !Arc::ptr_eq(resource.workspace(), &turn.workspace) { return Err(BrowserHostFailure::ResourceChanged); }
                        ActiveBrowserRun::Managed { resource, guard }
                    }
                    BoundBrowserProviderResource::AttachedChrome(resource) => ActiveBrowserRun::AttachedChrome(resource.begin_run().await?),
                };
                if admission.cancelled.load(Ordering::Acquire) { run.cancel(); }
                *admission.run.lock().map_err(|_| BrowserHostFailure::Unavailable("Browser run state is poisoned".into()))? = Some(run.clone());
                run
            }
        };
        if admission.cancelled.load(Ordering::Acquire) {
            run.cancel();
            return Err(BrowserHostFailure::Cancelled);
        }
        Ok(run)
    }

    async fn invoke_typed(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        // The invocation owns admission, native callbacks, effect persistence
        // and input release even if the Runtime drops its response future.
        let owner = self.clone();
        tokio::spawn(async move { owner.invoke_owned(request).await }).await
            .map_err(|_| Wave2HostPortError::unavailable("Browser invocation worker failed; cleanup remains owned"))?
    }

    async fn invoke_owned(
        &self,
        request: Wave2TypedHostRequest,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let admission = self.admission(&request).map_err(Wave2HostPortError::from)?;
        if matches!(&request.operation, Wave2TypedCapabilityOperation::BrowserRenderContent { .. }) {
            return self.dispatch(&admission, request.operation).await.map_err(Into::into);
        }
        let _operation = admission.operation.lock().await;
        admission.validate(&request).map_err(Wave2HostPortError::from)?;
        let effect_input = operation_input(&request.operation).clone();
        let action = browser_action(&request.operation).map_err(Wave2HostPortError::from)?;
        let effect_strategy = match action {
            BrowserCapabilityAction::Observe | BrowserCapabilityAction::RenderContent => None,
            BrowserCapabilityAction::Download => {
                Some(nomifun_agent_session::EffectStrategy::ManagedEffect)
            }
            BrowserCapabilityAction::Navigate
            | BrowserCapabilityAction::Act
            | BrowserCapabilityAction::Upload
            | BrowserCapabilityAction::Evaluate => {
                Some(nomifun_agent_session::EffectStrategy::ExternalUncertainEffect)
            }
        };
        let Some(strategy) = effect_strategy else {
            let result = self.dispatch(&admission, request.operation).await.map_err(Wave2HostPortError::from);
            if result.as_ref().err().is_none_or(|error| !browser_outcome_may_be_unknown(&error.code)) {
                self.release_managed_interaction(&admission).await?;
            }
            return result;
        };
        let reservation = match begin_wave2_exclusive_effect(
            &self.effect_store,
            &request.context,
            &admission.binding,
            &effect_input,
            strategy,
        )
        .await?
        {
            Wave2EffectAdmission::Replay(output) => return Ok(output),
            Wave2EffectAdmission::Reserved(reservation) => reservation,
        };
        match self.dispatch(&admission, request.operation).await {
            Ok(output) => {
                finish_wave2_succeeded_effect(
                    &reservation,
                    action.action_id(),
                    &output,
                )
                .await?;
                self.release_managed_interaction(&admission).await?;
                Ok(output)
            }
            Err(error) => {
                let host_error = Wave2HostPortError::from(error);
                if strategy.is_external_uncertain()
                    && browser_outcome_may_be_unknown(&host_error.code)
                {
                    finish_wave2_uncertain_effect(
                        &reservation,
                        action.action_id(),
                        &host_error,
                    ).await?;
                } else {
                    finish_wave2_failed_effect(
                        &reservation,
                        action.action_id(),
                        &host_error,
                    ).await?;
                    self.release_managed_interaction(&admission).await?;
                }
                Err(host_error)
            }
        }
    }

    async fn release_managed_interaction(&self, admission: &Arc<BrowserTurnAdmission>) -> Result<(), Wave2HostPortError> {
        if admission.cancelled.load(Ordering::Acquire) { return Ok(()); }
        let turn = self.managed_turns.lock()
            .map_err(|_| Wave2HostPortError::unavailable("managed Browser use registry is poisoned"))?
            .get(&admission.key).cloned();
        let Some(turn) = turn else { return Ok(()); };
        if self.effect_store.has_unsettled_effects_for_binding(
            &AgentSessionId::from(admission.key.agent_session_id.clone()), &admission.binding.binding_id,
        ).await
            .map_err(|error| Wave2HostPortError::unavailable(error.to_string()))? {
            return Ok(());
        }
        // Failed starts and unknown effects keep their existing recovery owner.
        let Ok(guard) = turn.ready.clone().await else { return Ok(()); };
        if !turn.workspace.finish_interaction(&guard).await.map_err(BrowserHostFailure::from).map_err(Wave2HostPortError::from)? {
            // An awaiting website dialog still owns a native continuation.
            return Ok(());
        }
        let mut turns = self.managed_turns.lock()
            .map_err(|_| Wave2HostPortError::unavailable("managed Browser use registry is poisoned"))?;
        if turns.get(&admission.key).is_some_and(|current| Arc::ptr_eq(current, &turn)) {
            turns.remove(&admission.key);
        }
        *admission.run.lock().map_err(|_| Wave2HostPortError::unavailable("Browser run state is poisoned"))? = None;
        Ok(())
    }

    async fn dispatch(
        &self,
        admission: &Arc<BrowserTurnAdmission>,
        operation: Wave2TypedCapabilityOperation,
    ) -> Result<StrictJsonValue, BrowserHostFailure> {
        match operation {
            Wave2TypedCapabilityOperation::BrowserRenderContent { input } => {
                self.render_content(admission, input).await
            }
            Wave2TypedCapabilityOperation::BrowserObserve { input } => {
                let input: ObserveInput = decode(input)?;
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        // Capture can activate the tab and invalidate previous references.
                        // The final observation must therefore follow capture on both hosts.
                        let screenshot = if input.screenshot {
                            match resource.screenshot(&guard, input.tab_id.clone()).await {
                                Ok(screenshot) => Some(screenshot),
                                Err(WorkspaceError::DialogPending) => None,
                                Err(error) => return Err(error.into()),
                            }
                        } else { None };
                        let observation = resource.observe(&guard, input.tab_id).await?;
                        let capabilities = resource.snapshot().await?.interaction_capabilities;
                        encode_managed_observation(observation, capabilities, screenshot, input.screenshot)
                    }
                    ActiveBrowserRun::AttachedChrome(turn) => {
                        if input.screenshot {
                            return Err(BrowserHostFailure::Unsupported("Attached Chrome does not support visible screenshots through browser/observe. Do not repeat this request; observe without screenshot.".into()));
                        }
                        match input.tab_id {
                            Some(tab_id) => {
                                set_attached_target(admission, &tab_id)?;
                                let output = turn
                                    .invoke(AttachedBrowserCommand::Observe { tab_id })
                                    .await?;
                                canonical_attached_observation(admission, output)
                            }
                            None => {
                                let output = turn.invoke(AttachedBrowserCommand::Tabs {}).await?;
                                remember_single_attached_target(admission, &output)?;
                                Ok(StrictJsonValue(output))
                            }
                        }
                    }
                }
            }
            Wave2TypedCapabilityOperation::BrowserNavigate { input } => {
                let input: NavigateInput = decode(input)?;
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        let current = resource.snapshot().await?;
                        let active = current.runtime.as_ref().and_then(|runtime| {
                            runtime.tabs.iter().find(|tab| {
                                runtime.active_tab_id.as_ref() == Some(&tab.target.tab_id)
                            })
                        });
                        let command = match active {
                            Some(tab) if !input.new_tab => BrowserTabCommand::Navigate {
                                target: tab.target.clone(),
                                url: input.url,
                            },
                            _ => BrowserTabCommand::Create { url: input.url },
                        };
                        let snapshot = resource.agent_command(&guard, command).await?;
                        runtime_output(&admission.authority, snapshot)
                    }
                    ActiveBrowserRun::AttachedChrome(turn) => {
                        if input.new_tab {
                            return Err(BrowserHostFailure::Unsupported(
                                "attached Chrome cannot create a tab".into(),
                            ));
                        }
                        let tab_id = self.attached_target(admission, &turn).await?;
                        forget_attached_observations(admission, &tab_id)?;
                        Ok(StrictJsonValue(
                            turn.invoke(AttachedBrowserCommand::Navigate {
                                tab_id,
                                url: input.url,
                            })
                            .await?,
                        ))
                    }
                }
            }
            Wave2TypedCapabilityOperation::BrowserAct { input } => {
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        match decode_managed_act(input.0)? {
                            ManagedAct::Input(action) => encode(resource.act(&guard, action).await?),
                            ManagedAct::Dialog(dialog) => {
                                encode(resource.respond_dialog(&guard, dialog.into()).await?)
                            }
                        }
                    }
                    ActiveBrowserRun::AttachedChrome(turn) => {
                        let command = canonical_attached_action(admission, input.0)?;
                        set_attached_target(admission, attached_command_tab(&command)?)?;
                        Ok(StrictJsonValue(turn.invoke(command).await?))
                    }
                }
            }
            Wave2TypedCapabilityOperation::BrowserDownload { input } => {
                let input: DownloadInput = decode(input)?;
                let scope = admission
                    .download_scope
                    .clone()
                    .ok_or(BrowserHostFailure::WorkspaceScope("download"))?;
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        encode(
                            resource
                                .download(&guard, input.element.into_reference(), scope)
                                .await?,
                        )
                    }
                    ActiveBrowserRun::AttachedChrome(_) => Err(BrowserHostFailure::Unsupported(
                        "attached Chrome does not implement browser/download".into(),
                    )),
                }
            }
            Wave2TypedCapabilityOperation::BrowserUpload { input } => {
                let input: UploadInput = decode(input)?;
                let scope = admission
                    .upload_scope
                    .clone()
                    .ok_or(BrowserHostFailure::WorkspaceScope("upload"))?;
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        encode(
                            resource
                                .upload(
                                    &guard,
                                    input.element.into_reference(),
                                    scope,
                                    input.files,
                                )
                                .await?,
                        )
                    }
                    ActiveBrowserRun::AttachedChrome(_) => Err(BrowserHostFailure::Unsupported(
                        "attached Chrome does not implement browser/upload".into(),
                    )),
                }
            }
            Wave2TypedCapabilityOperation::BrowserEvaluate { input } => {
                let input: EvaluateInput = decode(input)?;
                match self.run(admission).await? {
                    ActiveBrowserRun::Managed { resource, guard } => {
                        encode(resource.evaluate(&guard, input.request).await?)
                    }
                    ActiveBrowserRun::AttachedChrome(_) => Err(BrowserHostFailure::Unsupported(
                        "attached Chrome does not implement browser/evaluate".into(),
                    )),
                }
            }
            _ => Err(BrowserHostFailure::InvalidInput(
                "operation is not a Browser Action",
            )),
        }
    }

    async fn render_content(
        &self,
        admission: &Arc<BrowserTurnAdmission>,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, BrowserHostFailure> {
        let input: RenderContentInput = decode(input)?;
        admission.authority.authorize(BrowserCapabilityAction::RenderContent)?;
        if admission.authority.resource().provider().kind() != BrowserProviderKind::Managed {
            return Err(BrowserHostFailure::Unsupported(
                "selected Browser Provider cannot render isolated content".into(),
            ));
        }
        let url = url::Url::parse(&input.url)
            .map_err(|_| BrowserHostFailure::InvalidInput("browser render URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(BrowserHostFailure::InvalidInput("browser render URL"));
        }
        let runtime = self
            .headless_render
            .as_ref()
            .ok_or_else(|| BrowserHostFailure::Unavailable("headless renderer unavailable".into()))?;
        let output = runtime
            .render(url)
            .await
            .map_err(|error| BrowserHostFailure::Render(error.to_string()))?;
        Ok(StrictJsonValue(serde_json::json!({
            "final_url": output.final_url,
            "html": output.html,
            "html_truncated": output.html_truncated,
        })))
    }

    async fn attached_target(
        &self,
        admission: &Arc<BrowserTurnAdmission>,
        turn: &Arc<AuthorizedAttachedBrowserTurn>,
    ) -> Result<String, BrowserHostFailure> {
        if let Some(target) = admission
            .attached_target
            .lock()
            .map_err(|_| BrowserHostFailure::Unavailable("attached target state poisoned".into()))?
            .clone()
        {
            return Ok(target);
        }
        let inventory = turn.invoke(AttachedBrowserCommand::Tabs {}).await?;
        remember_single_attached_target(admission, &inventory)?;
        admission
            .attached_target
            .lock()
            .map_err(|_| BrowserHostFailure::Unavailable("attached target state poisoned".into()))?
            .clone()
            .ok_or_else(|| {
                BrowserHostFailure::Unsupported(
                    "observe and select one attached tab before navigation".into(),
                )
            })
    }
}

impl Wave2HostPort for BrowserRoleOwner {
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
            let request = request.into_typed()?;
            self.invoke_typed(request).await
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum BrowserHostFailure {
    #[error("there is no active Browser turn")]
    NoActiveTurn,
    #[error("the Browser turn authority changed")]
    AuthorityChanged,
    #[error("the Browser Provider changed")]
    ProviderChanged,
    #[error("the Browser Resource changed")]
    ResourceChanged,
    #[error("the Browser turn was cancelled")]
    Cancelled,
    #[error("invalid {0}")]
    InvalidInput(&'static str),
    #[error("browser/{0} has no authorized workspace scope")]
    WorkspaceScope(&'static str),
    #[error("unsupported Browser action: {0}")]
    Unsupported(String),
    #[error("Browser owner unavailable: {0}")]
    Unavailable(String),
    #[error("Browser render failed: {0}")]
    Render(String),
    #[error(transparent)]
    Workspace(WorkspaceError),
    #[error(transparent)]
    Attached(AttachedBrowserRuntimeError),
    #[error("Browser result serialization failed")]
    Serialization,
}

impl From<WorkspaceError> for BrowserHostFailure {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}

impl From<AttachedBrowserRuntimeError> for BrowserHostFailure {
    fn from(error: AttachedBrowserRuntimeError) -> Self {
        Self::Attached(error)
    }
}

impl From<BrowserHostFailure> for Wave2HostPortError {
    fn from(error: BrowserHostFailure) -> Self {
        match error {
            BrowserHostFailure::NoActiveTurn => Self::new(
                "BROWSER_STALE_RUN",
                "Browser Action has no admitted Agent turn",
            ),
            BrowserHostFailure::AuthorityChanged => Self::new(
                "BROWSER_AUTHORITY_CHANGED",
                "Browser invocation differs from its frozen Agent turn authority",
            ),
            BrowserHostFailure::ProviderChanged => Self::new(
                "BROWSER_PROVIDER_CHANGED",
                "Browser Role Provider differs from the frozen AgentSession Provider",
            ),
            BrowserHostFailure::ResourceChanged => Self::new(
                "BROWSER_RESOURCE_CHANGED",
                "Browser Resource differs from the frozen AgentSession binding",
            ),
            BrowserHostFailure::Cancelled => Self::new(
                "BROWSER_OPERATION_CANCELLED",
                "Browser turn cleanup has started",
            ),
            BrowserHostFailure::InvalidInput(field) => {
                Self::invalid_payload(format!("invalid {field}"))
            }
            BrowserHostFailure::WorkspaceScope(operation) => Self::resource_not_bound(format!(
                "browser/{operation} requires an authorized absolute workspace root",
            )),
            BrowserHostFailure::Unsupported(message) => {
                Self::new("BROWSER_UNSUPPORTED_ACTION", message)
            }
            BrowserHostFailure::Unavailable(message) => Self::unavailable(message),
            BrowserHostFailure::Render(message) => {
                Self::new("BROWSER_RENDER_FAILED", message)
            }
            BrowserHostFailure::Workspace(error) => Self::new(error.code(), error.to_string()),
            BrowserHostFailure::Attached(error) => {
                let code = match error {
                    AttachedBrowserRuntimeError::Unavailable => "BROWSER_PROVIDER_UNAVAILABLE",
                    AttachedBrowserRuntimeError::StaleRun => "BROWSER_STALE_RUN",
                    AttachedBrowserRuntimeError::Busy => "BROWSER_RUN_BUSY",
                    AttachedBrowserRuntimeError::Disconnected => {
                        "BROWSER_PROVIDER_DISCONNECTED"
                    }
                    AttachedBrowserRuntimeError::TabDenied => "BROWSER_TAB_DENIED",
                    AttachedBrowserRuntimeError::ActionDenied => "BROWSER_ACTION_DENIED",
                    AttachedBrowserRuntimeError::InvalidInput => "INVALID_PAYLOAD",
                    AttachedBrowserRuntimeError::ExecutionFailed => "BROWSER_EXECUTION_FAILED",
                    AttachedBrowserRuntimeError::Cancelled => "BROWSER_OPERATION_CANCELLED",
                };
                Self::new(code, error.to_string())
            }
            BrowserHostFailure::Serialization => Self::unavailable(
                "Browser owner could not encode its bounded result",
            ),
        }
    }
}

fn browser_lifecycle_error(error: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Browser Resource lifecycle: {error}"))
}

#[async_trait::async_trait]
impl crate::browser_workspace_provider::BrowserUserClosePort for BrowserRoleOwner {
    async fn close_user_workspace(&self, principal_id: &str, agent_session_id: &str, runtime_generation: u64) -> Result<(), WorkspaceError> {
        let session_id = AgentSessionId::from(agent_session_id.to_owned());
        let session = self.effect_store.get_live_session(&session_id).await.map_err(|_| WorkspaceError::ActionDenied)?;
        if session.owner_ref.principal_kind != "user" || session.owner_ref.principal_id != principal_id { return Err(WorkspaceError::ActionDenied); }
        let admission = self.turns.lock().map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
            .iter().find(|(key, turn)| key.principal_id == principal_id && key.agent_session_id == agent_session_id
                && turn.authority.resource().provider().kind() == BrowserProviderKind::Managed)
            .map(|(_, turn)| turn.clone());
        // Destruction and the first browser invocation share one admission
        // boundary, including a native start which fails before issuing a guard.
        // A click made during browser work is rejected, never queued for replay.
        let _operation = match &admission {
            Some(admission) => Some(admission.operation.try_lock().map_err(|_| WorkspaceError::Admission(RunAdmissionError::UserInputLocked))?),
            None => None,
        };
        let head = self.effect_store.head(&session_id).await.map_err(|_| WorkspaceError::ActionDenied)?;
        let resources = self.resources.as_ref().ok_or(WorkspaceError::NativeUnavailable)?;
        let Some(current) = resources.get_for_agent_session(principal_id, agent_session_id).await? else { return Ok(()); };
        if current.runtime_generation() != runtime_generation { return Err(WorkspaceError::StaleTarget); }
        // Failed native admission still belongs to this browser Turn even
        // when no guard was issued. An unrelated running Turn has no owner
        // here and must not prevent an idle user browser from closing.
        let turns = self.managed_turns.lock().map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
            .iter().filter(|(key, turn)| key.principal_id == principal_id && key.agent_session_id == agent_session_id
                && Arc::ptr_eq(&current, &turn.workspace))
            .map(|(key, turn)| (key.clone(), turn.clone())).collect::<Vec<_>>();
        if !turns.is_empty() {
            if head.status == "running" || head.active_turn_id.is_some() { return Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)); }
            let [(key, turn)] = turns.as_slice() else { return Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)); };
            if !turn.settled.load(Ordering::Acquire) { return Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked)); }
            let receipt = self.effect_store.read_turn_receipt(&session_id, &OperationId::from(key.turn_id.clone()))
                .await.map_err(|_| WorkspaceError::ActionDenied)?;
            let terminal = receipt.terminal_event.ok_or(WorkspaceError::ActionDenied)?;
            if receipt.status == nomifun_agent_session::TurnReceiptStatus::Running
                || terminal.agent_session_id != session_id || terminal.correlation_id.as_ref() != key.turn_id
                || !matches!(terminal.kind.0.as_str(), "turn/completed" | "turn/failed" | "turn/cancelled") {
                return Err(WorkspaceError::ActionDenied);
            }
            resources.close_after_terminal(current.key().clone(), runtime_generation).await
        } else {
            resources.close_idle(current.key().clone(), runtime_generation).await
        }
    }
}

fn browser_action(
    operation: &Wave2TypedCapabilityOperation,
) -> Result<BrowserCapabilityAction, BrowserHostFailure> {
    match operation {
        Wave2TypedCapabilityOperation::BrowserObserve { .. } => {
            Ok(BrowserCapabilityAction::Observe)
        }
        Wave2TypedCapabilityOperation::BrowserNavigate { .. } => {
            Ok(BrowserCapabilityAction::Navigate)
        }
        Wave2TypedCapabilityOperation::BrowserAct { .. } => Ok(BrowserCapabilityAction::Act),
        Wave2TypedCapabilityOperation::BrowserRenderContent { .. } => {
            Ok(BrowserCapabilityAction::RenderContent)
        }
        Wave2TypedCapabilityOperation::BrowserDownload { .. } => {
            Ok(BrowserCapabilityAction::Download)
        }
        Wave2TypedCapabilityOperation::BrowserUpload { .. } => {
            Ok(BrowserCapabilityAction::Upload)
        }
        Wave2TypedCapabilityOperation::BrowserEvaluate { .. } => {
            Ok(BrowserCapabilityAction::Evaluate)
        }
        _ => Err(BrowserHostFailure::InvalidInput(
            "operation is not a Browser Action",
        )),
    }
}

fn operation_input(operation: &Wave2TypedCapabilityOperation) -> &StrictJsonValue {
    match operation {
        Wave2TypedCapabilityOperation::BrowserObserve { input }
        | Wave2TypedCapabilityOperation::BrowserNavigate { input }
        | Wave2TypedCapabilityOperation::BrowserAct { input }
        | Wave2TypedCapabilityOperation::BrowserRenderContent { input }
        | Wave2TypedCapabilityOperation::BrowserDownload { input }
        | Wave2TypedCapabilityOperation::BrowserUpload { input }
        | Wave2TypedCapabilityOperation::BrowserEvaluate { input } => input,
        _ => unreachable!("Browser owner accepts only Browser operations"),
    }
}

fn decode<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, BrowserHostFailure> {
    serde_json::from_value(input.0)
        .map_err(|_| BrowserHostFailure::InvalidInput("Browser Action payload"))
}

fn encode(value: impl serde::Serialize) -> Result<StrictJsonValue, BrowserHostFailure> {
    serde_json::to_value(value)
        .map(StrictJsonValue)
        .map_err(|_| BrowserHostFailure::Serialization)
}

pub(super) fn encode_managed_observation(
    mut observation: nomifun_browser_platform::runtime::BrowserObservation,
    capabilities: Option<nomifun_browser_platform::runtime::BrowserInteractionCapabilities>,
    screenshot: Option<nomifun_browser_platform::runtime::BrowserScreenshot>,
    screenshot_requested: bool,
) -> Result<StrictJsonValue, BrowserHostFailure> {
    if screenshot.as_ref().is_some_and(|screenshot| screenshot.target != observation.target) {
        return Err(WorkspaceError::StaleTarget.into());
    }
    // A dialog can open between capture and observation. Do not show pixels as
    // the current operable page in that case; retain only the exact dialog.
    let screenshot = if observation.script_dialog.is_some() { None } else { screenshot };
    if screenshot_requested && screenshot.is_none() && observation.script_dialog.is_none() {
        return Err(WorkspaceError::DialogPending.into());
    }
    observation.load = observation.load.as_ref().map(|load| load.agent_projection());
    observation.content_url = observation.content_url.as_deref()
        .map(nomifun_browser_platform::url_projection::project_metadata_url);
    let mut output = encode(observation)?;
    let object = output.0.as_object_mut().ok_or(BrowserHostFailure::Serialization)?;
    if let Some(capabilities) = capabilities {
        object.insert("interaction_capabilities".into(), serde_json::to_value(capabilities)
                .map_err(|_| BrowserHostFailure::Serialization)?);
    }
    if let Some(screenshot) = screenshot {
        object.insert("screenshot".into(), serde_json::json!({
            "target": screenshot.target, "width": screenshot.width, "height": screenshot.height,
            "viewport_width": screenshot.viewport_width, "viewport_height": screenshot.viewport_height,
            "png_base64": screenshot.png_base64,
        }));
    } else if screenshot_requested {
        object.insert("screenshot_status".into(), serde_json::json!("awaiting_dialog"));
    }
    Ok(output)
}

fn runtime_output(
    authority: &BrowserSessionAuthority,
    mut snapshot: BrowserRuntimeSnapshot,
) -> Result<StrictJsonValue, BrowserHostFailure> {
    if authority.authorize(BrowserCapabilityAction::Observe).is_ok() {
        for tab in &mut snapshot.tabs {
            tab.url = nomifun_browser_platform::url_projection::project_metadata_url(&tab.url);
            tab.load = tab.load.as_ref().map(|load| load.agent_projection());
        }
        return encode(snapshot);
    }
    let target = snapshot
        .tabs
        .iter()
        .find(|tab| snapshot.active_tab_id.as_ref() == Some(&tab.target.tab_id))
        .map(|tab| tab.target.clone());
    Ok(StrictJsonValue(serde_json::json!({
        "runtime_generation": snapshot.runtime_generation,
        "active_tab_id": snapshot.active_tab_id,
        "target": target,
    })))
}

fn browser_outcome_may_be_unknown(code: &str) -> bool {
    !matches!(
        code,
        "INVALID_PAYLOAD"
            | "CAPABILITY_UNAVAILABLE"
            | "CAPABILITY_UNAVAILABLE_ON_PLATFORM"
            | "PRESET_RESOURCE_NOT_BOUND"
            | "BROWSER_ACTION_DENIED"
            | "BROWSER_AUTHORITY_CHANGED"
            | "BROWSER_PROVIDER_CHANGED"
            | "BROWSER_PROVIDER_UNAVAILABLE"
            | "BROWSER_RESOURCE_CHANGED"
            | "BROWSER_RESOURCE_CLOSED"
            | "BROWSER_NATIVE_SURFACE_UNAVAILABLE"
            | "BROWSER_STALE_RUN"
            | "BROWSER_RUN_BUSY"
            | "BROWSER_INPUT_GATE_FAILED"
            | "BROWSER_USER_INPUT_LOCKED"
            | "BROWSER_STALE_OBSERVATION"
            | "BROWSER_STALE_TARGET"
            | "BROWSER_TAB_NOT_FOUND"
            | "BROWSER_TAB_DENIED"
            | "BROWSER_TAB_LIMIT"
            | "BROWSER_INVALID_URL"
            | "BROWSER_NOT_ACTIONABLE"
            | "BROWSER_DIALOG_PENDING"
            | "BROWSER_UNSUPPORTED_ACTION"
            | "BROWSER_UPLOAD_PATH_DENIED"
            | "BROWSER_UPLOAD_SOURCE_CHANGED"
            | "BROWSER_UPLOAD_SOURCE_NAME_UNSUPPORTED"
            | "BROWSER_UPLOAD_LIMIT"
            | "BROWSER_DOWNLOAD_LIMIT"
            | "BROWSER_DOWNLOAD_DENIED"
            | "BROWSER_OPERATION_CANCELLED"
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObserveInput {
    tab_id: Option<String>,
    #[serde(default)]
    screenshot: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigateInput {
    url: String,
    #[serde(default)]
    new_tab: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderContentInput {
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadInput {
    element: BrowserElementInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadInput {
    element: BrowserElementInput,
    files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluateInput {
    request: BrowserEvaluation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserElementInput {
    reference: BrowserElementRef,
    #[serde(rename = "role")]
    _role: String,
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "focused")]
    _focused: bool,
}

impl BrowserElementInput {
    fn into_reference(self) -> BrowserElementRef {
        self.reference
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ManagedAct {
    Input(BrowserAction),
    Dialog(ManagedDialogAction),
}

fn decode_managed_act(mut value: serde_json::Value) -> Result<ManagedAct, BrowserHostFailure> {
    let object = value
        .as_object_mut()
        .ok_or(BrowserHostFailure::InvalidInput("Browser Action payload"))?;
    for key in ["element", "from", "to"] {
        let Some(element) = object.get_mut(key) else {
            continue;
        };
        let canonical = serde_json::from_value::<BrowserElementInput>(element.take())
            .map_err(|_| BrowserHostFailure::InvalidInput("Browser observed element"))?;
        *element = serde_json::to_value(canonical.into_reference())
            .map_err(|_| BrowserHostFailure::Serialization)?;
    }
    serde_json::from_value(value)
        .map_err(|_| BrowserHostFailure::InvalidInput("Browser Action payload"))
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ManagedDialogAction {
    Dialog {
        target: nomifun_browser_platform::runtime::BrowserTabTarget,
        request_id: String,
        accept: bool,
        text: Option<String>,
    },
}

impl From<ManagedDialogAction> for BrowserDialogReply {
    fn from(value: ManagedDialogAction) -> Self {
        match value {
            ManagedDialogAction::Dialog {
                target,
                request_id,
                accept,
                text,
            } => Self {
                target,
                request_id,
                accept,
                text,
            },
        }
    }
}

fn canonical_attached_observation(
    admission: &BrowserTurnAdmission,
    output: serde_json::Value,
) -> Result<StrictJsonValue, BrowserHostFailure> {
    let tab_id = output
        .get("tab_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .ok_or(BrowserHostFailure::InvalidInput(
            "attached Browser observation tab",
        ))?;
    let observation_id = output
        .get("observation_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or(BrowserHostFailure::InvalidInput(
            "attached Browser observation identity",
        ))?;
    let content = output
        .get("content")
        .and_then(serde_json::Value::as_str)
        .filter(|value| value.len() <= 200_000)
        .ok_or(BrowserHostFailure::InvalidInput(
            "attached Browser observation content",
        ))?;
    let source_elements = output
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .filter(|elements| elements.len() <= 2_000)
        .ok_or(BrowserHostFailure::InvalidInput(
            "attached Browser observation elements",
        ))?;
    let observation_generation = attached_observation_generation(observation_id);
    let target = serde_json::json!({
        "tab_id":tab_id,
        "runtime_generation":0,
        "document_generation":0,
    });
    let mut elements = Vec::with_capacity(source_elements.len());
    for element in source_elements {
        let ref_id = element
            .get("ref_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(BrowserHostFailure::InvalidInput(
                "attached Browser element reference",
            ))?;
        let role = element
            .get("role")
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() <= 512)
            .unwrap_or_default();
        let name = element
            .get("name")
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.len() <= 4_096)
            .unwrap_or_default();
        elements.push(serde_json::json!({
            "reference":{
                "target":target.clone(),
                "observation_generation":observation_generation,
                "ref_id":ref_id,
            },
            "role":role,
            "name":name,
            "focused":element.get("focused").and_then(serde_json::Value::as_bool).unwrap_or(false),
        }));
    }
    {
        let mut observations = admission.attached_observations.lock().map_err(|_| {
            BrowserHostFailure::Unavailable("attached observation state poisoned".into())
        })?;
        observations.retain(|(observed_tab, _), _| observed_tab != tab_id);
        observations.insert(
            (tab_id.to_owned(), observation_generation),
            observation_id.to_owned(),
        );
    }
    Ok(StrictJsonValue(serde_json::json!({
        "target":target,
        "observation_generation":observation_generation,
        "content":content,
        "elements":elements,
        "unobserved_frames":output
            .get("unobserved_child_frames")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        "untrusted_page_content":true,
    })))
}

fn attached_observation_generation(observation_id: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(observation_id.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 has eight bytes")).max(1)
}

fn forget_attached_observations(
    admission: &BrowserTurnAdmission,
    tab_id: &str,
) -> Result<(), BrowserHostFailure> {
    admission
        .attached_observations
        .lock()
        .map_err(|_| {
            BrowserHostFailure::Unavailable("attached observation state poisoned".into())
        })?
        .retain(|(observed_tab, _), _| observed_tab != tab_id);
    Ok(())
}

fn attached_element_identity(
    admission: &BrowserTurnAdmission,
    element: BrowserElementRef,
) -> Result<(String, String, String), BrowserHostFailure> {
    let BrowserElementRef {
        target,
        observation_generation,
        ref_id,
    } = element;
    let tab_id = canonical_attached_target(target)?;
    if ref_id.is_empty() || ref_id.len() > 128 {
        return Err(BrowserHostFailure::InvalidInput(
            "attached Browser element reference",
        ));
    }
    let observation_id = admission
        .attached_observations
        .lock()
        .map_err(|_| {
            BrowserHostFailure::Unavailable("attached observation state poisoned".into())
        })?
        .get(&(tab_id.clone(), observation_generation))
        .cloned()
        .ok_or_else(|| BrowserHostFailure::Attached(AttachedBrowserRuntimeError::InvalidInput))?;
    Ok((tab_id, observation_id, ref_id))
}

fn canonical_attached_target(target: BrowserTabTarget) -> Result<String, BrowserHostFailure> {
    if target.runtime_generation != 0 || target.document_generation != 0 {
        return Err(BrowserHostFailure::InvalidInput(
            "attached Browser target generation",
        ));
    }
    if target.tab_id.is_empty() || target.tab_id.len() > 512 {
        return Err(BrowserHostFailure::InvalidInput(
            "attached Browser target",
        ));
    }
    Ok(target.tab_id)
}

fn canonical_attached_action(
    admission: &BrowserTurnAdmission,
    value: serde_json::Value,
) -> Result<AttachedBrowserCommand, BrowserHostFailure> {
    let value = decode_managed_act(value)?;
    match value {
        ManagedAct::Input(action) => match action {
            BrowserAction::Click {
                element,
                button: BrowserMouseButton::Left,
                click_count: 1,
            } => {
                let (tab_id, observation_id, ref_id) =
                    attached_element_identity(admission, element)?;
                Ok(AttachedBrowserCommand::Click {
                    tab_id,
                    observation_id,
                    ref_id,
                })
            }
            BrowserAction::Type { element, text } => {
                let (tab_id, observation_id, ref_id) =
                    attached_element_identity(admission, element)?;
                Ok(AttachedBrowserCommand::Type {
                    tab_id,
                    observation_id,
                    ref_id,
                    text,
                    replace: false,
                })
            }
            BrowserAction::Press { element, keys } => {
                let (tab_id, observation_id, _) =
                    attached_element_identity(admission, element)?;
                Ok(AttachedBrowserCommand::Press {
                    tab_id,
                    observation_id,
                    keys,
                })
            }
            BrowserAction::Scroll {
                element,
                delta_x,
                delta_y,
            } => {
                let (tab_id, observation_id, _) =
                    attached_element_identity(admission, element)?;
                Ok(AttachedBrowserCommand::Scroll {
                    tab_id,
                    observation_id,
                    delta_x,
                    delta_y,
                })
            }
            BrowserAction::Click { .. }
            | BrowserAction::Hover { .. }
            | BrowserAction::Select { .. }
            | BrowserAction::Drag { .. } => Err(BrowserHostFailure::Unsupported(
                "selected attached Chrome Provider cannot perform this Browser input".into(),
            )),
        },
        ManagedAct::Dialog(ManagedDialogAction::Dialog {
            target,
            request_id,
            accept,
            text,
        }) => Ok(AttachedBrowserCommand::Dialog {
            tab_id: canonical_attached_target(target)?,
            dialog_id: request_id,
            accept,
            prompt_text: text,
        }),
    }
}

fn attached_command_tab(command: &AttachedBrowserCommand) -> Result<&str, BrowserHostFailure> {
    match command {
        AttachedBrowserCommand::Observe { tab_id }
        | AttachedBrowserCommand::Navigate { tab_id, .. }
        | AttachedBrowserCommand::Dialog { tab_id, .. }
        | AttachedBrowserCommand::Click { tab_id, .. }
        | AttachedBrowserCommand::Type { tab_id, .. }
        | AttachedBrowserCommand::Press { tab_id, .. }
        | AttachedBrowserCommand::Scroll { tab_id, .. } => Ok(tab_id),
        AttachedBrowserCommand::Tabs {} => Err(BrowserHostFailure::InvalidInput(
            "attached Browser target",
        )),
    }
}

fn set_attached_target(
    admission: &BrowserTurnAdmission,
    tab_id: &str,
) -> Result<(), BrowserHostFailure> {
    if tab_id.trim().is_empty() || tab_id.len() > 512 {
        return Err(BrowserHostFailure::InvalidInput("attached Browser tab"));
    }
    *admission
        .attached_target
        .lock()
        .map_err(|_| BrowserHostFailure::Unavailable("attached target state poisoned".into()))? =
        Some(tab_id.to_owned());
    Ok(())
}

fn remember_single_attached_target(
    admission: &BrowserTurnAdmission,
    output: &serde_json::Value,
) -> Result<(), BrowserHostFailure> {
    let Some(tabs) = output.get("tabs").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    if let [tab] = tabs.as_slice()
        && let Some(tab_id) = tab.get("tab_id").and_then(serde_json::Value::as_str)
    {
        set_attached_target(admission, tab_id)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "engine_browser_tools/interaction_tests.rs"]
mod interaction_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_agent_contracts::{
        DigestHex, PackageRef, ResourceBindingId, ResourceId, ResourceKind,
        ResolvedSnapshotId,
    };
    use nomifun_browser_platform::runtime::{BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime};
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::Ordering;

    struct NeverRuntimeFactory;

    #[async_trait]
    impl BrowserRuntimeFactory for NeverRuntimeFactory {
        async fn create(
            &self,
            _request: CreateBrowserRuntime,
        ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
            Err(WorkspaceError::NativeUnavailable)
        }
    }

    fn provider() -> ExactRoleProviderRef {
        let registration = nomifun_agent_domain_wave2::registrations()
            .unwrap()
            .into_iter()
            .find(|registration| {
                registration.metadata.manifest.payload.package_id.as_ref()
                    == nomifun_agent_domain_wave2::BROWSER_PACKAGE_ID
            })
            .unwrap();
        let manifest = &registration.metadata.manifest.payload;
        let contribution = &manifest.contributions.role_providers[0];
        ExactRoleProviderRef {
            role: contribution.role.clone(),
            package: PackageRef {
                id: manifest.package_id.clone(),
                version: manifest.package_version.clone(),
            },
            mount_id: nomifun_agent_domain_wave2::BROWSER_MOUNT_ID.into(),
            contribution_digest: nomifun_agent_contracts::digest_payload(contribution).unwrap(),
        }
    }

    pub(super) fn render_admission(owner: &str, session: &str) -> Arc<BrowserTurnAdmission> {
        let provider = provider();
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("browser:managed-browser"),
            resource_kind: ResourceKind::from(BROWSER_RESOURCE_KIND),
            resource_id: ResourceId::from("managed-browser"),
            owner_id: owner.to_owned(),
            operations: BTreeSet::from(["render_content".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([
                ("provider_kind".to_owned(), "managed".to_owned()),
            ]),
        };
        let descriptor = browser_workspace_provider::provider_descriptor(&provider, &binding)
            .unwrap();
        let resource = browser_workspace_provider::browser_resource_binding(
            binding.clone(),
            descriptor,
        )
        .unwrap();
        let authority = BrowserSessionAuthority::from_action_ids(
            owner,
            session,
            ["browser/render_content"],
            resource,
        )
        .unwrap();
        Arc::new(BrowserTurnAdmission {
            key: TurnKey::new(owner, session, "turn"),
            principal: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: owner.to_owned(),
            },
            snapshot: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("a".repeat(64)),
            },
            registry_generation: 1,
            provider,
            binding,
            authority,
            upload_scope: None,
            download_scope: None,
            run: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            attached_target: Mutex::new(None),
            attached_observations: Mutex::new(BTreeMap::new()),
            observations_started: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        })
    }

    #[test]
    fn attached_provider_translates_only_fresh_canonical_browser_elements() {
        let admission = render_admission("owner", "session");
        let observed = canonical_attached_observation(
            &admission,
            json!({
                "tab_id":"tab",
                "observation_id":"observation-1",
                "content":"button Increment",
                "elements":[{"ref_id":"ref","role":"button","name":"Increment","focused":false}],
                "unobserved_child_frames":0
            }),
        )
        .unwrap();
        let element = observed.0["elements"][0].clone();
        let valid = canonical_attached_action(
            &admission,
            json!({"action":"click","element":element.clone()}),
        )
        .unwrap();
        assert!(matches!(valid, AttachedBrowserCommand::Click {
            ref tab_id, ref observation_id, ref ref_id
        } if tab_id == "tab" && observation_id == "observation-1" && ref_id == "ref"));
        canonical_attached_observation(
            &admission,
            json!({
                "tab_id":"tab",
                "observation_id":"observation-2",
                "content":"button Increment",
                "elements":[{"ref_id":"ref","role":"button","name":"Increment","focused":false}]
            }),
        )
        .unwrap();
        assert!(canonical_attached_action(
            &admission,
            json!({"action":"click","element":element}),
        )
        .is_err());
        for invalid in [
            json!({"action":"click","tab_id":"tab","observation_id":"observation","ref_id":"ref","principal_id":"forged"}),
            json!({"action":"navigate","tab_id":"tab","url":"https://example.com"}),
            json!({"action":"type","tab_id":"tab","observation_id":"observation","ref_id":"ref"}),
        ] {
            assert!(canonical_attached_action(&admission, invalid).is_err());
        }
    }

    #[test]
    fn only_physical_uncertainty_fences_external_effect_replay() {
        assert!(browser_outcome_may_be_unknown("BROWSER_ACTION_INTERRUPTED"));
        assert!(browser_outcome_may_be_unknown("BROWSER_PROVIDER_DISCONNECTED"));
        assert!(browser_outcome_may_be_unknown("BROWSER_NATIVE_COMMAND_FAILED"));
        assert!(!browser_outcome_may_be_unknown("INVALID_PAYLOAD"));
        assert!(!browser_outcome_may_be_unknown("CAPABILITY_UNAVAILABLE"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_PROVIDER_UNAVAILABLE"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_NATIVE_SURFACE_UNAVAILABLE"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_STALE_OBSERVATION"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_UPLOAD_SOURCE_CHANGED"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_UPLOAD_SOURCE_NAME_UNSUPPORTED"));
        assert!(!browser_outcome_may_be_unknown("BROWSER_OPERATION_CANCELLED"));
    }

    #[test]
    fn managed_dialog_is_an_exact_browser_act_payload() {
        let action = serde_json::from_value::<ManagedAct>(json!({
            "action":"dialog",
            "target":{"tab_id":"tab","runtime_generation":2,"document_generation":3},
            "request_id":"dialog",
            "accept":true,
            "text":"ok"
        }));
        assert!(matches!(action, Ok(ManagedAct::Dialog(_))));
    }

    #[tokio::test]
    async fn render_content_uses_authorized_managed_resource_without_opening_interactive_run() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(
            database.pool().clone(),
        )
        .await
        .unwrap();
        let resources = Arc::new(BrowserResourceService::new(Arc::new(
            NeverRuntimeFactory,
        )));
        let (render, calls) = crate::headless_render::test_support::runtime();
        let root = tempfile::tempdir().unwrap();
        let owner = BrowserRoleOwner::new(
            Some(resources),
            None,
            root.path().to_path_buf(),
            Some(render.clone()),
            store,
        );
        let admission = render_admission("owner", "session");
        let output = owner
            .render_content(
                &admission,
                StrictJsonValue(json!({"url":"https://example.com/render"})),
            )
            .await
            .unwrap();
        assert_eq!(output.0["final_url"], "https://example.com/render");
        assert!(output.0["html"].as_str().unwrap().contains("selected Provider"));
        assert_eq!(output.0["html_truncated"], false);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(admission.active_run().unwrap().is_none());
        assert!(owner.managed_turns.lock().unwrap().is_empty());
        assert!(owner.resources.as_ref().unwrap().get_for_agent_session("owner", "session").await.unwrap().is_none());
        render.shutdown().await.unwrap();
        database.close().await;
    }
}

#[cfg(test)]
mod managed_user_browser_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use nomifun_browser_platform::{
        workspace::managed_workspace_key,
        run_guard::{NativeInputGate, RunAdmissionError},
        runtime::{BrowserNativeSurfacePort, BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime},
    };

    struct GateRuntime { generation: u64, fail_lock: Arc<AtomicBool>, fail_unlock: Arc<AtomicBool>, locked: Arc<AtomicBool> }
    #[async_trait::async_trait]
    impl NativeInputGate for GateRuntime {
        async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
            self.locked.store(true, Ordering::SeqCst);
            if self.fail_lock.swap(false, Ordering::SeqCst) { return Err(RunAdmissionError::InputGateFailed); }
            Ok(())
        }
        async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> { Ok(()) }
        async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
            if self.fail_unlock.swap(false, Ordering::SeqCst) { return Err(RunAdmissionError::InputGateFailed); }
            self.locked.store(false, Ordering::SeqCst); Ok(())
        }
    }
    #[async_trait::async_trait]
    impl BrowserRuntime for GateRuntime {
        fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { None }
        async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
            Ok(BrowserRuntimeSnapshot { runtime_generation: self.generation, revision: 1, active_tab_id: None, tabs: vec![], downloads: vec![] })
        }
        async fn execute(&self, _: BrowserTabCommand, _: tokio_util::sync::CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { self.snapshot().await }
        async fn close(&self) -> Result<(), WorkspaceError> { Ok(()) }
    }
    struct GateFactory(Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicBool>);
    #[async_trait::async_trait]
    impl BrowserRuntimeFactory for GateFactory {
        async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
            self.1.store(!request.user_input_enabled, Ordering::SeqCst);
            Ok(Arc::new(GateRuntime { generation: request.runtime_generation, fail_lock: self.0.clone(), locked: self.1.clone(), fail_unlock: self.2.clone() }))
        }
    }

    fn navigate_admission(owner: &str, session: &str) -> Arc<BrowserTurnAdmission> {
        let mut admission = super::tests::render_admission(owner, session);
        let value = Arc::get_mut(&mut admission).unwrap();
        value.binding.operations = std::collections::BTreeSet::from(["navigate".to_owned()]);
        let descriptor = browser_workspace_provider::provider_descriptor(&value.provider, &value.binding).unwrap();
        value.authority = BrowserSessionAuthority::from_action_ids(owner, session, ["browser/navigate"],
            browser_workspace_provider::browser_resource_binding(value.binding.clone(), descriptor).unwrap()).unwrap();
        admission
    }

    #[tokio::test]
    async fn user_close_serializes_first_browser_use_and_recovers_failed_start_only_after_its_canonical_terminal() {
        use crate::browser_workspace_provider::BrowserUserClosePort;
        use nomifun_agent_contracts::{AgentBindingValue, AgentPresetId, DigestHex, PresetRevisionRef,
            SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef};
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let sessions = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(db.pool().clone()).await.unwrap();
        let principal_id = "0190f5fe-7c00-7a00-8000-000000000001";
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: principal_id.into() };
        let authority = navigate_admission(principal_id, "before-open");
        let binding = AgentBindingValue {
            preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("browser-close-fixture"), revision: 1, revision_digest: DigestHex::from("a".repeat(64)) },
            resolved_snapshot_ref: authority.snapshot.clone(), typed_resource_bindings: vec![authority.binding.clone()], binding_version: 1,
        };
        let opened = sessions.open(principal.clone(), binding, None, vec![], "browser-close-fixture", 1).await.unwrap();
        let session = opened.session.agent_session_id;
        let active = sessions.start_turn(&principal, &session, "browser-close-failed-start", serde_json::json!({"content":"use the browser"})).await.unwrap();
        let mut admission = navigate_admission(principal_id, session.as_ref());
        Arc::get_mut(&mut admission).unwrap().key.turn_id = active.operation_id.as_ref().to_owned();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let physical_locked = Arc::new(AtomicBool::new(false));
        let resources = Arc::new(BrowserResourceService::new(Arc::new(GateFactory(
            Arc::new(AtomicBool::new(true)), physical_locked.clone(), Arc::new(AtomicBool::new(false))))));
        let owner = BrowserRoleOwner::new(Some(resources.clone()), None, root.path().to_path_buf(), None, store.clone());
        owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        let key = managed_workspace_key(principal_id, session.as_ref()).unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let user = resources.ensure_user(principal_id, session.as_ref(), profile).await.unwrap();
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/user".into() }).await.unwrap();
        let generation = user.runtime_generation();

        // A click racing first browser use fails immediately instead of waiting
        // to destroy whichever runtime the browser invocation later claims.
        let operation = admission.operation.lock().await;
        let close = tokio::time::timeout(std::time::Duration::from_secs(1),
            owner.close_user_workspace(principal_id, session.as_ref(), generation)).await.unwrap();
        assert!(matches!(close, Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));
        drop(operation);
        assert!(owner.dispatch(&admission, Wave2TypedCapabilityOperation::BrowserNavigate {
            input: StrictJsonValue(serde_json::json!({"url":"https://example.test/agent"})),
        }).await.is_err());
        assert!(admission.active_run().unwrap().is_none(), "native admission failed before it could issue a guard");
        assert_eq!(owner.managed_turns.lock().unwrap().len(), 1);
        assert!(physical_locked.load(Ordering::SeqCst));
        assert!(matches!(owner.close_user_workspace(principal_id, session.as_ref(), generation).await,
            Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));
        owner.settle_turn(principal_id, session.as_ref(), active.operation_id.as_ref()).await.unwrap();
        assert!(matches!(owner.close_user_workspace(principal_id, session.as_ref(), generation).await,
            Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));

        let started = sessions.turn_receipt(&principal, &session, &active.operation_id).await.unwrap().started_event.unwrap();
        store.append_turn_terminal(&SessionEventAppend {
            agent_session_id: session.clone(), event_id: "browser-close-failed-terminal".into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: "browser-close-failed-terminal".into(), semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/failed".into()), kind_version: 1, correlation_id: active.operation_id.as_ref().into(),
                causation_event_id: Some(started.event_id), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::json!({"error":"native admission failed"}))),
            },
        }, &active.operation_id).await.unwrap();
        owner.close_user_workspace(principal_id, session.as_ref(), generation).await.unwrap();
        assert!(resources.get_for_agent_session(principal_id, session.as_ref()).await.unwrap().is_none());
        owner.finish_turn(principal_id, session.as_ref(), active.operation_id.as_ref()).await.unwrap();
        assert!(owner.managed_turns.lock().unwrap().is_empty());
        assert!(owner.turns.lock().unwrap().is_empty());
        resources.shutdown().await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn parked_browser_yields_user_control_and_retires_old_observation_authority() {
        use nomifun_agent_contracts::{AgentBindingValue,AgentPresetId,PresetRevisionRef,
            SemanticSessionEventDraft,SessionEventAppend,SessionEventKind,SessionEventPayloadRef};
        let db=nomifun_db::init_database_memory().await.unwrap();
        let store=nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let sessions=nomifun_conversation::CanonicalAgentSessionOwner::from_pool(db.pool().clone()).await.unwrap();
        let principal_id="0190f5fe-7c00-7a00-8000-000000000001";
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:principal_id.into()};
        let authority=navigate_admission(principal_id,"before-open");
        let opened=sessions.open(principal.clone(),AgentBindingValue {
            preset_revision_ref:PresetRevisionRef {preset_id:AgentPresetId::from("browser-wait-fixture"),revision:1,revision_digest:"a".repeat(64).into()},
            resolved_snapshot_ref:authority.snapshot.clone(),typed_resource_bindings:vec![authority.binding.clone()],binding_version:1,
        },None,vec![],"browser-wait-fixture",1).await.unwrap();
        let session=opened.session.agent_session_id;
        let active=sessions.start_turn(&principal,&session,"browser-wait-turn",serde_json::json!({"content":"Authorize in the browser"})).await.unwrap();
        let started=sessions.turn_receipt(&principal,&session,&active.operation_id).await.unwrap().started_event.unwrap();
        let root=tempfile::tempdir().unwrap();crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let locked=Arc::new(AtomicBool::new(false));
        let resources=Arc::new(BrowserResourceService::new(Arc::new(GateFactory(
            Arc::new(AtomicBool::new(false)),locked.clone(),Arc::new(AtomicBool::new(false))))));
        let owner=BrowserRoleOwner::new(Some(resources.clone()),None,root.path().to_path_buf(),None,store.clone());
        let mut original=navigate_admission(principal_id,session.as_ref());
        Arc::get_mut(&mut original).unwrap().key.turn_id=active.operation_id.as_ref().to_owned();
        owner.turns.lock().unwrap().insert(original.key.clone(),original.clone());
        owner.dispatch(&original,Wave2TypedCapabilityOperation::BrowserNavigate {
            input:StrictJsonValue(serde_json::json!({"url":"https://example.test/authorize"})),
        }).await.unwrap();
        assert!(locked.load(Ordering::SeqCst));
        assert!(owner.yield_to_user(principal_id,session.as_ref(),active.operation_id.as_ref()).await.is_err(),"an active unparked model cannot yield user input");
        assert!(locked.load(Ordering::SeqCst));assert!(!original.cancelled.load(Ordering::Acquire));
        store.append_event(&SessionEventAppend {
            agent_session_id:session.clone(),event_id:"browser-park".into(),producer_id:"runtime_supervisor".into(),idempotency_key:"browser-park".into(),
            semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("runtime/progress-recorded".into()),kind_version:1,
                correlation_id:active.operation_id.as_ref().into(),causation_event_id:Some(started.event_id),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::json!({"producer_seq":1,
                    "event":{"event":"turn_waiting","model_steps":1,"request_id":null,"reason":"external_resource"}})))},
        }).await.unwrap();
        owner.yield_to_user(principal_id,session.as_ref(),active.operation_id.as_ref()).await.unwrap();
        assert!(!locked.load(Ordering::SeqCst));
        assert!(original.cancelled.load(Ordering::Acquire),"old native operations remain revoked");
        assert!(owner.turns.lock().unwrap().is_empty());
        let user=resources.get_for_agent_session(principal_id,session.as_ref()).await.unwrap().unwrap();
        user.user_command(BrowserTabCommand::Create {url:"https://example.test/human-authorized".into()}).await.unwrap();
        let mut resumed=navigate_admission(principal_id,session.as_ref());
        Arc::get_mut(&mut resumed).unwrap().key.turn_id=active.operation_id.as_ref().to_owned();
        owner.turns.lock().unwrap().insert(resumed.key.clone(),resumed.clone());
        assert!(resumed.active_run().unwrap().is_none());
        assert!(resumed.attached_observations.lock().unwrap().is_empty());
        owner.dispatch(&resumed,Wave2TypedCapabilityOperation::BrowserNavigate {
            input:StrictJsonValue(serde_json::json!({"url":"https://example.test/verify"})),
        }).await.unwrap();
        assert!(locked.load(Ordering::SeqCst));
        owner.settle_turn(principal_id,session.as_ref(),active.operation_id.as_ref()).await.unwrap();
        owner.finish_turn(principal_id,session.as_ref(),active.operation_id.as_ref()).await.unwrap();
        resources.shutdown().await.unwrap();db.close().await;
    }

    #[tokio::test]
    async fn cancelling_unused_browser_authority_never_claims_user_input() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let physical_locked = Arc::new(AtomicBool::new(false));
        let resources = Arc::new(BrowserResourceService::new(Arc::new(GateFactory(
            Arc::new(AtomicBool::new(false)), physical_locked.clone(), Arc::new(AtomicBool::new(false))))));
        let admission = navigate_admission("alice", "cancel-before-browser");
        let owner = BrowserRoleOwner::new(Some(resources.clone()), None, root.path().to_path_buf(), None, store);
        owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        let key = managed_workspace_key("alice", "cancel-before-browser").unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let user = resources.ensure_user("alice", "cancel-before-browser", profile).await.unwrap();
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/user".into() }).await.unwrap();

        owner.cancel_turn("alice", "cancel-before-browser", "turn").unwrap();
        assert!(matches!(owner.dispatch(&admission, Wave2TypedCapabilityOperation::BrowserNavigate {
            input: StrictJsonValue(serde_json::json!({"url":"https://example.test/agent"})),
        }).await, Err(BrowserHostFailure::Cancelled)));
        owner.settle_turn("alice", "cancel-before-browser", "turn").await.unwrap();
        assert!(admission.active_run().unwrap().is_none());
        assert!(owner.managed_turns.lock().unwrap().is_empty());
        assert!(!physical_locked.load(Ordering::SeqCst));
        assert_eq!(user.snapshot().await.unwrap().run.input_state, nomifun_browser_platform::run_guard::BrowserInputState::UserReady);
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/while-cancelled".into() }).await.unwrap();
        resources.close_idle(key, user.runtime_generation()).await.unwrap();
        owner.finish_turn("alice", "cancel-before-browser", "turn").await.unwrap();
        assert!(owner.turns.lock().unwrap().is_empty());
        resources.shutdown().await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn a_known_closed_first_binding_does_not_pin_the_turn_to_its_retired_generation() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let physical_locked = Arc::new(AtomicBool::new(false));
        let resources = Arc::new(BrowserResourceService::new(Arc::new(GateFactory(
            Arc::new(AtomicBool::new(false)), physical_locked.clone(), Arc::new(AtomicBool::new(false))))));
        let admission = navigate_admission("alice", "closed-before-browser");
        let owner = BrowserRoleOwner::new(Some(resources.clone()), None, root.path().to_path_buf(), None, store);
        owner.turns.lock().unwrap().insert(admission.key.clone(), admission.clone());
        let key = managed_workspace_key("alice", "closed-before-browser").unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let closed = resources.ensure_user("alice", "closed-before-browser", profile.clone()).await.unwrap();
        resources.close_idle(key.clone(), closed.runtime_generation()).await.unwrap();
        assert!(owner.begin_managed_turn(&admission.key, closed.clone(), false).await.is_err());
        assert!(owner.managed_turns.lock().unwrap().is_empty());
        assert!(admission.active_run().unwrap().is_none());

        let replacement = resources.ensure_user("alice", "closed-before-browser", profile).await.unwrap();
        assert!(replacement.runtime_generation() > closed.runtime_generation());
        owner.dispatch(&admission, Wave2TypedCapabilityOperation::BrowserNavigate {
            input: StrictJsonValue(serde_json::json!({"url":"https://example.test/agent"})),
        }).await.unwrap();
        assert!(physical_locked.load(Ordering::SeqCst));
        owner.settle_turn("alice", "closed-before-browser", "turn").await.unwrap();
        owner.finish_turn("alice", "closed-before-browser", "turn").await.unwrap();
        resources.close_idle(key, replacement.runtime_generation()).await.unwrap();
        resources.shutdown().await.unwrap();
        db.close().await;
    }

    #[tokio::test]
    async fn a_failed_managed_use_is_recovered_before_the_next_turn() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let fail_lock = Arc::new(AtomicBool::new(true));
        let physical_locked = Arc::new(AtomicBool::new(false));
        let resources = Arc::new(BrowserResourceService::new(Arc::new(GateFactory(fail_lock, physical_locked.clone(), Arc::new(AtomicBool::new(false))))));
        let key = managed_workspace_key("alice", "user-session").unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let user = resources.ensure_user("alice", "user-session", profile).await.unwrap();
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/".into() }).await.unwrap();
        let owner = BrowserRoleOwner::new(Some(resources), None, root.path().to_path_buf(), None, store);
        let session: AgentSessionId = "user-session".into();
        assert!(owner.begin_managed_turn(&TurnKey::new("alice", session.as_ref(), "failed-turn"), user.clone(), false).await.is_err());
        assert!(user.snapshot().await.unwrap().run.input_gate_failed);
        assert!(owner.turns.lock().unwrap().is_empty(), "no Browser Tool authority was created");
        owner.settle_turn("alice", "user-session", "failed-turn").await.unwrap();
        assert!(user.snapshot().await.unwrap().run.input_gate_failed, "downstream cleanup or terminal may still fail; retain the physical gate");
        assert!(physical_locked.load(Ordering::SeqCst), "a later cleanup or terminal failure cannot restore physical input");
        assert!(owner.ensure_previous_turn_finished("alice", "user-session").is_err());
        owner.finish_turn("alice", "user-session", "failed-turn").await.unwrap();
        assert!(!user.snapshot().await.unwrap().run.input_gate_failed);
        assert!(!physical_locked.load(Ordering::SeqCst));
        assert!(owner.managed_turns.lock().unwrap().is_empty());
        owner.begin_managed_turn(&TurnKey::new("alice", session.as_ref(), "next-turn"), user.clone(), false).await.unwrap();
        assert_eq!(user.snapshot().await.unwrap().run.input_state, nomifun_browser_platform::run_guard::BrowserInputState::AgentRunning);
        assert!(matches!(user.user_command(BrowserTabCommand::Create { url: "https://example.test/".into() }).await,
            Err(WorkspaceError::Admission(RunAdmissionError::UserInputLocked))));
        owner.settle_turn("alice", "user-session", "next-turn").await.unwrap();
        assert_eq!(user.snapshot().await.unwrap().run.input_state, nomifun_browser_platform::run_guard::BrowserInputState::AgentRunning,
            "Browser drain is not the canonical terminal and cannot release user input");
        assert!(physical_locked.load(Ordering::SeqCst));
        owner.finish_turn("alice", "user-session", "next-turn").await.unwrap();
        assert_eq!(user.snapshot().await.unwrap().run.input_state, nomifun_browser_platform::run_guard::BrowserInputState::UserReady);
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/".into() }).await.unwrap();
        db.close().await;
    }
    #[tokio::test]
    async fn durable_terminal_release_failure_retains_the_same_owner_for_retry_before_a_successor() {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = nomifun_agent_session::AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        crate::config::load_or_create_storage_generation(root.path()).unwrap();
        let physical_locked = Arc::new(AtomicBool::new(false));
        let factory = Arc::new(GateFactory(Arc::new(AtomicBool::new(false)), physical_locked.clone(), Arc::new(AtomicBool::new(true))));
        let resources = Arc::new(BrowserResourceService::new(factory));
        let key = managed_workspace_key("alice", "terminal-retry").unwrap();
        let profile = nomifun_browser_platform::runtime::BrowserProfileStore::new(root.path()).unwrap()
            .profile_for(&key, nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).unwrap();
        let user = resources.ensure_user("alice", "terminal-retry", profile).await.unwrap();
        user.user_command(BrowserTabCommand::Create { url: "https://example.test/".into() }).await.unwrap();
        let owner = BrowserRoleOwner::new(Some(resources), None, root.path().to_path_buf(), None, store);
        owner.begin_managed_turn(&TurnKey::new("alice", "terminal-retry", "old-turn"), user.clone(), false).await.unwrap();
        owner.settle_turn("alice", "terminal-retry", "old-turn").await.unwrap();
        assert!(physical_locked.load(Ordering::SeqCst), "settlement alone must preserve native input exclusion");
        // The trusted terminal callback now attempts final native release.
        assert!(owner.finish_turn("alice", "terminal-retry", "old-turn").await.is_err());
        assert!(physical_locked.load(Ordering::SeqCst));
        assert_eq!(owner.managed_turns.lock().unwrap().len(), 1);
        assert!(owner.ensure_previous_turn_finished("alice", "terminal-retry").is_err());
        // Retrying that exact durable terminal succeeds before successor admission.
        owner.finish_turn("alice", "terminal-retry", "old-turn").await.unwrap();
        assert!(!physical_locked.load(Ordering::SeqCst));
        owner.ensure_previous_turn_finished("alice", "terminal-retry").unwrap();
        owner.begin_managed_turn(&TurnKey::new("alice", "terminal-retry", "successor"), user.clone(), false).await.unwrap();
        assert!(physical_locked.load(Ordering::SeqCst));
        owner.settle_turn("alice", "terminal-retry", "successor").await.unwrap();
        owner.finish_turn("alice", "terminal-retry", "successor").await.unwrap();
        assert!(!physical_locked.load(Ordering::SeqCst));
        db.close().await;
    }

}
