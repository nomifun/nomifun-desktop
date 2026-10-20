use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};

use crate::session_port::{
    CronRunResourceRetirement, CronTurnReceiptState, CronTurnReconciliation,
};
use dashmap::{DashMap, mapref::entry::Entry};
use nomifun_api_types::{
    AgentBindingValueDto, AgentChatModelSelectionDto, AgentResourceSelectionDto,
    CreateCronJobRequest, CronJobResponse, CronJobRunResponse, CronScheduleDto, HasSkillResponse,
    ListCronJobsQuery, ProductAgentSelectionDto, RunNowResponse, SaveCronSkillRequest,
    UpdateCronJobRequest,
};
use nomifun_common::{
    AppError, ConversationId, CronJobId, CronJobRunId, ExecutionAuthority, UserId, now_ms,
};
use nomifun_db::{
    CRON_RUN_HISTORY_LIMIT, CronRunHistoryRow, FinalizeCronRunOutcome, FinalizeCronRunParams,
    ICronRepository, ReserveCronRunParams, UpdateCronJobParams,
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::events::CronEventEmitter;

use crate::error::CronError;
use crate::executor::{ExecutionResult, JobExecutor};
use crate::scheduler::{
    CronScheduler, compute_next_run, compute_occurrence_successor, validate_schedule,
};
use crate::skill_file::{
    CRON_SKILLS_REL_DIR, build_skill_content, delete_skill_file, validate_skill_content,
    write_raw_skill_file,
};
use crate::types::{
    CreatedBy, CronJob, CronSchedule, ExecutionMode, cron_job_from_row, cron_job_to_response,
    cron_job_to_row, schedule_from_dto,
};

const PLACEHOLDER_PATTERNS: &[&str] = &[
    "todo:",
    "todo ",
    "fill in",
    "placeholder",
    "replace this",
    "your ",
    "insert ",
    "add your",
    "write your",
    "put your",
];

/// Host-owned sink for detached Cron workers.
///
/// Cron stores only this domain-owned capability. Application assembly adapts
/// its process-wide background-task registry to this trait.
pub trait CronBackgroundTaskRegistrar: Send + Sync {
    fn register(&self, task: JoinHandle<()>);
    /// Application shutdown takes over physical resources. A domain retry
    /// must not keep that shutdown waiting through its backoff.
    fn shutdown_signal(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(std::future::pending())
    }
}

/// Host-owned preparation of the standard canonical Agent binding. Cron owns
/// schedule authoring inputs; the same application compiler/resource registry
/// used by interactive Sessions owns all execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronAgentBindingPresentation {
    pub agent_label: Option<String>,
    pub model: Option<AgentChatModelSelectionDto>,
}

#[async_trait::async_trait]
pub trait CronAgentBindingResolver: Send + Sync {
    async fn binding_presentation(
        &self,
        owner_id: &str,
        binding: &AgentBindingValueDto,
    ) -> Result<CronAgentBindingPresentation, AppError>;
    async fn resolve_binding(
        &self,
        owner_id: &str,
        selection: &ProductAgentSelectionDto,
        model: Option<&AgentChatModelSelectionDto>,
        workspace: Option<&str>,
        resource_selections: &[AgentResourceSelectionDto],
    ) -> Result<AgentBindingValueDto, AppError>;
    /// Explicit edits derive from the exact saved execution contract. Missing
    /// inputs retain the original model/resources and never refresh a Preset.
    async fn revise_binding(
        &self,
        owner_id: &str,
        current: &AgentBindingValueDto,
        model: Option<&AgentChatModelSelectionDto>,
        workspace: Option<&str>,
        resource_selections: Option<&[AgentResourceSelectionDto]>,
    ) -> Result<AgentBindingValueDto, AppError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEmbeddedCreateCommand {
    pub name: String,
    pub schedule: String,
    pub schedule_description: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEmbeddedUpdateCommand {
    pub job_id: String,
    pub name: String,
    pub schedule: String,
    pub schedule_description: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEmbeddedDeleteCommand {
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEmbeddedMutationRequest<T> {
    pub operation_id: String,
    pub command: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronEmbeddedCommandResult {
    pub success: bool,
    pub message: String,
}

#[derive(Clone)]
pub struct CronEmbeddedMutationWaiter {
    receiver: watch::Receiver<Option<Result<CronEmbeddedCommandResult, String>>>,
}

impl CronEmbeddedMutationWaiter {
    /// Wait for the process-owned mutation result.
    ///
    /// Dropping this waiter never cancels the mutation owner. A later replay
    /// with the same operation identity can subscribe to the same receipt.
    pub async fn wait(mut self) -> Result<CronEmbeddedCommandResult, CronError> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result.map_err(CronError::OutcomeUnknown);
            }
            if self.receiver.changed().await.is_err() {
                return Err(CronError::OutcomeUnknown(
                    "mutation owner closed before publishing a terminal receipt".to_owned(),
                ));
            }
        }
    }
}

struct EmbeddedMutationEntry {
    fingerprint: String,
    sender: watch::Sender<Option<Result<CronEmbeddedCommandResult, String>>>,
    completed: AtomicBool,
    outcome_unknown: AtomicBool,
}

const EMBEDDED_MUTATION_OWNER_DROPPED: &str =
    "mutation owner stopped before publishing a terminal receipt";

impl EmbeddedMutationEntry {
    fn waiter(&self) -> CronEmbeddedMutationWaiter {
        CronEmbeddedMutationWaiter {
            receiver: self.sender.subscribe(),
        }
    }

    fn publish_terminal(&self, result: Result<CronEmbeddedCommandResult, String>) {
        let outcome_unknown = result.is_err();
        // `send_replace` is synchronous and retains the value even when every
        // current waiter was dropped. Publish before marking the entry
        // evictable so a same-operation subscriber can never observe a
        // completed entry whose receipt has not been installed yet.
        self.sender.send_replace(Some(result));
        self.outcome_unknown
            .store(outcome_unknown, Ordering::Release);
        self.completed.store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn evictable(&self) -> bool {
        self.completed.load(Ordering::Acquire) && !self.outcome_unknown.load(Ordering::Acquire)
    }
}

/// Owns responsibility for publishing exactly one terminal observation for a
/// detached embedded mutation. Tokio abort drops the task future, and unwinding
/// a panic drops its captures, so both paths deterministically publish an
/// unknown outcome instead of leaving the map-owned watch sender open forever.
struct EmbeddedMutationOwner {
    state: Arc<EmbeddedMutationEntry>,
    armed: bool,
}

impl EmbeddedMutationOwner {
    fn new(state: Arc<EmbeddedMutationEntry>) -> Self {
        Self { state, armed: true }
    }

    fn publish(mut self, result: Result<CronEmbeddedCommandResult, String>) {
        self.state.publish_terminal(result);
        self.armed = false;
    }
}

impl Drop for EmbeddedMutationOwner {
    fn drop(&mut self) {
        if self.armed {
            self.state
                .publish_terminal(Err(EMBEDDED_MUTATION_OWNER_DROPPED.to_owned()));
            self.armed = false;
        }
    }
}

fn validate_cron_job_id(job_id: &str) -> Result<String, CronError> {
    CronJobId::parse(job_id.trim())
        .map(CronJobId::into_string)
        .map_err(|error| {
            CronError::App(AppError::BadRequest(format!(
                "invalid cron job id: {error}"
            )))
        })
}

fn validate_cron_user_id(user_id: &str) -> Result<&str, CronError> {
    UserId::try_from(user_id).map(|_| user_id).map_err(|error| {
        CronError::App(AppError::Forbidden(format!("invalid cron caller: {error}")))
    })
}

fn validate_conversation_id(conversation_id: &str) -> Result<&str, CronError> {
    ConversationId::try_from(conversation_id)
        .map(|_| conversation_id)
        .map_err(|error| {
            CronError::App(AppError::BadRequest(format!(
                "invalid conversation id: {error}"
            )))
        })
}

#[derive(Clone)]
pub struct CronService {
    authoritative_user_id: Arc<str>,
    repo: Arc<dyn ICronRepository>,
    scheduler: Arc<CronScheduler>,
    executor: Arc<JobExecutor>,
    emitter: CronEventEmitter,
    data_dir: PathBuf,
    job_gates: Arc<DashMap<String, Weak<AsyncMutex<()>>>>,
    active_runs: Arc<DashMap<String, String>>,
    embedded_mutations: Arc<DashMap<String, Arc<EmbeddedMutationEntry>>>,
    background_task_registrar: Arc<RwLock<Option<Arc<dyn CronBackgroundTaskRegistrar>>>>,
    agent_binding_resolver: Arc<RwLock<Option<Arc<dyn CronAgentBindingResolver>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronRunCancellationReceipt {
    pub accepted: bool,
    pub execution_stopped: bool,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Default)]
struct CronJobRunProjection {
    last_run_at: Option<i64>,
    last_status: Option<String>,
    last_error: Option<Option<String>>,
    increment_run_count: bool,
    domain_failure: Option<nomifun_api_types::AgentStreamErrorData>,
}

struct ActiveScheduledRunGuard {
    runs: Arc<DashMap<String, String>>,
    run_id: String,
}

impl Drop for ActiveScheduledRunGuard {
    fn drop(&mut self) {
        self.runs.remove(&self.run_id);
    }
}

impl CronService {
    pub fn new(
        authoritative_user_id: Arc<str>,
        repo: Arc<dyn ICronRepository>,
        scheduler: Arc<CronScheduler>,
        executor: Arc<JobExecutor>,
        emitter: CronEventEmitter,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            authoritative_user_id,
            repo,
            scheduler,
            executor,
            emitter,
            data_dir,
            job_gates: Arc::new(DashMap::new()),
            active_runs: Arc::new(DashMap::new()),
            embedded_mutations: Arc::new(DashMap::new()),
            background_task_registrar: Arc::new(RwLock::new(None)),
            agent_binding_resolver: Arc::new(RwLock::new(None)),
        }
    }

    /// Install the host-owned join sink without importing a Conversation
    /// lifecycle contract into Cron core.
    pub fn with_cron_background_task_registrar(
        &self,
        registrar: Arc<dyn CronBackgroundTaskRegistrar>,
    ) {
        if let Ok(mut guard) = self.background_task_registrar.write() {
            *guard = Some(registrar);
        }
    }

    /// Install canonical create/edit preparation. Scheduled execution never
    /// asks this port to resolve a current Preset or choose another model.
    pub fn with_agent_binding_resolver(&self, resolver: Arc<dyn CronAgentBindingResolver>) {
        if let Ok(mut guard) = self.agent_binding_resolver.write() {
            *guard = Some(resolver);
        }
    }

    async fn prepare_agent_binding(
        &self,
        owner: &str,
        selection: &ProductAgentSelectionDto,
        model: Option<&AgentChatModelSelectionDto>,
        workspace: Option<&str>,
        resources: &[AgentResourceSelectionDto],
    ) -> Result<AgentBindingValueDto, CronError> {
        let resolver = self
            .agent_binding_resolver
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| {
                CronError::InvalidAgentConfig(
                    "canonical scheduled Agent preparation is unavailable".into(),
                )
            })?;
        resolver
            .resolve_binding(owner, selection, model, workspace, resources)
            .await
            .map_err(CronError::App)
    }

    /// Stop timer admission before the host begins joining background work.
    /// Already-dispatched occurrences remain owned by their registered task;
    /// this only prevents a timer from creating new database work during
    /// shutdown.
    pub fn shutdown_timers(&self) {
        self.scheduler.shutdown();
    }

    /// Return the process-local mutation/admission gate for one durable job.
    ///
    /// Weak values let idle jobs disappear without retaining a mutex forever.
    /// Dead keys are pruned opportunistically; a live waiter holds a strong
    /// reference, so pruning cannot split its critical section.
    fn job_gate(&self, job_id: &str) -> Arc<AsyncMutex<()>> {
        if self.job_gates.len() >= 1_024 {
            self.job_gates.retain(|_, gate| gate.strong_count() > 0);
        }
        match self.job_gates.entry(job_id.to_owned()) {
            Entry::Vacant(entry) => {
                let gate = Arc::new(AsyncMutex::new(()));
                entry.insert(Arc::downgrade(&gate));
                gate
            }
            Entry::Occupied(mut entry) => match entry.get().upgrade() {
                Some(gate) => gate,
                None => {
                    let gate = Arc::new(AsyncMutex::new(()));
                    entry.insert(Arc::downgrade(&gate));
                    gate
                }
            },
        }
    }

    fn mark_run_active(&self, job_id: &str, run_id: &str) -> ActiveScheduledRunGuard {
        self.active_runs
            .insert(run_id.to_owned(), job_id.to_owned());
        ActiveScheduledRunGuard {
            runs: Arc::clone(&self.active_runs),
            run_id: run_id.to_owned(),
        }
    }

    fn has_active_run_for_job(&self, job_id: &str) -> bool {
        self.active_runs.iter().any(|entry| entry.value() == job_id)
    }

    fn register_background_task(&self, task: JoinHandle<()>) {
        if let Some(registrar) = self
            .background_task_registrar
            .read()
            .ok()
            .and_then(|slot| slot.clone())
        {
            registrar.register(task);
        }
    }

    fn submit_embedded_mutation<F, Fut>(
        &self,
        user_id: &str,
        operation_id: &str,
        fingerprint: String,
        operation: F,
    ) -> Result<CronEmbeddedMutationWaiter, CronError>
    where
        F: FnOnce(CronService) -> Fut + Send + 'static,
        Fut: Future<Output = Result<CronEmbeddedCommandResult, CronError>> + Send + 'static,
    {
        let user_id = validate_cron_user_id(user_id)?.to_owned();
        let operation_id = validate_embedded_operation_id(operation_id)?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|error| {
            CronError::Scheduler(format!(
                "Cron mutation owner requires an active async runtime: {error}"
            ))
        })?;
        let operation_key = format!("{user_id}\0{operation_id}");
        if let Some(entry) = self.embedded_mutations.get(&operation_key) {
            if entry.fingerprint != fingerprint {
                return Err(CronError::App(AppError::Conflict(format!(
                    "Cron embedded operation '{operation_id}' was reused with a different request"
                ))));
            }
            return Ok(entry.waiter());
        }
        if self.embedded_mutations.len() >= 4_096 {
            return Err(CronError::App(AppError::Conflict(
                "Cron embedded operation receipt capacity is exhausted; wait for process restart before admitting a new operation"
                    .to_owned(),
            )));
        }
        match self.embedded_mutations.entry(operation_key) {
            Entry::Occupied(entry) => {
                if entry.get().fingerprint != fingerprint {
                    return Err(CronError::App(AppError::Conflict(format!(
                        "Cron embedded operation '{operation_id}' was reused with a different request"
                    ))));
                }
                Ok(entry.get().waiter())
            }
            Entry::Vacant(entry) => {
                let (sender, receiver) = watch::channel(None);
                let state = Arc::new(EmbeddedMutationEntry {
                    fingerprint,
                    sender,
                    completed: AtomicBool::new(false),
                    outcome_unknown: AtomicBool::new(false),
                });
                entry.insert(Arc::clone(&state));
                let owner = EmbeddedMutationOwner::new(Arc::clone(&state));
                let service = self.clone();
                let task = runtime.spawn(async move {
                    let result = match operation(service).await {
                        Ok(result) => Ok(result),
                        Err(CronError::OutcomeUnknown(message)) => Err(message),
                        Err(error) => Ok(embedded_error(error.to_string())),
                    };
                    owner.publish(result);
                });
                self.register_background_task(task);
                Ok(CronEmbeddedMutationWaiter { receiver })
            }
        }
    }

    fn execution_authority(&self, user_id: &str) -> ExecutionAuthority {
        ExecutionAuthority::resolve(user_id, self.authoritative_user_id.as_ref())
    }

    fn require_host_control(&self, user_id: &str) -> Result<(), CronError> {
        if self.execution_authority(user_id).controls_host() {
            Ok(())
        } else {
            Err(CronError::App(AppError::Forbidden(
                "Cron skill management requires the installation owner".into(),
            )))
        }
    }

    async fn emit_job_created_for(&self, job: &CronJob) {
        self.emitter
            .emit_job_created(&job.user_id, &cron_job_to_response(job));
    }

    async fn emit_job_updated_for(&self, job: &CronJob) {
        self.emitter
            .emit_job_updated(&job.user_id, &cron_job_to_response(job));
    }

    /// Broadcast the committed summary with the same exact read projections
    /// as HTTP; an event cannot erase Session-derived Agent/model/workspace
    /// presentation or borrow another Turn's failure.
    async fn emit_persisted_job_updated_for(&self, job: &CronJob) {
        let row = match self
            .repo
            .get_by_cron_job_id(&job.user_id, &job.cron_job_id)
            .await
        {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    job_id = %job.cron_job_id,
                    error = %error,
                    "Failed to reload cron job for update event"
                );
                return;
            }
        };
        match cron_job_from_row(row) {
            Ok(persisted) => match self.with_failure_projection(persisted).await {
                Ok(projected) => self.emit_job_updated_for(&projected).await,
                Err(error) => {
                    warn!(job_id=%job.cron_job_id,%error,"Cannot project committed Cron update")
                }
            },
            Err(error) => warn!(
                job_id = %job.cron_job_id,
                error = %error,
                "Refusing to emit an invalid persisted cron job"
            ),
        }
    }

    async fn emit_job_removed_for(&self, job: &CronJob) {
        self.emitter
            .emit_job_removed(&job.user_id, &job.cron_job_id);
    }

    async fn emit_job_executed_for(&self, job: &CronJob, status: &str, error: Option<&str>) {
        self.emitter
            .emit_job_executed(&job.user_id, &job.cron_job_id, status, error);
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    pub async fn add_job(
        &self,
        user_id: &str,
        req: CreateCronJobRequest,
    ) -> Result<CronJob, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let execution_mode = parse_execution_mode(req.execution_mode.as_deref())?;
        if execution_mode == ExecutionMode::NewConversation && req.conversation_id.is_some() {
            return Err(CronError::App(AppError::BadRequest(
                "new_conversation tasks cannot specify a Session".into(),
            )));
        }
        let schedule = schedule_from_dto(&req.schedule);
        validate_schedule(&schedule)?;
        let bound = req.conversation_id.is_some();
        if bound
            && (req.agent_selection.is_some()
                || req.model.is_some()
                || req.workspace.is_some()
                || !req.resource_selections.is_empty())
        {
            return Err(CronError::InvalidAgentConfig("A bound Session owns its Agent, model and resources; use its explicit binding transition".into()));
        }
        if let Some(session) = req.conversation_id.as_deref() {
            validate_conversation_id(session)?;
            self.executor
                .get_session_projection(user_id, session)
                .await?;
        }
        let selection = if bound {
            None
        } else {
            Some(
                req.agent_selection
                    .unwrap_or(ProductAgentSelectionDto::Template {
                        template_key: "assistant.general".into(),
                    }),
            )
        };
        let binding = if let Some(selection) = selection.as_ref() {
            Some(
                self.prepare_agent_binding(
                    user_id,
                    selection,
                    req.model.as_ref(),
                    req.workspace.as_deref(),
                    &req.resource_selections,
                )
                .await?,
            )
        } else {
            None
        };
        let now = now_ms();
        let next_run_at = compute_next_run(&schedule, now);
        validate_enabled_next_run(&schedule, true, next_run_at)?;
        let job = CronJob {
            cron_job_id: CronJobId::new().into_string(),
            user_id: user_id.into(),
            name: req.name,
            enabled: true,
            schedule_revision: 1,
            next_run_at,
            schedule,
            message: req.message.unwrap_or_default(),
            execution_mode,
            agent_selection: selection,
            model: if bound { None } else { req.model },
            agent_binding: binding,
            agent_label: None,
            effective_model: None,
            clear_context_each_run: req.clear_context_each_run,
            configuration_error: None,
            last_failure: None,
            conversation_id: req.conversation_id,
            conversation_title: req.conversation_title,
            created_by: CreatedBy::from_str(&req.created_by)?,
            skill_content: None,
            description: req.description,
            created_at: now,
            updated_at: now,
            last_run_at: None,
            last_run_id: None,
            last_status: None,
            last_error: None,
            run_count: 0,
        };
        let job = self.with_failure_projection(job).await?;
        let row = cron_job_to_row(&job)?;
        if bound {
            self.repo.insert_with_session_relation(&row).await?;
        } else {
            self.repo.insert(&row).await?;
        }
        self.scheduler.schedule_job(&job);
        self.emit_job_created_for(&job).await;
        Ok(job)
    }

    pub async fn update_job(
        &self,
        user_id: &str,
        job_id: &str,
        req: UpdateCronJobRequest,
    ) -> Result<CronJob, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        let gate = self.job_gate(&job_id);
        let _guard = gate.lock().await;
        let mut job = self.get_job(user_id, &job_id).await?;
        let previous = job.clone();
        let changes_binding = req.agent_selection.is_some()
            || req.model.is_some()
            || req.workspace.is_some()
            || req.resource_selections.is_some();
        if changes_binding
            && job.execution_mode == ExecutionMode::Existing
            && job.conversation_id.is_some()
        {
            return Err(CronError::InvalidAgentConfig("A continuing Session owns its Agent and model; change them through the Session's explicit transition".into()));
        }
        if changes_binding {
            let selection = req
                .agent_selection
                .clone()
                .or(job.agent_selection.clone())
                .unwrap_or(ProductAgentSelectionDto::Template {
                    template_key: "assistant.general".into(),
                });
            let author_model = req.model.clone().or(job.model.clone());
            let preparation_model = author_model.clone().or(job.effective_model.clone());
            let changes_agent = req.agent_selection.is_some();
            let revises_exact_binding = job.agent_binding.is_some() && !changes_agent;
            let binding =
                if let Some(current) = job.agent_binding.as_ref().filter(|_| !changes_agent) {
                    let resolver = self
                        .agent_binding_resolver
                        .read()
                        .ok()
                        .and_then(|slot| slot.clone())
                        .ok_or_else(|| {
                            CronError::InvalidAgentConfig(
                                "canonical scheduled Agent preparation is unavailable".into(),
                            )
                        })?;
                    resolver
                        .revise_binding(
                            user_id,
                            current,
                            req.model.as_ref(),
                            req.workspace.as_deref(),
                            req.resource_selections.as_deref(),
                        )
                        .await?
                } else {
                    let workspace = req.workspace.clone().or_else(|| {
                        job.agent_binding
                            .as_ref()
                            .and_then(|binding| {
                                binding
                                    .typed_resource_bindings
                                    .iter()
                                    .find(|resource| resource.resource_kind == "workspace")
                            })
                            .and_then(|resource| resource.typed_parameters.get("workspace_root"))
                            .cloned()
                    });
                    self.prepare_agent_binding(
                        user_id,
                        &selection,
                        preparation_model.as_ref(),
                        workspace.as_deref(),
                        req.resource_selections.as_deref().unwrap_or_default(),
                    )
                    .await?
                };
            job.agent_binding = Some(binding);
            if !revises_exact_binding {
                job.agent_selection = Some(selection);
            }
            job.model = author_model;
            job.configuration_error = None;
        }
        if let Some(value) = req.name {
            job.name = value;
        }
        if let Some(value) = req.description {
            job.description = Some(value);
        }
        if let Some(value) = req.message {
            job.message = value;
        }
        if let Some(value) = req.conversation_title {
            job.conversation_title = Some(value);
        }
        if let Some(value) = req.clear_context_each_run {
            job.clear_context_each_run = value;
        }
        if let Some(value) = req.enabled {
            job.enabled = value;
        }
        let replaces_timer = req.schedule.is_some() || req.enabled.is_some();
        if let Some(value) = req.schedule.as_ref() {
            job.schedule = schedule_from_dto_with_existing_timezone(value, &job.schedule);
            validate_schedule(&job.schedule)?;
        }
        if job.enabled
            && job.conversation_id.is_none()
            && (job.agent_binding.is_none() || job.configuration_error.is_some())
        {
            return Err(CronError::InvalidAgentConfig(
                "Select an Agent and model before enabling this task".into(),
            ));
        }
        if replaces_timer {
            job.schedule_revision = job
                .schedule_revision
                .checked_add(1)
                .ok_or_else(|| CronError::Scheduler("schedule revision overflow".into()))?;
            job.next_run_at = if job.enabled {
                compute_next_run(&job.schedule, now_ms())
            } else {
                None
            };
            validate_enabled_next_run(&job.schedule, job.enabled, job.next_run_at)?;
        }
        job.updated_at = now_ms();
        let job = self.with_failure_projection(job).await?;
        let mut params = update_params_from_job(&job)?;
        if replaces_timer {
            params.next_run_at = Some(job.next_run_at);
        }
        params.expected_schedule_revision = Some(previous.schedule_revision);
        if replaces_timer {
            self.scheduler.cancel_job_for_owner(&job_id, user_id);
        }
        if let Err(error) = self.repo.update(user_id, &job_id, &params).await {
            if replaces_timer {
                self.restore_timer_from_authoritative_row(user_id, &job_id, Some(&previous))
                    .await;
            }
            return Err(error.into());
        }
        if replaces_timer {
            self.scheduler.schedule_job(&job);
        }
        self.emit_job_updated_for(&job).await;
        Ok(job)
    }

    pub async fn remove_job(&self, user_id: &str, job_id: &str) -> Result<(), CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        let gate = self.job_gate(&job_id);
        let _job_guard = gate.lock().await;
        let job = self.get_job(user_id, &job_id).await?;
        if self.has_active_run_for_job(&job_id) {
            return Err(CronError::App(AppError::Conflict(format!(
                "cron job '{job_id}' has an admitted execution; \
                 wait for its durable receipt to become terminal before deletion"
            ))));
        }
        self.scheduler.cancel_job_for_owner(&job_id, user_id);
        if let Err(delete_error) = self.repo.delete(user_id, &job_id).await {
            self.restore_timer_from_authoritative_row(user_id, &job_id, Some(&job))
                .await;
            return Err(delete_error.into());
        }
        if let Err(err) = delete_skill_file(&self.data_dir, &job_id).await {
            warn!(
                job_id,
                error = %err,
                "Cron job deleted; generated skill directory is orphaned and will be retried by startup reconciliation"
            );
        }
        self.emit_job_removed_for(&job).await;
        info!(job_id, "Cron job removed");
        Ok(())
    }

    async fn read_run_failure(
        &self,
        owner: &str,
        row: &CronRunHistoryRow,
    ) -> Option<nomifun_api_types::AgentStreamErrorData> {
        if row.status != "error" {
            return None;
        }
        if let Some(raw) = row.domain_failure_json.as_deref() {
            match serde_json::from_str(raw) {
                Ok(failure) => return Some(failure),
                Err(error) => {
                    warn!(run_id=%row.cron_job_run_id,%error,"Invalid host-authored Cron domain failure");
                    return None;
                }
            }
        }
        if let Some(session) = row.conversation_id.as_deref() {
            match self
                .executor
                .turn_failure(owner, session, &row.cron_job_run_id)
                .await
            {
                Ok(Some(failure)) => return Some(failure),
                Ok(None) => {}
                Err(error) => {
                    warn!(run_id=%row.cron_job_run_id,%error,"Cannot read exact scheduled Turn failure");
                    return None;
                }
            }
        }
        row.result_error
            .as_deref()
            .map(|raw| nomifun_api_types::AgentStreamErrorData::legacy(raw, None))
    }

    async fn with_failure_projection(&self, mut job: CronJob) -> Result<CronJob, CronError> {
        if let Some(session) = job.conversation_id.as_deref() {
            let display = self
                .executor
                .get_session_projection(&job.user_id, session)
                .await?;
            job.agent_selection = display.agent_selection;
            job.agent_binding = display.agent_binding;
            job.effective_model = display.model;
            job.agent_label = display.agent_label;
        }
        if job.conversation_id.is_none()
            && let Some(binding) = job.agent_binding.as_ref()
        {
            if let Some(resolver) = self
                .agent_binding_resolver
                .read()
                .ok()
                .and_then(|slot| slot.clone())
            {
                let presentation = resolver.binding_presentation(&job.user_id, binding).await?;
                job.effective_model = presentation.model;
                job.agent_label = presentation.agent_label;
            }
        }
        if job.last_status == Some(crate::types::JobStatus::Error)
            && let Some(run) = job.last_run_id.as_deref()
        {
            if let Some(source) = self
                .repo
                .get_run_reservation(&job.user_id, &job.cron_job_id, run)
                .await?
            {
                let row = CronRunHistoryRow {
                    id: source.id,
                    cron_job_run_id: source.cron_job_run_id,
                    cron_job_id: source.cron_job_id,
                    executed_at_ms: source.settled_at_ms.unwrap_or(source.updated_at_ms),
                    created_at_ms: source.created_at_ms,
                    status: source.status,
                    conversation_id: source.conversation_id,
                    result_error: source.result_error,
                    domain_failure_json: source.domain_failure_json,
                };
                job.last_failure = self.read_run_failure(&job.user_id, &row).await;
                if let Some(failure) = job.last_failure.as_ref() {
                    job.last_error = Some(failure.message.clone());
                }
            }
        }
        Ok(job)
    }

    pub async fn get_job(&self, user_id: &str, job_id: &str) -> Result<CronJob, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        let row = self
            .repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        self.with_failure_projection(cron_job_from_row(row)?).await
    }

    pub async fn list_conversations_by_cron_job(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<Vec<nomifun_api_types::ConversationResponse>, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        self.get_job(user_id, &job_id).await?;
        self.executor
            .list_conversations_by_cron_job(user_id, &job_id)
            .await
            .map_err(CronError::from)
    }

    /// Submit an embedded create mutation to a process-owned task.
    ///
    /// The returned waiter is cancellation-safe: caller timeout only abandons
    /// observation, not the mutation. Reusing the same operation identity and
    /// payload subscribes to the existing receipt.
    pub fn submit_embedded_create(
        &self,
        user_id: &str,
        session_id: &str,
        request: CronEmbeddedMutationRequest<CronEmbeddedCreateCommand>,
    ) -> Result<CronEmbeddedMutationWaiter, CronError> {
        let CronEmbeddedMutationRequest {
            operation_id,
            command,
        } = request;
        let owned_user_id = user_id.to_owned();
        let owned_session_id = session_id.to_owned();
        let fingerprint = embedded_create_fingerprint(&owned_session_id, &command);
        self.submit_embedded_mutation(
            user_id,
            &operation_id,
            fingerprint,
            move |service| async move {
                service
                    .apply_embedded_create(&owned_user_id, &owned_session_id, &command)
                    .await
            },
        )
    }

    /// Cancellation-safe owner entry point for an embedded update mutation.
    pub fn submit_embedded_update(
        &self,
        user_id: &str,
        session_id: &str,
        request: CronEmbeddedMutationRequest<CronEmbeddedUpdateCommand>,
    ) -> Result<CronEmbeddedMutationWaiter, CronError> {
        let CronEmbeddedMutationRequest {
            operation_id,
            command,
        } = request;
        let owned_user_id = user_id.to_owned();
        let owned_session_id = session_id.to_owned();
        let fingerprint = embedded_update_fingerprint(&owned_session_id, &command);
        self.submit_embedded_mutation(
            user_id,
            &operation_id,
            fingerprint,
            move |service| async move {
                service
                    .apply_embedded_update(&owned_user_id, &owned_session_id, &command)
                    .await
            },
        )
    }

    /// Cancellation-safe owner entry point for an embedded delete mutation.
    pub fn submit_embedded_delete(
        &self,
        user_id: &str,
        request: CronEmbeddedMutationRequest<CronEmbeddedDeleteCommand>,
    ) -> Result<CronEmbeddedMutationWaiter, CronError> {
        let CronEmbeddedMutationRequest {
            operation_id,
            command,
        } = request;
        let owned_user_id = user_id.to_owned();
        let fingerprint = embedded_delete_fingerprint(&command);
        self.submit_embedded_mutation(
            user_id,
            &operation_id,
            fingerprint,
            move |service| async move {
                service
                    .apply_embedded_delete(&owned_user_id, &command.job_id)
                    .await
            },
        )
    }

    async fn apply_embedded_create(
        &self,
        user_id: &str,
        session_id: &str,
        command: &CronEmbeddedCreateCommand,
    ) -> Result<CronEmbeddedCommandResult, CronError> {
        if ConversationId::try_from(session_id).is_err() {
            return Ok(embedded_error(format!(
                "invalid conversation id '{session_id}'"
            )));
        }
        let session = match self
            .executor
            .get_session_projection(user_id, session_id)
            .await
        {
            Ok(session) => session,
            Err(error) => return Ok(embedded_error(error.to_string())),
        };
        let request = CreateCronJobRequest {
            name: command.name.clone(),
            description: None,
            schedule: CronScheduleDto::Cron {
                expr: command.schedule.clone(),
                tz: None,
                description: Some(command.schedule_description.clone()),
            },
            message: Some(command.message.clone()),
            conversation_id: Some(session_id.to_owned()),
            conversation_title: Some(session.name),
            created_by: "agent".to_owned(),
            execution_mode: Some("existing".to_owned()),
            agent_selection: None,
            model: None,
            workspace: None,
            resource_selections: vec![],
            clear_context_each_run: false,
        };
        match self.add_job(user_id, request).await {
            Ok(job) => Ok(CronEmbeddedCommandResult {
                success: true,
                message: format!("Created cron job '{}' ({})", job.name, job.cron_job_id),
            }),
            Err(error @ CronError::OutcomeUnknown(_)) => Err(error),
            Err(error) => Ok(embedded_error(error.to_string())),
        }
    }

    async fn apply_embedded_update(
        &self,
        user_id: &str,
        session_id: &str,
        command: &CronEmbeddedUpdateCommand,
    ) -> Result<CronEmbeddedCommandResult, CronError> {
        if ConversationId::try_from(session_id).is_err() {
            return Ok(embedded_error(format!(
                "invalid conversation id '{session_id}'"
            )));
        }
        let session = match self
            .executor
            .get_session_projection(user_id, session_id)
            .await
        {
            Ok(session) => session,
            Err(error) => return Ok(embedded_error(error.to_string())),
        };
        let job_id = match validate_cron_job_id(&command.job_id) {
            Ok(job_id) => job_id,
            Err(error) => return Ok(embedded_error(error.to_string())),
        };
        let row = match self.repo.get_by_cron_job_id(user_id, &job_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(embedded_error(format!("Cron job not found: {job_id}"))),
            Err(error) => return Ok(embedded_error(error.to_string())),
        };
        if row.execution_mode != ExecutionMode::Existing.as_str()
            || row.conversation_id.as_deref() != Some(session_id)
            || session
                .cron_job_id
                .as_deref()
                .is_some_and(|bound| bound != job_id.as_str())
        {
            return Ok(embedded_error(format!(
                "cron job '{job_id}' is not bound to conversation '{session_id}'"
            )));
        }

        let request = UpdateCronJobRequest {
            name: Some(command.name.clone()),
            description: None,
            enabled: None,
            schedule: Some(CronScheduleDto::Cron {
                expr: command.schedule.clone(),
                tz: None,
                description: Some(command.schedule_description.clone()),
            }),
            message: Some(command.message.clone()),
            agent_selection: None,
            model: None,
            workspace: None,
            resource_selections: None,
            clear_context_each_run: None,
            conversation_title: None,
        };
        match self.update_job(user_id, &job_id, request).await {
            Ok(job) => Ok(CronEmbeddedCommandResult {
                success: true,
                message: format!("Updated cron job '{}' ({})", job.name, job.cron_job_id),
            }),
            Err(error @ CronError::OutcomeUnknown(_)) => Err(error),
            Err(error) => Ok(embedded_error(error.to_string())),
        }
    }

    async fn apply_embedded_delete(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<CronEmbeddedCommandResult, CronError> {
        match self.remove_job(user_id, job_id).await {
            Ok(()) => Ok(CronEmbeddedCommandResult {
                success: true,
                message: format!("Deleted cron job '{job_id}'"),
            }),
            Err(error @ CronError::OutcomeUnknown(_)) => Err(error),
            Err(error) => Ok(embedded_error(error.to_string())),
        }
    }

    pub async fn list_jobs(
        &self,
        user_id: &str,
        query: &ListCronJobsQuery,
    ) -> Result<Vec<CronJob>, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        if let Some(conversation_id) = query.conversation_id.as_deref() {
            validate_conversation_id(conversation_id)?;
        }
        let rows = if let Some(conv_id) = &query.conversation_id {
            self.repo.list_by_conversation(user_id, conv_id).await?
        } else {
            self.repo.list_all(user_id).await?
        };

        let mut jobs = Vec::with_capacity(rows.len());
        for row in rows {
            jobs.push(
                self.with_failure_projection(cron_job_from_row(row)?)
                    .await?,
            );
        }
        Ok(jobs)
    }

    pub async fn list_runs(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<Vec<CronJobRunResponse>, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        self.repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;

        let rows = self
            .repo
            .list_runs_by_job(user_id, &job_id, CRON_RUN_HISTORY_LIMIT)
            .await?;

        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let failure = self.read_run_failure(user_id, &row).await;
            let mut response = cron_run_to_response(row)?;
            response.failure = failure;
            result.push(response);
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Init / Tick / Resume / RunNow
    // -----------------------------------------------------------------------

    pub async fn init(&self) {
        // Re-initialization is also a generation boundary. Revoke every old
        // callback synchronously before filesystem or database reconciliation.
        self.scheduler.cancel_all();
        self.reconcile_skill_files().await;

        let rows = match self.repo.list_enabled_for_scheduler().await {
            Ok(rows) => rows,
            Err(e) => {
                error!(error = %e, "Failed to load enabled cron jobs");
                return;
            }
        };

        let mut eligible = 0u32;
        let mut orphans = 0u32;
        for row in rows {
            let db_id = row.id;
            let raw_cron_job_id = row.cron_job_id.clone();
            let cron_job_id = match validate_cron_job_id(&raw_cron_job_id) {
                Ok(cron_job_id) => cron_job_id,
                Err(error) => {
                    error!(
                        db_id,
                        cron_job_id = %raw_cron_job_id,
                        error = %error,
                        "Init: invalid cron job id"
                    );
                    continue;
                }
            };
            let gate = self.job_gate(&cron_job_id);
            let _job_guard = gate.lock().await;
            // `list_enabled_for_scheduler` is only a work list. A user update
            // or delete may have committed while this init pass waited for
            // the per-job gate, so never make timer decisions from that stale
            // snapshot.
            let current_row = match self
                .repo
                .get_by_cron_job_id_for_scheduler(&cron_job_id)
                .await
            {
                Ok(Some(current_row)) => current_row,
                Ok(None) => continue,
                Err(error) => {
                    error!(
                        db_id,
                        cron_job_id = %cron_job_id,
                        error = %error,
                        "Init: failed to reload authoritative cron job"
                    );
                    continue;
                }
            };
            let job = match cron_job_from_row(current_row) {
                Ok(j) => j,
                Err(e) => {
                    error!(
                        db_id,
                        cron_job_id = %raw_cron_job_id,
                        error = %e,
                        "Failed to parse cron job row"
                    );
                    continue;
                }
            };

            if self.is_orphan(&job).await {
                warn!(
                    job_id = %job.cron_job_id,
                    job_name = %job.name,
                    conversation_id = ?job.conversation_id,
                    execution_mode = job.execution_mode.as_str(),
                    "Deleting orphan cron job whose bound conversation no longer exists"
                );
                if let Err(e) = self.repo.delete(&job.user_id, &job.cron_job_id).await {
                    error!(job_id = %job.cron_job_id, error = %e, "Failed to delete orphan cron job");
                } else if let Err(e) = delete_skill_file(&self.data_dir, &job.cron_job_id).await {
                    warn!(job_id = %job.cron_job_id, error = %e, "Failed to delete orphan cron skill directory");
                }
                orphans += 1;
                continue;
            }

            eligible += 1;
        }

        // This is the only process-start path. Recover every crash-ambiguous
        // admission before installing a timer: a durable `reserved` row may
        // already have emitted effects, so boot must settle it and never
        // redrive it. `handle_system_resume` then repairs missed/future timers
        // from the newly settled state.
        self.recover_interrupted_runs_on_boot().await;
        self.handle_system_resume().await;

        info!(scheduled = eligible, orphans, "Cron service initialized");
    }

    async fn recover_interrupted_runs_on_boot(&self) {
        let reservations = match self.repo.list_reserved_runs_for_scheduler().await {
            Ok(reservations) => reservations,
            Err(error) => {
                error!(error = %error, "Boot: failed to enumerate interrupted cron runs");
                return;
            }
        };

        for reservation in reservations {
            // Pre-upgrade code could update `cron_jobs` and die before it
            // settled the exact reservation. No timestamp or aggregate status
            // can identify which run performed that write, so legacy rows are
            // permanently fail-closed instead of risking a double projection.
            if reservation.job_projection_state != "pending" {
                warn!(
                    job_id = %reservation.cron_job_id,
                    run_id = %reservation.cron_job_run_id,
                    projection_state = %reservation.job_projection_state,
                    "Boot: legacy Cron reservation remains quarantined because its job projection is unknowable"
                );
                continue;
            }

            let row = match self
                .repo
                .get_by_cron_job_id_for_scheduler(&reservation.cron_job_id)
                .await
            {
                Ok(Some(row)) => row,
                Ok(None) => {
                    if reservation.cancel_requested_at_ms.is_some() {
                        match self
                            .repo
                            .attached_run_owner_for_scheduler(&reservation.cron_job_run_id)
                            .await
                        {
                            Ok(Some(owner)) => {
                                if let Err(error) = self
                                    .settle_cancelled_run_without_job(&owner, &reservation)
                                    .await
                                {
                                    warn!(run_id=%reservation.cron_job_run_id,%error,"cancelled attached Cron source without job remains unconfirmed");
                                }
                                continue;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                warn!(run_id=%reservation.cron_job_run_id,%error,"original Cron source owner cannot be read");
                                continue;
                            }
                        }
                    }
                    warn!(
                        job_id = %reservation.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        "Boot: Cron reservation has no owning job and remains quarantined"
                    );
                    continue;
                }
                Err(error) => {
                    error!(
                        job_id = %reservation.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        error = %error,
                        "Boot: failed to load interrupted Cron job; reservation remains non-terminal"
                    );
                    continue;
                }
            };
            let mut job = match cron_job_from_row(row) {
                Ok(job) => job,
                Err(error) => {
                    error!(
                        job_id = %reservation.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        error = %error,
                        "Boot: invalid Cron job keeps its exact reservation quarantined"
                    );
                    continue;
                }
            };
            if let Some(revision) = reservation.schedule_revision {
                job.schedule_revision = revision;
            }
            let gate = self.job_gate(&job.cron_job_id);
            let _job_guard = gate.lock().await;

            if self.active_runs.contains_key(&reservation.cron_job_run_id) {
                continue;
            }
            let recovered = self.recover_exact_reserved_run(&job, &reservation).await;
            let Some((status, result_error)) = recovered else {
                continue;
            };

            self.emit_persisted_job_updated_for(&job).await;
            self.emit_job_executed_for(&job, status, result_error.as_deref())
                .await;
        }
    }

    fn reattach_reserved_receipt_worker(
        &self,
        job: &CronJob,
        reservation: &nomifun_db::models::CronRunReservationRow,
        conversation_id: &str,
    ) {
        if self.active_runs.contains_key(&reservation.cron_job_run_id) {
            return;
        }
        let service = self.clone();
        let job = job.clone();
        let run_id = reservation.cron_job_run_id.clone();
        let planned_at = reservation.planned_at_ms;
        let scheduled = reservation.trigger_kind == "scheduled";
        let conversation_id = conversation_id.to_owned();
        let active_run = self.mark_run_active(&job.cron_job_id, &run_id);
        let task = tokio::spawn(async move {
            let _active_run = active_run;
            let result = service
                .executor
                .observe_reserved_turn(&job, &run_id, &conversation_id)
                .await;
            if scheduled {
                if let Some(planned_at) = planned_at {
                    service
                        .handle_execution_result(
                            job,
                            &run_id,
                            planned_at,
                            result,
                            Some(&conversation_id),
                        )
                        .await;
                }
            } else {
                service
                    .handle_run_now_result(&job, &run_id, result, Some(&conversation_id))
                    .await;
            }
        });
        self.register_background_task(task);
    }

    async fn recover_exact_reserved_run(
        &self,
        job: &CronJob,
        reservation: &nomifun_db::models::CronRunReservationRow,
    ) -> Option<(&'static str, Option<String>)> {
        if reservation.cancel_requested_at_ms.is_some() {
            if let Some(session) = reservation.conversation_id.as_deref() {
                if let Err(error) = self
                    .executor
                    .cancel_reserved_turn(
                        &job.user_id,
                        &reservation.cron_job_id,
                        &reservation.cron_job_run_id,
                        session,
                    )
                    .await
                {
                    warn!(run_id=%reservation.cron_job_run_id,%error,"exact cancelled Cron owner remains unconfirmed");
                    return None;
                }
            } else {
                return self
                    .finalize_run_once(
                        job,
                        &reservation.cron_job_run_id,
                        "cancelled",
                        None,
                        None,
                        cancelled_run_projection(),
                    )
                    .await
                    .then_some(("cancelled", None));
            }
        }
        let Some(conversation_id) = reservation.conversation_id.as_deref() else {
            // Current-protocol execution attaches its Conversation before the
            // first send call. An unattached `pending` row therefore proves
            // that no Conversation claim was possible.
            let message = "cron execution was interrupted before its exact Conversation was attached; automatic redrive is forbidden";
            return self
                .finalize_run_once(
                    job,
                    &reservation.cron_job_run_id,
                    "error",
                    None,
                    Some(message),
                    error_run_projection(message),
                )
                .await
                .then(|| ("error", Some(message.to_owned())));
        };

        let mut state = match self
            .executor
            .public_turn_delivery_state(&job.user_id, conversation_id, &reservation.cron_job_run_id)
            .await
        {
            Ok(state) => state,
            Err(error) => {
                warn!(
                    job_id = %job.cron_job_id,
                    run_id = %reservation.cron_job_run_id,
                    conversation_id,
                    error = %error,
                    "Boot: exact Conversation receipt cannot be read; Cron reservation remains non-terminal"
                );
                return None;
            }
        };

        if matches!(state, CronTurnReceiptState::Accepted { .. }) {
            match self
                .executor
                .reconcile_accepted_turn_on_boot(
                    &job.user_id,
                    conversation_id,
                    &reservation.cron_job_run_id,
                )
                .await
            {
                Ok(CronTurnReconciliation::ReconciledOrTerminalReRead) => {
                    state = match self
                        .executor
                        .public_turn_delivery_state(
                            &job.user_id,
                            conversation_id,
                            &reservation.cron_job_run_id,
                        )
                        .await
                    {
                        Ok(state) => state,
                        Err(error) => {
                            warn!(
                                job_id = %job.cron_job_id,
                                run_id = %reservation.cron_job_run_id,
                                conversation_id,
                                error = %error,
                                "Boot: reconciled Conversation receipt cannot be re-read; Cron remains non-terminal"
                            );
                            return None;
                        }
                    };
                }
                Ok(CronTurnReconciliation::LiveExactOwnerWait) => {
                    self.reattach_reserved_receipt_worker(job, reservation, conversation_id);
                    return None;
                }
                Ok(
                    CronTurnReconciliation::ExternalProofRequiredFailClosed
                    | CronTurnReconciliation::StaleConflict,
                ) => {
                    warn!(
                        job_id = %job.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        conversation_id,
                        "Boot: accepted Conversation turn lacks exact terminal proof; Cron remains quarantined"
                    );
                    return None;
                }
                Err(error) => {
                    warn!(
                        job_id = %job.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        conversation_id,
                        error = %error,
                        "Boot: accepted Conversation turn reconciliation failed; Cron remains quarantined"
                    );
                    return None;
                }
            }
        }

        match state {
            CronTurnReceiptState::Missing => {
                if reservation.cancel_requested_at_ms.is_some() {
                    return self
                        .finalize_run_once(
                            job,
                            &reservation.cron_job_run_id,
                            "cancelled",
                            Some(conversation_id),
                            None,
                            cancelled_run_projection(),
                        )
                        .await
                        .then_some(("cancelled", None));
                }
                // Attachment precedes the send call. Missing receipt after a
                // process restart is exact proof that the receiver never
                // accepted this occurrence.
                let message = "cron execution was interrupted before its exact Conversation turn was accepted; automatic redrive is forbidden";
                self.finalize_run_once(
                    job,
                    &reservation.cron_job_run_id,
                    "error",
                    Some(conversation_id),
                    Some(message),
                    error_run_projection(message),
                )
                .await
                .then(|| ("error", Some(message.to_owned())))
            }
            CronTurnReceiptState::Suspended(_) => {
                self.reattach_reserved_receipt_worker(job, reservation, conversation_id);
                None
            }
            CronTurnReceiptState::Accepted { .. } => {
                self.reattach_reserved_receipt_worker(job, reservation, conversation_id);
                None
            }
            CronTurnReceiptState::Completed(delivery) => match delivery.disposition() {
                Some(crate::session_port::CronTurnDisposition::Cancelled) => self
                    .finalize_run_once(
                        job,
                        &reservation.cron_job_run_id,
                        "cancelled",
                        Some(conversation_id),
                        None,
                        cancelled_run_projection(),
                    )
                    .await
                    .then_some(("cancelled", None)),
                Some(crate::session_port::CronTurnDisposition::InputRequired) => {
                    let projection = self
                        .input_handoff_run_projection(job, conversation_id)
                        .await?;
                    self.finalize_run_once(
                        job,
                        &reservation.cron_job_run_id,
                        "input_required",
                        Some(conversation_id),
                        None,
                        projection,
                    )
                    .await
                    .then_some(("input_required", None))
                }
                Some(crate::session_port::CronTurnDisposition::Delivered) => {
                    let (status, result_error, projection) =
                        match self.success_run_projection(job, conversation_id).await {
                            Ok(projection) => ("ok", None, projection),
                            Err(message) => {
                                let projection = error_run_projection(&message);
                                ("error", Some(message), projection)
                            }
                        };
                    self.finalize_run_once(
                        job,
                        &reservation.cron_job_run_id,
                        status,
                        Some(conversation_id),
                        result_error.as_deref(),
                        projection,
                    )
                    .await
                    .then_some((status, result_error))
                }
                Some(crate::session_port::CronTurnDisposition::Failed(failure)) => self
                    .finalize_run_once(
                        job,
                        &reservation.cron_job_run_id,
                        "error",
                        Some(conversation_id),
                        None,
                        CronJobRunProjection {
                            last_run_at: Some(now_ms()),
                            last_status: Some("error".into()),
                            last_error: Some(None),
                            increment_run_count: true,
                            ..Default::default()
                        },
                    )
                    .await
                    .then(|| ("error", Some(failure.message))),
                None => {
                    warn!(
                        job_id = %job.cron_job_id,
                        run_id = %reservation.cron_job_run_id,
                        conversation_id,
                        "Boot: completed Conversation receipt has no durable result; Cron remains quarantined"
                    );
                    None
                }
            },
        }
    }

    /// Reconcile the filesystem side of the cron aggregate before scheduling.
    ///
    /// Keep a skill directory only when its live cron aggregate still belongs
    /// to the installation owner. Remove secondary-user and orphan directories
    /// so filesystem state cannot outlive the v3 ownership boundary.
    async fn reconcile_skill_files(&self) {
        match self
            .repo
            .list_all(self.authoritative_user_id.as_ref())
            .await
        {
            Ok(rows) => {
                for row in rows {
                    let Some(content) = row
                        .skill_content
                        .as_deref()
                        .filter(|content| !content.trim().is_empty())
                    else {
                        continue;
                    };
                    if let Err(error) =
                        write_raw_skill_file(&self.data_dir, &row.cron_job_id, content).await
                    {
                        warn!(
                            job_id = %row.cron_job_id,
                            error = %error,
                            "Failed to rebuild generated cron SKILL.md from canonical database content"
                        );
                    }
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "Could not rebuild generated cron skill files from database"
                );
            }
        }

        let root = self.data_dir.join(CRON_SKILLS_REL_DIR);
        let mut entries = match tokio::fs::read_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                warn!(path = %root.display(), error = %error, "Failed to scan cron skill directory");
                return;
            }
        };

        let mut removed = 0u32;
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    warn!(path = %root.display(), error = %error, "Failed while scanning cron skill directory");
                    break;
                }
            };
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(job_id) = CronJobId::parse(&name) else {
                match tokio::fs::remove_dir_all(entry.path()).await {
                    Ok(()) => removed += 1,
                    Err(error) => {
                        warn!(path = %entry.path().display(), error = %error, "Failed to remove invalid cron skill directory");
                    }
                }
                continue;
            };

            let retain = match self
                .repo
                .get_by_cron_job_id_for_scheduler(job_id.as_str())
                .await
            {
                Ok(Some(row)) => {
                    self.execution_authority(&row.user_id).controls_host()
                        && row
                            .skill_content
                            .as_deref()
                            .is_some_and(|content| !content.trim().is_empty())
                }
                Ok(None) => false,
                Err(error) => {
                    warn!(job_id = %job_id, error = %error, "Could not verify cron skill ownership; retaining directory");
                    continue;
                }
            };
            if retain {
                continue;
            }
            match delete_skill_file(&self.data_dir, job_id.as_str()).await {
                Ok(()) => removed += 1,
                Err(error) => {
                    warn!(job_id = %job_id, error = %error, "Failed to remove unauthorized cron skill directory");
                }
            }
        }

        if removed > 0 {
            info!(
                removed,
                "Removed unauthorized or orphan cron skill directories"
            );
        }
    }

    /// Production timer entry point. The process-local installation generation
    /// is checked before every database await and again immediately before the
    /// durable claim. Resume, disable and reschedule revoke it synchronously.
    pub async fn tick_occurrence_with_generation(
        &self,
        expected_user_id: &str,
        job_id: &str,
        expected_schedule_revision: i64,
        planned_at_ms: i64,
        generation: u64,
    ) {
        let expected_user_id = match validate_cron_user_id(expected_user_id) {
            Ok(user_id) => user_id,
            Err(error) => {
                error!(job_id, error = %error, "Tick: invalid timer owner");
                return;
            }
        };
        let job_id = match validate_cron_job_id(job_id) {
            Ok(job_id) => job_id,
            Err(error) => {
                error!(job_id, error = %error, "Tick: invalid cron job id");
                return;
            }
        };
        let gate = self.job_gate(&job_id);
        let job_guard = gate.lock().await;
        if !self
            .scheduler
            .is_current_generation(&job_id, expected_user_id, generation)
        {
            info!(
                job_id,
                generation,
                planned_at_ms,
                "Tick: revoked timer generation absorbed before database lookup"
            );
            return;
        }
        let row = match self.repo.get_by_cron_job_id_for_scheduler(&job_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                warn!(job_id, "Tick: job not found, cancelling timer");
                self.scheduler
                    .cancel_generation(&job_id, expected_user_id, generation);
                return;
            }
            Err(e) => {
                error!(job_id, error = %e, "Tick: failed to load job");
                self.scheduler.retry_current_occurrence(
                    &job_id,
                    expected_user_id,
                    expected_schedule_revision,
                    planned_at_ms,
                    generation,
                );
                return;
            }
        };

        if row.user_id != expected_user_id {
            warn!(
                job_id,
                expected_user_id,
                actual_user_id = %row.user_id,
                "Tick: timer owner no longer matches persisted job; cancelling stale timer"
            );
            self.scheduler
                .cancel_generation(&job_id, expected_user_id, generation);
            return;
        }
        if row.schedule_revision != expected_schedule_revision {
            info!(
                job_id,
                expected_schedule_revision,
                actual_schedule_revision = row.schedule_revision,
                planned_at_ms,
                "Tick: stale schedule callback absorbed"
            );
            return;
        }

        let job = match cron_job_from_row(row) {
            Ok(j) => j,
            Err(e) => {
                error!(job_id, error = %e, "Tick: failed to parse job");
                self.scheduler
                    .cancel_generation(&job_id, expected_user_id, generation);
                return;
            }
        };

        if job.next_run_at != Some(planned_at_ms) {
            self.scheduler.schedule_job(&job);
            return;
        }

        if !job.enabled {
            info!(job_id, "Tick: job disabled, skipping");
            self.scheduler
                .cancel_generation(&job_id, expected_user_id, generation);
            return;
        }

        // The row read above is diagnostic only. This second local fence and
        // the repository's INSERT..SELECT CAS are the actual admission
        // authority; no model/runtime side effect occurs before both succeed.
        if !self
            .scheduler
            .is_current_generation(&job_id, expected_user_id, generation)
        {
            info!(
                job_id,
                generation,
                planned_at_ms,
                "Tick: timer was revoked before durable occurrence claim"
            );
            return;
        }

        let operation_key = format!(
            "cron:scheduled:{}:{}:{}",
            job.cron_job_id, expected_schedule_revision, planned_at_ms
        );
        let request_fingerprint = format!(
            "scheduled:v1:{}:{}:{}",
            job.cron_job_id, expected_schedule_revision, planned_at_ms
        );
        let claim_now = now_ms();
        let successor = compute_occurrence_successor(&job.schedule, planned_at_ms, claim_now);
        let reservation = match self
            .repo
            .reserve_run(
                &job.user_id,
                &ReserveCronRunParams {
                    cron_job_run_id: CronJobRunId::new().into_string(),
                    cron_job_id: job.cron_job_id.clone(),
                    trigger_kind: "scheduled".to_owned(),
                    operation_key,
                    request_fingerprint,
                    schedule_revision: Some(expected_schedule_revision),
                    planned_at_ms: Some(planned_at_ms),
                    next_run_at: successor,
                    disable: successor.is_none(),
                    now: claim_now,
                },
            )
            .await
        {
            Ok((reservation, true)) => reservation,
            Ok((reservation, false)) => {
                info!(
                    job_id,
                    run_id = %reservation.cron_job_run_id,
                    status = %reservation.status,
                    "Tick: durable occurrence replay absorbed"
                );
                self.restore_timer_from_authoritative_row(expected_user_id, &job_id, None)
                    .await;
                self.scheduler.retry_current_occurrence(
                    &job_id,
                    expected_user_id,
                    expected_schedule_revision,
                    planned_at_ms,
                    generation,
                );
                return;
            }
            Err(error) => {
                error!(
                    job_id,
                    expected_schedule_revision,
                    planned_at_ms,
                    error = %error,
                    "Tick: failed to reserve durable occurrence"
                );
                self.scheduler.retry_current_occurrence(
                    &job_id,
                    expected_user_id,
                    expected_schedule_revision,
                    planned_at_ms,
                    generation,
                );
                return;
            }
        };

        // `reserve_run` is a durable authority proof, but it is an await point.
        // Resume may synchronously revoke the process-local installation while
        // that transaction is in flight. Re-prove both the local generation
        // and the exact persisted occurrence before the first executor side
        // effect.
        let admission_failure = {
            match self.repo.get_by_cron_job_id_for_scheduler(&job_id).await {
                Ok(Some(current))
                    if current.user_id == expected_user_id
                        && current.schedule_revision == expected_schedule_revision
                        && (current.enabled
                            || (successor.is_none() && current.next_run_at.is_none())) =>
                {
                    None
                }
                Ok(Some(_)) => Some(
                    "persisted schedule changed after durable reservation and before execution",
                ),
                Ok(None) => Some("cron job disappeared after durable reservation"),
                Err(read_error) => {
                    error!(
                        job_id,
                        run_id = %reservation.cron_job_run_id,
                        error = %read_error,
                        "Tick: failed post-reservation authority read"
                    );
                    Some("post-reservation schedule authority could not be verified")
                }
            }
        };
        if let Some(reason) = admission_failure {
            warn!(
                job_id,
                run_id = %reservation.cron_job_run_id,
                expected_schedule_revision,
                planned_at_ms,
                reason,
                "Tick: reserved occurrence was revoked before executor admission"
            );
            let _ = self
                .finalize_run_once(
                    &job,
                    &reservation.cron_job_run_id,
                    "skipped",
                    job.conversation_id.as_deref(),
                    Some(reason),
                    CronJobRunProjection::default(),
                )
                .await;
            self.scheduler.retry_current_occurrence(
                &job_id,
                expected_user_id,
                expected_schedule_revision,
                planned_at_ms,
                generation,
            );
            return;
        }

        let active_run = self.scheduler.commit_if_current_generation(
            &job_id,
            expected_user_id,
            generation,
            || self.mark_run_active(&job_id, &reservation.cron_job_run_id),
        );
        let Some(_active_run) = active_run else {
            self.finalize_run_once(
                &job,
                &reservation.cron_job_run_id,
                "skipped",
                None,
                Some("timer revoked before exact run ownership was fixed"),
                CronJobRunProjection::default(),
            )
            .await;
            return;
        };
        self.restore_timer_from_authoritative_row(expected_user_id, &job_id, None)
            .await;
        self.scheduler.retry_current_occurrence(
            &job_id,
            expected_user_id,
            expected_schedule_revision,
            planned_at_ms,
            generation,
        );
        drop(job_guard);
        // Resolve (and, for new-conversation jobs, create) the exact target
        // before model execution, then durably attach it to this occurrence.
        // Boot recovery must never guess a Conversation from mutable job state:
        // the reservation's immutable run id + attached conversation id are
        // the only coordinates of its exact delivery receipt.
        let prepared = match self
            .executor
            .prepare_run_now(&job, &reservation.cron_job_run_id)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let failure = crate::executor::preparation_context(
                    crate::executor::domain_failure(&error),
                    &job,
                    None,
                );
                self.handle_execution_result(
                    job,
                    &reservation.cron_job_run_id,
                    planned_at_ms,
                    ExecutionResult::Error { failure },
                    None,
                )
                .await;
                return;
            }
        };
        let conversation_id = prepared.conversation_id.clone();
        if let Err(error) = self
            .attach_prepared_session(&job, &reservation.cron_job_run_id, &conversation_id)
            .await
        {
            self.handle_execution_result(
                job,
                &reservation.cron_job_run_id,
                planned_at_ms,
                ExecutionResult::Error {
                    failure: crate::executor::domain_failure(&CronError::Database(error)),
                },
                Some(&conversation_id),
            )
            .await;
            return;
        }
        let resource_retirement = prepared.resource_retirement.clone();
        let result = self.executor.execute_prepared(&job, prepared).await;
        self.complete_prepared_run(
            job,
            reservation.cron_job_run_id,
            Some(planned_at_ms),
            conversation_id,
            result,
            resource_retirement,
        )
        .await;
    }

    pub async fn handle_system_resume(&self) {
        // This must be the first operation in the resume path. Timer futures
        // can be released by the OS before the resume event is delivered; their
        // already-dispatched callbacks carry generations that are now revoked.
        self.scheduler.cancel_all();
        let rows = match self.repo.list_enabled_for_scheduler().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "Resume: failed to load enabled jobs");
                return;
            }
        };

        for row in rows {
            let db_id = row.id;
            let raw_cron_job_id = row.cron_job_id.clone();
            let cron_job_id = match validate_cron_job_id(&raw_cron_job_id) {
                Ok(cron_job_id) => cron_job_id,
                Err(error) => {
                    error!(
                        db_id,
                        cron_job_id = %raw_cron_job_id,
                        error = %error,
                        "Resume: invalid cron job id"
                    );
                    continue;
                }
            };
            let gate = self.job_gate(&cron_job_id);
            let _job_guard = gate.lock().await;
            // The enabled list is a work list only. Reload after taking the
            // gate so a concurrent user update/delete cannot be overwritten by
            // a stale resume snapshot or stale timer installation.
            let current_row = match self
                .repo
                .get_by_cron_job_id_for_scheduler(&cron_job_id)
                .await
            {
                Ok(Some(current_row)) => current_row,
                Ok(None) => continue,
                Err(error) => {
                    error!(
                        db_id,
                        cron_job_id = %cron_job_id,
                        error = %error,
                        "Resume: failed to reload authoritative cron job"
                    );
                    continue;
                }
            };
            let job = match cron_job_from_row(current_row) {
                Ok(j) => j,
                Err(e) => {
                    error!(
                        db_id,
                        cron_job_id = %raw_cron_job_id,
                        error = %e,
                        "Resume: failed to parse job"
                    );
                    continue;
                }
            };
            if !job.enabled {
                self.scheduler.schedule_job(&job);
                continue;
            }
            let now = now_ms();

            if let Some(next_run) = job.next_run_at
                && next_run < now
            {
                info!(
                    job_id = %job.cron_job_id,
                    conversation_id = ?job.conversation_id,
                    "Resume: missed job detected, marking missed without auto-execution"
                );
                let operation_key = format!(
                    "cron:scheduled:{}:{}:{}",
                    job.cron_job_id, job.schedule_revision, next_run
                );
                let request_fingerprint = format!(
                    "scheduled:v1:{}:{}:{}",
                    job.cron_job_id, job.schedule_revision, next_run
                );
                let reservation = match self
                    .repo
                    .reserve_run(
                        &job.user_id,
                        &ReserveCronRunParams {
                            cron_job_run_id: CronJobRunId::new().into_string(),
                            cron_job_id: job.cron_job_id.clone(),
                            trigger_kind: "scheduled".to_owned(),
                            operation_key,
                            request_fingerprint,
                            schedule_revision: Some(job.schedule_revision),
                            planned_at_ms: Some(next_run),
                            next_run_at: compute_occurrence_successor(&job.schedule, next_run, now),
                            disable: compute_occurrence_successor(&job.schedule, next_run, now)
                                .is_none(),
                            now,
                        },
                    )
                    .await
                {
                    Ok((reservation, true)) => reservation,
                    Ok((_reservation, false)) => {
                        self.restore_timer_from_authoritative_row(
                            &job.user_id,
                            &job.cron_job_id,
                            None,
                        )
                        .await;
                        continue;
                    }
                    Err(error) => {
                        error!(
                            job_id = %job.cron_job_id,
                            error = %error,
                            "Resume: failed to reserve missed occurrence"
                        );
                        continue;
                    }
                };
                if self
                    .finalize_run_once(
                        &job,
                        &reservation.cron_job_run_id,
                        "missed",
                        job.conversation_id.as_deref(),
                        None,
                        missed_run_projection(),
                    )
                    .await
                {
                    self.insert_missed_job_tips(&job).await;
                    self.emit_persisted_job_updated_for(&job).await;
                    self.emit_job_executed_for(&job, "missed", None).await;
                }
                self.restore_timer_from_authoritative_row(&job.user_id, &job.cron_job_id, None)
                    .await;
                continue;
            }

            self.scheduler.reschedule_job(&job);
        }

        info!("System resume: all cron timers rescheduled");
    }

    /// Stop only this immutable occurrence. Persist and signal outside the
    /// ordinary job FIFO: a stalled preparation/control cannot delay Stop.
    /// The retained host task keeps responsibility when the HTTP waiter leaves.
    pub async fn cancel_run(
        &self,
        user_id: &str,
        job_id: &str,
        run_id: &str,
    ) -> Result<CronRunCancellationReceipt, CronError> {
        let owner = validate_cron_user_id(user_id)?.to_owned();
        let job = validate_cron_job_id(job_id)?;
        let run = CronJobRunId::parse(run_id)
            .map_err(|error| {
                CronError::App(AppError::BadRequest(format!(
                    "invalid cron run id: {error}"
                )))
            })?
            .into_string();
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = service.cancel_run_owned(&owner, &job, &run).await;
            let _ = sender.send(result);
        });
        self.register_background_task(task);
        receiver.await.map_err(|_| {
            CronError::OutcomeUnknown(
                "exact Cron cancellation owner ended without a receipt".into(),
            )
        })?
    }

    async fn cancel_run_owned(
        &self,
        owner: &str,
        job_id: &str,
        run_id: &str,
    ) -> Result<CronRunCancellationReceipt, CronError> {
        let reservation = self
            .repo
            .request_run_cancellation(owner, job_id, run_id, now_ms())
            .await?;
        if reservation.status != "reserved" {
            // Preserve an earlier delivered/error outcome. A terminal Cron
            // summary alone is not proof that all native cleanup has finished.
            return Ok(CronRunCancellationReceipt {
                accepted: true,
                execution_stopped: reservation.conversation_id.is_none(),
                conversation_id: reservation.conversation_id,
            });
        }
        let execution_stopped = match reservation.conversation_id.as_deref() {
            Some(session) => self
                .executor
                .cancel_reserved_turn(owner, job_id, run_id, session)
                .await
                .map(|receipt| receipt.execution_stopped),
            None => Ok(true), // The same durable writer now fences attachment.
        };
        // Settlement/advancement uses its original reservation source. It may
        // wait for ordinary job mutations; the cancellation fence and exact
        // native signal above never wait for that gate.
        let service = self.clone();
        let owner = owner.to_owned();
        let job_id = job_id.to_owned();
        let run_id = run_id.to_owned();
        let task = tokio::spawn(async move {
            let gate = service.job_gate(&job_id);
            let _guard = gate.lock().await;
            let settle = async {
                let Some(reservation) = service
                    .repo
                    .get_run_reservation(&owner, &job_id, &run_id)
                    .await?
                else {
                    return Ok::<(), CronError>(());
                };
                if reservation.status != "reserved" || reservation.cancel_requested_at_ms.is_none()
                {
                    return Ok(());
                }
                let Some(row) = service.repo.get_by_cron_job_id(&owner, &job_id).await? else {
                    return service
                        .settle_cancelled_run_without_job(&owner, &reservation)
                        .await;
                };
                let mut job = cron_job_from_row(row)?;
                // A subsequent job edit must never grant this old occurrence
                // the current configuration's schedule advancement authority.
                if let Some(revision) = reservation.schedule_revision {
                    job.schedule_revision = revision;
                }
                if let Some((status, result_error)) =
                    service.recover_exact_reserved_run(&job, &reservation).await
                {
                    service.emit_persisted_job_updated_for(&job).await;
                    service
                        .emit_job_executed_for(&job, status, result_error.as_deref())
                        .await;
                }
                Ok(())
            }
            .await;
            if let Err(error) = settle {
                warn!(%run_id,%error,"cancelled Cron settlement retains its original reservation for retry");
            }
        });
        self.register_background_task(task);
        Ok(CronRunCancellationReceipt {
            accepted: true,
            execution_stopped: execution_stopped?,
            conversation_id: reservation.conversation_id,
        })
    }

    async fn settle_cancelled_run_without_job(
        &self,
        owner: &str,
        reservation: &nomifun_db::CronRunReservationRow,
    ) -> Result<(), CronError> {
        let Some(session) = reservation.conversation_id.as_deref() else {
            return Err(CronError::OutcomeUnknown(
                "a missing job cannot authorize its unattached reservation".into(),
            ));
        };
        self.executor
            .cancel_reserved_turn(
                owner,
                &reservation.cron_job_id,
                &reservation.cron_job_run_id,
                session,
            )
            .await?;
        let state = self
            .executor
            .public_turn_delivery_state(owner, session, &reservation.cron_job_run_id)
            .await?;
        let (status, error) = match state {
            CronTurnReceiptState::Missing => ("cancelled", None),
            CronTurnReceiptState::Completed(delivery) => match delivery.disposition() {
                Some(crate::session_port::CronTurnDisposition::Cancelled) => ("cancelled", None),
                Some(crate::session_port::CronTurnDisposition::Delivered) => ("ok", None),
                Some(crate::session_port::CronTurnDisposition::InputRequired) => {
                    ("input_required", None)
                }
                Some(crate::session_port::CronTurnDisposition::Failed(_)) => ("error", None),
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        self.repo
            .finalize_run_with_job_projection(
                owner,
                &FinalizeCronRunParams {
                    cron_job_run_id: reservation.cron_job_run_id.clone(),
                    status: status.into(),
                    conversation_id: Some(session.into()),
                    result_error: error,
                    domain_failure_json: None,
                    now: now_ms(),
                    last_run_at: None,
                    last_status: None,
                    last_error: None,
                    increment_run_count: false,
                },
            )
            .await?;
        Ok(())
    }

    pub async fn run_now(
        &self,
        user_id: &str,
        job_id: &str,
        operation_id: &str,
    ) -> Result<RunNowResponse, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        if operation_id.is_empty()
            || operation_id.len() > 256
            || !operation_id
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(CronError::App(AppError::BadRequest(
                "cron run-now operation identity must contain 1..=256 visible ASCII bytes"
                    .to_owned(),
            )));
        }
        let gate = self.job_gate(&job_id);
        let job_guard = gate.lock().await;
        let row = self
            .repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        let job = cron_job_from_row(row)?;

        let operation_key = format!("cron:run-now:{user_id}:{operation_id}");
        let request_fingerprint = format!("run-now:v1:{user_id}:{}", job.cron_job_id);
        let (reservation, inserted) = self
            .repo
            .reserve_run(
                user_id,
                &ReserveCronRunParams {
                    cron_job_run_id: CronJobRunId::new().into_string(),
                    cron_job_id: job.cron_job_id.clone(),
                    trigger_kind: "run_now".to_owned(),
                    operation_key,
                    request_fingerprint,
                    schedule_revision: None,
                    planned_at_ms: None,
                    next_run_at: None,
                    disable: false,
                    now: now_ms(),
                },
            )
            .await?;
        if !inserted {
            if let Some(conversation_id) = reservation.conversation_id {
                validate_conversation_id(&conversation_id)?;
                return Ok(RunNowResponse { conversation_id });
            }
            let message = reservation.result_error.unwrap_or_else(|| {
                "cron run-now was already admitted without a recoverable conversation; automatic redrive is forbidden".to_owned()
            });
            if reservation.status == "reserved" {
                let _ = self
                    .finalize_run_once(
                        &job,
                        &reservation.cron_job_run_id,
                        "error",
                        None,
                        Some(&message),
                        CronJobRunProjection::default(),
                    )
                    .await;
            }
            return Err(CronError::App(AppError::Conflict(message)));
        }

        let prepared = match self
            .executor
            .prepare_run_now(&job, &reservation.cron_job_run_id)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let failure = crate::executor::preparation_context(
                    crate::executor::domain_failure(&error),
                    &job,
                    None,
                );
                let message = failure.message.clone();
                let _ = self
                    .finalize_run_once(
                        &job,
                        &reservation.cron_job_run_id,
                        "error",
                        None,
                        Some(&message),
                        typed_error_run_projection(failure),
                    )
                    .await;
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "error", Some(&message))
                    .await;
                return Err(error);
            }
        };
        let conversation_id = prepared.conversation_id.clone();
        validate_conversation_id(&conversation_id)?;
        if let Err(error) = self
            .attach_prepared_session(&job, &reservation.cron_job_run_id, &conversation_id)
            .await
        {
            let message = error.to_string();
            let _ = self
                .finalize_run_once(
                    &job,
                    &reservation.cron_job_run_id,
                    "error",
                    None,
                    Some(&message),
                    CronJobRunProjection::default(),
                )
                .await;
            return Err(error.into());
        }
        let service = self.clone();
        let run_id = reservation.cron_job_run_id;
        let resource_retirement = prepared.resource_retirement.clone();
        let prepared_conversation_id = conversation_id.clone();
        let active_run = self.mark_run_active(&job.cron_job_id, &run_id);
        drop(job_guard);

        let task = tokio::spawn(async move {
            let _active_run = active_run;
            let result = service.executor.execute_prepared(&job, prepared).await;
            service
                .complete_prepared_run(
                    job,
                    run_id,
                    None,
                    prepared_conversation_id,
                    result,
                    resource_retirement,
                )
                .await;
        });
        self.register_background_task(task);

        // The executor returns the canonical conversation entity ID unchanged.
        Ok(RunNowResponse { conversation_id })
    }

    // -----------------------------------------------------------------------
    // Skill management
    // -----------------------------------------------------------------------

    pub async fn save_skill(
        &self,
        user_id: &str,
        job_id: &str,
        req: SaveCronSkillRequest,
    ) -> Result<(), CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        let row = self
            .repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        // Resolve the aggregate through its owner-scoped repository before the
        // host-capability gate. A caller must not be able to distinguish
        // another user's cron id from a missing id; only an owned model-only
        // job reaches the explicit installation-owner denial below.
        self.require_host_control(user_id)?;

        validate_skill_body_content(&req.content)?;
        let job = cron_job_from_row(row)?;
        let canonical_content = canonical_skill_content(&job, &req.content)?;

        let params = UpdateCronJobParams {
            skill_content: Some(Some(canonical_content.clone())),
            ..Default::default()
        };
        self.repo.update(user_id, &job_id, &params).await?;
        if let Err(err) = write_raw_skill_file(&self.data_dir, &job_id, &canonical_content).await {
            warn!(
                job_id,
                error = %err,
                "Cron skill metadata saved; generated SKILL.md is stale and will be rebuilt before execution"
            );
            return Err(CronError::InvalidSkillContent(format!(
                "skill metadata saved, but generated SKILL.md could not be written: {err}"
            )));
        }
        info!(job_id, "Skill content saved");
        Ok(())
    }

    pub async fn has_skill(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<HasSkillResponse, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        let row = self
            .repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        self.require_host_control(user_id)?;

        let has_skill = row
            .skill_content
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty());

        Ok(HasSkillResponse { has_skill })
    }

    pub async fn delete_skill(&self, user_id: &str, job_id: &str) -> Result<(), CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        let job_id = validate_cron_job_id(job_id)?;
        self.repo
            .get_by_cron_job_id(user_id, &job_id)
            .await?
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        self.require_host_control(user_id)?;

        let params = UpdateCronJobParams {
            skill_content: Some(None),
            ..Default::default()
        };
        self.repo.update(user_id, &job_id, &params).await?;
        if let Err(err) = delete_skill_file(&self.data_dir, &job_id).await {
            warn!(
                job_id,
                error = %err,
                "Cron skill metadata deleted; generated skill directory is orphaned and requires cleanup"
            );
            return Err(CronError::InvalidSkillContent(format!(
                "skill metadata deleted, but generated SKILL.md could not be removed: {err}"
            )));
        }

        info!(job_id, "Skill content deleted");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    pub fn to_response(job: &CronJob) -> CronJobResponse {
        cron_job_to_response(job)
    }

    /// Reinstall only the row that is currently authoritative in SQLite after
    /// a timer-bearing mutation fails. The fallback is used solely when the
    /// repair read itself is unavailable.
    async fn restore_timer_from_authoritative_row(
        &self,
        user_id: &str,
        job_id: &str,
        fallback: Option<&CronJob>,
    ) {
        match self.repo.get_by_cron_job_id(user_id, job_id).await {
            Ok(Some(row)) => match cron_job_from_row(row) {
                Ok(job) => self.scheduler.schedule_job(&job),
                Err(parse_error) => {
                    error!(
                        job_id,
                        error = %parse_error,
                        "Failed to parse authoritative cron row while restoring its timer"
                    );
                    self.scheduler.cancel_job_for_owner(job_id, user_id);
                }
            },
            Ok(None) => self.scheduler.cancel_job_for_owner(job_id, user_id),
            Err(read_error) => {
                error!(
                    job_id,
                    error = %read_error,
                    "Failed to reload authoritative cron row while restoring its timer"
                );
                if let Some(job) = fallback {
                    self.scheduler.schedule_job(job);
                }
            }
        }
    }

    async fn attach_prepared_session(
        &self,
        job: &CronJob,
        run: &str,
        session: &str,
    ) -> Result<nomifun_db::CronRunReservationRow, nomifun_db::DbError> {
        if job.execution_mode == ExecutionMode::Existing {
            self.repo
                .attach_continuing_session(&job.user_id, run, session, now_ms())
                .await
        } else {
            self.repo
                .attach_run_conversation(&job.user_id, run, session, now_ms())
                .await
        }
    }

    async fn is_orphan(&self, job: &CronJob) -> bool {
        // NewConversation jobs never depend on an existing conversation —
        // every run materializes a fresh one. They must not be cleaned up
        // based on conversation state.
        if matches!(job.execution_mode, ExecutionMode::NewConversation) {
            return false;
        }

        // Existing-mode jobs can be unbound until their first lazy-binding run.
        let Some(conversation_id) = job.conversation_id.as_deref() else {
            return false;
        };

        if self.executor.busy_guard().is_busy(conversation_id) {
            return false;
        }

        // Only true orphan case: Existing + bound conversation_id, but that
        // conversation has been deleted.
        match self
            .executor
            .get_session_projection(&job.user_id, conversation_id)
            .await
        {
            Ok(session) => {
                session.owner_id != job.user_id
                    || session.agent_session_id.as_ref() != conversation_id
            }
            Err(AppError::NotFound(_)) => true,
            Err(err) => {
                warn!(
                    job_id = %job.cron_job_id,
                    conversation_id,
                    error = %err,
                    "Failed to verify cron conversation during orphan cleanup"
                );
                false
            }
        }
    }

    /// Retain the formal finalizer and its resource handoff together. The
    /// caller observes business settlement without waiting for resource
    /// observation/retry, and dropping that observer cannot lose the handoff
    /// after the durable terminal commit.
    async fn complete_prepared_run(
        &self,
        job: CronJob,
        run_id: String,
        planned_at_ms: Option<i64>,
        conversation_id: String,
        result: ExecutionResult,
        resource_retirement: Option<CronRunResourceRetirement>,
    ) {
        let service = self.clone();
        let (settled, observer) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let request = service
                .finalize_prepared_run(
                    job,
                    run_id,
                    planned_at_ms,
                    conversation_id,
                    result,
                    resource_retirement,
                )
                .await;
            // Seal the original writer's immutable authority before releasing
            // its business observer. A legal subsequent delete may remove
            // both job and run rows; it cannot revoke this accepted handoff.
            let _ = settled.send(());
            if let Some(request) = request {
                service.handoff_finished_run_resources(request).await;
            }
        });
        self.register_background_task(task);
        let _ = observer.await;
    }

    async fn finalize_prepared_run(
        &self,
        job: CronJob,
        run_id: String,
        planned_at_ms: Option<i64>,
        conversation_id: String,
        result: ExecutionResult,
        resource_retirement: Option<CronRunResourceRetirement>,
    ) -> Option<CronRunResourceRetirement> {
        let terminal_mission = matches!(
            &result,
            ExecutionResult::Success { .. }
                | ExecutionResult::Error { .. }
                | ExecutionResult::TurnFailed { .. }
                | ExecutionResult::Cancelled { .. }
        );
        let finalized = if let Some(planned_at) = planned_at_ms {
            self.handle_execution_result(
                job.clone(),
                &run_id,
                planned_at,
                result,
                Some(&conversation_id),
            )
            .await
        } else {
            self.handle_run_now_result(&job, &run_id, result, Some(&conversation_id))
                .await
        };
        finalized.filter(|_| terminal_mission).and_then(|status| {
            resource_retirement
                .filter(|request| {
                    request.owner_id == job.user_id
                        && request.cron_job_id == job.cron_job_id
                        && request.cron_job_run_id == run_id
                        && request.agent_session_id.as_ref() == conversation_id
                })
                .and_then(|request| request.seal_finalized(status))
        })
    }

    async fn handoff_finished_run_resources(&self, request: CronRunResourceRetirement) {
        if request.finalized_status().is_none() {
            return;
        }
        let shutdown = self
            .background_task_registrar
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .map(|registrar| registrar.shutdown_signal())
            .unwrap_or_else(|| Box::pin(std::future::pending()));
        let handoff = async {
            let mut delay = std::time::Duration::from_millis(100);
            loop {
                // The formal writer's live receipt remains authoritative
                // after legal job/run deletion. This retries only the same
                // resource handoff, never preparation or business delivery.
                match self
                    .executor
                    .retire_new_conversation_run_resources(&request)
                    .await
                {
                    Ok(()) => return,
                    Err(error) => warn!(run_id = %request.cron_job_run_id, %error,
                        "Finished Cron run resource handoff remains owned for retry"),
                }
                tokio::time::sleep(delay).await;
                delay = delay
                    .saturating_mul(2)
                    .min(std::time::Duration::from_secs(30));
            }
        };
        tokio::select! {
            biased;
            _ = shutdown => {},
            _ = handoff => {},
        }
    }

    async fn finalize_run_once(
        &self,
        job: &CronJob,
        run_id: &str,
        status: &str,
        conversation_id: Option<&str>,
        result_error: Option<&str>,
        projection: CronJobRunProjection,
    ) -> bool {
        self.finalize_run_outcome(
            job,
            run_id,
            status,
            conversation_id,
            result_error,
            projection,
        )
        .await
            == Some(true)
    }

    /// `Some(false)` preserves an exact AlreadyApplied receipt without
    /// repeating projection/events. `None` grants no retirement authority.
    async fn finalize_run_outcome(
        &self,
        job: &CronJob,
        run_id: &str,
        status: &str,
        conversation_id: Option<&str>,
        _result_error: Option<&str>,
        projection: CronJobRunProjection,
    ) -> Option<bool> {
        match self
            .repo
            .finalize_run_with_job_projection(
                &job.user_id,
                &FinalizeCronRunParams {
                    cron_job_run_id: run_id.to_owned(),
                    status: status.to_owned(),
                    conversation_id: conversation_id.map(str::to_owned),
                    result_error: None,
                    domain_failure_json: projection.domain_failure.as_ref().map(|failure| {
                        serde_json::to_string(failure).expect("domain failure serializes")
                    }),
                    now: now_ms(),
                    last_run_at: projection.last_run_at,
                    last_status: projection.last_status,
                    last_error: projection.last_error,
                    increment_run_count: projection.increment_run_count,
                },
            )
            .await
        {
            Ok(FinalizeCronRunOutcome::Applied) => Some(true),
            Ok(FinalizeCronRunOutcome::AlreadyApplied) => Some(false),
            Ok(FinalizeCronRunOutcome::LegacyProjectionUnknown) => {
                warn!(
                    job_id = %job.cron_job_id,
                    run_id,
                    status,
                    "Refusing to guess whether a legacy Cron run already updated its job projection"
                );
                None
            }
            Err(error) => {
                error!(
                    job_id = %job.cron_job_id,
                    run_id,
                    status,
                    error = %error,
                    "Failed to atomically finalize durable cron run and job projection"
                );
                None
            }
        }
    }

    async fn handle_execution_result(
        &self,
        job: CronJob,
        run_id: &str,
        _expected_planned_at_ms: i64,
        result: ExecutionResult,
        prepared_conversation_id: Option<&str>,
    ) -> Option<&'static str> {
        let gate = self.job_gate(&job.cron_job_id);
        let _job_guard = gate.lock().await;

        match result {
            ExecutionResult::Cancelled { conversation_id } => {
                let finalized = self
                    .finalize_run_outcome(
                        &job,
                        run_id,
                        "cancelled",
                        Some(&conversation_id),
                        None,
                        cancelled_run_projection(),
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "cancelled");
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "cancelled", None).await;
                return Some("cancelled");
            }
            ExecutionResult::InputRequired { conversation_id } => {
                let Some(projection) = self
                    .input_handoff_run_projection(&job, &conversation_id)
                    .await
                else {
                    return None;
                };
                if !self
                    .finalize_run_once(
                        &job,
                        run_id,
                        "input_required",
                        Some(&conversation_id),
                        None,
                        projection,
                    )
                    .await
                {
                    return None;
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "input_required", None)
                    .await;
            }
            ExecutionResult::Success { conversation_id } => {
                let (status, error, projection) =
                    match self.success_run_projection(&job, &conversation_id).await {
                        Ok(projection) => ("ok", None, projection),
                        Err(message) => {
                            let projection = error_run_projection(&message);
                            ("error", Some(message), projection)
                        }
                    };
                let finalized = self
                    .finalize_run_outcome(
                        &job,
                        run_id,
                        status,
                        Some(&conversation_id),
                        error.as_deref(),
                        projection,
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| status);
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, status, error.as_deref())
                    .await;
                return Some(status);
            }
            ExecutionResult::Skipped => {
                if !self
                    .finalize_run_once(
                        &job,
                        run_id,
                        "skipped",
                        job.conversation_id.as_deref(),
                        None,
                        skipped_run_projection(),
                    )
                    .await
                {
                    return None;
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "skipped", None).await;
            }
            ExecutionResult::TurnFailed { conversation_id } => {
                let projection = CronJobRunProjection {
                    last_run_at: Some(now_ms()),
                    last_status: Some("error".into()),
                    last_error: Some(None),
                    increment_run_count: true,
                    ..Default::default()
                };
                let finalized = self
                    .finalize_run_outcome(
                        &job,
                        run_id,
                        "error",
                        Some(&conversation_id),
                        None,
                        projection,
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "error");
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "error", None).await;
                return Some("error");
            }
            ExecutionResult::Error { failure } => {
                let message = failure.message.clone();
                let finalized = self
                    .finalize_run_outcome(
                        &job,
                        run_id,
                        "error",
                        prepared_conversation_id.or(job.conversation_id.as_deref()),
                        Some(&message),
                        typed_error_run_projection(failure),
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "error");
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "error", Some(&message))
                    .await;
                return Some("error");
            }
            ExecutionResult::Quarantined { message } => {
                warn!(
                    job_id = %job.cron_job_id,
                    run_id,
                    reason = %message,
                    "Cron occurrence remains durably reserved because its accepted Conversation turn is not terminal"
                );
            }
        }
        None
    }

    async fn handle_run_now_result(
        &self,
        job: &CronJob,
        run_id: &str,
        result: ExecutionResult,
        prepared_conversation_id: Option<&str>,
    ) -> Option<&'static str> {
        let gate = self.job_gate(&job.cron_job_id);
        let _job_guard = gate.lock().await;
        match result {
            ExecutionResult::Cancelled { conversation_id } => {
                let finalized = self
                    .finalize_run_outcome(
                        job,
                        run_id,
                        "cancelled",
                        Some(&conversation_id),
                        None,
                        cancelled_run_projection(),
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "cancelled");
                }
                self.emit_persisted_job_updated_for(job).await;
                self.emit_job_executed_for(job, "cancelled", None).await;
                return Some("cancelled");
            }
            ExecutionResult::InputRequired { conversation_id } => {
                let Some(projection) = self
                    .input_handoff_run_projection(job, &conversation_id)
                    .await
                else {
                    return None;
                };
                if !self
                    .finalize_run_once(
                        job,
                        run_id,
                        "input_required",
                        Some(&conversation_id),
                        None,
                        projection,
                    )
                    .await
                {
                    return None;
                }
                self.emit_persisted_job_updated_for(job).await;
                self.emit_job_executed_for(job, "input_required", None)
                    .await;
            }
            ExecutionResult::Success { conversation_id } => {
                let (status, error, projection) =
                    match self.success_run_projection(job, &conversation_id).await {
                        Ok(projection) => ("ok", None, projection),
                        Err(message) => {
                            let projection = error_run_projection(&message);
                            ("error", Some(message), projection)
                        }
                    };
                let finalized = self
                    .finalize_run_outcome(
                        job,
                        run_id,
                        status,
                        Some(&conversation_id),
                        error.as_deref(),
                        projection,
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| status);
                }
                self.emit_persisted_job_updated_for(job).await;
                self.emit_job_executed_for(job, status, error.as_deref())
                    .await;
                return Some(status);
            }
            ExecutionResult::TurnFailed { conversation_id } => {
                let projection = CronJobRunProjection {
                    last_run_at: Some(now_ms()),
                    last_status: Some("error".into()),
                    last_error: Some(None),
                    increment_run_count: true,
                    ..Default::default()
                };
                let finalized = self
                    .finalize_run_outcome(
                        &job,
                        run_id,
                        "error",
                        Some(&conversation_id),
                        None,
                        projection,
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "error");
                }
                self.emit_persisted_job_updated_for(&job).await;
                self.emit_job_executed_for(&job, "error", None).await;
                return Some("error");
            }
            ExecutionResult::Error { failure } => {
                let message = failure.message.clone();
                let finalized = self
                    .finalize_run_outcome(
                        job,
                        run_id,
                        "error",
                        prepared_conversation_id.or(job.conversation_id.as_deref()),
                        Some(&message),
                        typed_error_run_projection(failure),
                    )
                    .await;
                if finalized != Some(true) {
                    return finalized.map(|_| "error");
                }
                self.emit_persisted_job_updated_for(job).await;
                self.emit_job_executed_for(job, "error", Some(&message))
                    .await;
                return Some("error");
            }
            ExecutionResult::Skipped => {
                if !self
                    .finalize_run_once(
                        job,
                        run_id,
                        "skipped",
                        job.conversation_id.as_deref(),
                        None,
                        skipped_run_projection(),
                    )
                    .await
                {
                    return None;
                }
                self.emit_persisted_job_updated_for(job).await;
                self.emit_job_executed_for(job, "skipped", None).await;
            }
            ExecutionResult::Quarantined { message } => {
                warn!(
                    job_id = %job.cron_job_id,
                    run_id,
                    reason = %message,
                    "Cron run-now remains durably reserved because its accepted Conversation turn is not terminal"
                );
            }
        }
        None
    }

    async fn input_handoff_run_projection(
        &self,
        job: &CronJob,
        conversation_id: &str,
    ) -> Option<CronJobRunProjection> {
        match self.success_run_projection(job, conversation_id).await {
            Ok(mut projection) => {
                projection.last_status = Some("input_required".into());
                Some(projection)
            }
            Err(error) => {
                warn!(job_id = %job.cron_job_id, error,
                "Cannot settle the exact input handoff projection; retaining its reservation");
                None
            }
        }
    }

    async fn success_run_projection(
        &self,
        _job: &CronJob,
        conversation_id: &str,
    ) -> Result<CronJobRunProjection, String> {
        ConversationId::parse(conversation_id).map_err(|error| error.to_string())?;
        Ok(CronJobRunProjection {
            last_run_at: Some(now_ms()),
            last_status: Some("ok".into()),
            last_error: Some(None),
            increment_run_count: true,
            domain_failure: None,
        })
    }

    async fn insert_missed_job_tips(&self, job: &CronJob) {
        let Some(conversation_id) = job.conversation_id.as_deref() else {
            return;
        };

        let content = format!(
            "Scheduled task \"{}\" was missed while the system was unavailable. It was not run automatically.",
            job.name
        );

        match self
            .executor
            .insert_tips_message(&job.user_id, conversation_id, &content, "warning")
            .await
        {
            Ok(()) => {
                self.emitter.emit_conversation_tips(
                    &job.user_id,
                    conversation_id,
                    &content,
                    "warning",
                );
            }
            Err(err) => {
                warn!(
                    job_id = %job.cron_job_id,
                    conversation_id,
                    error = %err,
                    "Failed to persist missed-job tips message"
                );
            }
        }
    }

    /// Cancel process-local timers, emit lifecycle events, and remove generated
    /// skill files for Cron rows that have already been deleted durably.
    pub async fn delete_jobs_by_agent_session(
        &self,
        user_id: &str,
        agent_session_id: &str,
    ) -> Result<Vec<String>, CronError> {
        let user_id = validate_cron_user_id(user_id)?;
        if ConversationId::try_from(agent_session_id).is_err() {
            return Err(CronError::App(AppError::BadRequest(format!(
                "invalid AgentSession id '{agent_session_id}'"
            ))));
        }
        let job_ids = self
            .repo
            .delete_by_conversation(user_id, agent_session_id)
            .await
            .map_err(CronError::from)?;
        self.cleanup_deleted_jobs(user_id, &job_ids).await;
        Ok(job_ids)
    }

    /// Cancel process-local timers, emit lifecycle events, and remove generated
    /// skill files for Cron rows that have already been deleted durably.
    pub async fn cleanup_deleted_jobs(&self, user_id: &str, job_ids: &[String]) {
        let user_id = match validate_cron_user_id(user_id) {
            Ok(user_id) => user_id,
            Err(error) => {
                error!(error = %error, "refusing cron cleanup for invalid caller");
                return;
            }
        };
        for job_id in job_ids {
            if CronJobId::parse(job_id).is_err() {
                continue;
            }
            let gate = self.job_gate(job_id);
            let _job_guard = gate.lock().await;
            self.scheduler.cancel_job_for_owner(job_id, user_id);
            self.emitter.emit_job_removed(user_id, job_id);
            if let Err(error) = delete_skill_file(&self.data_dir, job_id).await {
                warn!(
                    job_id,
                    error = %error,
                    "Cron row was cascade-deleted; generated skill directory is orphaned and will be retried by startup reconciliation"
                );
            }
        }
    }
}

fn validate_embedded_operation_id(operation_id: &str) -> Result<String, CronError> {
    if operation_id.is_empty()
        || operation_id.len() > 256
        || !operation_id
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(CronError::App(AppError::BadRequest(
            "Cron embedded operation identity must contain 1..=256 visible ASCII bytes".to_owned(),
        )));
    }
    Ok(operation_id.to_owned())
}

fn embedded_create_fingerprint(session_id: &str, command: &CronEmbeddedCreateCommand) -> String {
    serde_json::to_string(&(
        "cron-embedded-create-v1",
        session_id,
        &command.name,
        &command.schedule,
        &command.schedule_description,
        &command.message,
    ))
    .expect("Cron embedded create fingerprint fields are serializable")
}

fn embedded_update_fingerprint(session_id: &str, command: &CronEmbeddedUpdateCommand) -> String {
    serde_json::to_string(&(
        "cron-embedded-update-v1",
        session_id,
        &command.job_id,
        &command.name,
        &command.schedule,
        &command.schedule_description,
        &command.message,
    ))
    .expect("Cron embedded update fingerprint fields are serializable")
}

fn embedded_delete_fingerprint(command: &CronEmbeddedDeleteCommand) -> String {
    serde_json::to_string(&("cron-embedded-delete-v1", &command.job_id))
        .expect("Cron embedded delete fingerprint fields are serializable")
}

fn embedded_error(message: String) -> CronEmbeddedCommandResult {
    CronEmbeddedCommandResult {
        success: false,
        message,
    }
}

fn validate_enabled_next_run(
    schedule: &CronSchedule,
    enabled: bool,
    next: Option<i64>,
) -> Result<(), CronError> {
    if enabled && matches!(schedule, CronSchedule::Every { .. }) && next.is_none() {
        return Err(CronError::InvalidSchedule(
            "every_ms overflows the next run time".into(),
        ));
    }
    Ok(())
}

fn parse_execution_mode(mode: Option<&str>) -> Result<ExecutionMode, CronError> {
    match mode {
        None | Some("existing") => Ok(ExecutionMode::Existing),
        Some(s) => ExecutionMode::from_str(s),
    }
}

fn validate_skill_body_content(content: &str) -> Result<(), CronError> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return Err(CronError::InvalidSkillContent(
            "content must not be empty".into(),
        ));
    }

    let lower = trimmed.to_lowercase();
    for pattern in PLACEHOLDER_PATTERNS {
        if lower.starts_with(pattern) {
            return Err(CronError::InvalidSkillContent(
                "content appears to be placeholder text".into(),
            ));
        }
    }

    Ok(())
}

fn schedule_description(schedule: &CronSchedule) -> Option<&str> {
    match schedule {
        CronSchedule::At { description, .. }
        | CronSchedule::Every { description, .. }
        | CronSchedule::Cron { description, .. } => description.as_deref(),
    }
}

fn canonical_skill_content(job: &CronJob, content: &str) -> Result<String, CronError> {
    if validate_skill_content(content).is_ok() {
        return Ok(content.to_owned());
    }
    let description = job
        .description
        .clone()
        .unwrap_or_else(|| format!("Saved cron skill for {}", job.name));
    let content = build_skill_content(
        &job.name,
        &description,
        content.trim(),
        schedule_description(&job.schedule),
    );
    validate_skill_content(&content)?;
    Ok(content)
}

fn update_params_from_job(job: &CronJob) -> Result<UpdateCronJobParams, CronError> {
    let row = cron_job_to_row(job)?;
    Ok(UpdateCronJobParams {
        name: Some(row.name),
        enabled: Some(row.enabled),
        schedule_revision: Some(row.schedule_revision),
        schedule_kind: Some(row.schedule_kind),
        schedule_value: Some(row.schedule_value),
        schedule_tz: Some(row.schedule_tz),
        schedule_description: Some(row.schedule_description),
        payload_message: Some(row.payload_message),
        execution_mode: Some(row.execution_mode),
        agent_selection: Some(row.agent_selection),
        model_selection: Some(row.model_selection),
        agent_binding: Some(row.agent_binding),
        clear_context_each_run: Some(row.clear_context_each_run),
        configuration_error: Some(row.configuration_error),
        conversation_id: Some(row.conversation_id),
        conversation_title: Some(row.conversation_title),
        skill_content: Some(row.skill_content),
        description: Some(row.description),
        ..Default::default()
    })
}

fn cancelled_run_projection() -> CronJobRunProjection {
    CronJobRunProjection {
        last_run_at: Some(now_ms()),
        last_status: Some("cancelled".into()),
        last_error: Some(None),
        increment_run_count: true,
        domain_failure: None,
    }
}

fn error_run_projection(message: &str) -> CronJobRunProjection {
    typed_error_run_projection(nomifun_api_types::AgentStreamErrorData::legacy(
        message, None,
    ))
}

fn typed_error_run_projection(
    failure: nomifun_api_types::AgentStreamErrorData,
) -> CronJobRunProjection {
    CronJobRunProjection {
        last_run_at: Some(now_ms()),
        last_status: Some("error".into()),
        // The compatibility string is derived on reads from this exact typed
        // failure; it is not another durable copy of the diagnostic.
        last_error: Some(None),
        increment_run_count: true,
        domain_failure: Some(failure),
    }
}

fn skipped_run_projection() -> CronJobRunProjection {
    CronJobRunProjection {
        last_status: Some("skipped".to_owned()),
        ..Default::default()
    }
}

fn missed_run_projection() -> CronJobRunProjection {
    CronJobRunProjection {
        last_status: Some("missed".to_owned()),
        last_error: Some(None),
        ..Default::default()
    }
}

fn cron_run_to_response(row: CronRunHistoryRow) -> Result<CronJobRunResponse, CronError> {
    let cron_job_run_id = CronJobRunId::parse(row.cron_job_run_id).map_err(|error| {
        CronError::Scheduler(format!("invalid persisted cron job run id: {error}"))
    })?;
    let cron_job_id = CronJobId::parse(row.cron_job_id).map_err(|error| {
        CronError::Scheduler(format!(
            "invalid persisted cron job id in run history: {error}"
        ))
    })?;
    Ok(CronJobRunResponse {
        cron_job_run_id: cron_job_run_id.into_string(),
        cron_job_id: cron_job_id.into_string(),
        executed_at_ms: row.executed_at_ms,
        status: row.status,
        conversation_id: row.conversation_id,
        failure: None,
    })
}

fn schedule_from_dto_with_existing_timezone(
    dto: &CronScheduleDto,
    existing: &CronSchedule,
) -> CronSchedule {
    match dto {
        CronScheduleDto::Cron {
            expr,
            tz,
            description,
        } => CronSchedule::Cron {
            expr: expr.clone(),
            tz: tz.clone().or_else(|| match existing {
                CronSchedule::Cron { tz, .. } => tz.clone(),
                _ => None,
            }),
            description: description.clone(),
        },
        _ => schedule_from_dto(dto),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded_mutation_entry() -> Arc<EmbeddedMutationEntry> {
        let (sender, _) = watch::channel(None);
        Arc::new(EmbeddedMutationEntry {
            fingerprint: "test-fingerprint".to_owned(),
            sender,
            completed: AtomicBool::new(false),
            outcome_unknown: AtomicBool::new(false),
        })
    }

    async fn wait_for_owner_unknown(waiter: CronEmbeddedMutationWaiter) -> String {
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter.wait())
            .await
            .expect("abandoned embedded mutation waiter must be bounded")
            .expect_err("abandoned embedded mutation must not report deterministic failure");
        match error {
            CronError::OutcomeUnknown(message) => message,
            other => panic!("unexpected embedded mutation error: {other}"),
        }
    }

    #[tokio::test]
    async fn embedded_mutation_owner_drop_publishes_outcome_unknown() {
        let state = embedded_mutation_entry();
        let waiter = state.waiter();

        drop(EmbeddedMutationOwner::new(state.clone()));

        assert_eq!(
            wait_for_owner_unknown(waiter).await,
            EMBEDDED_MUTATION_OWNER_DROPPED
        );
        assert!(state.completed.load(Ordering::Acquire));
        assert!(
            !state.evictable(),
            "unknown owner-drop receipt must remain absorbing"
        );
    }

    #[tokio::test]
    async fn embedded_mutation_owner_panic_publishes_outcome_unknown() {
        let state = embedded_mutation_entry();
        let waiter = state.waiter();
        let owner = EmbeddedMutationOwner::new(state.clone());

        let task = tokio::spawn(async move {
            let _owner = owner;
            panic!("injected embedded mutation owner panic");
        });
        assert!(task.await.expect_err("owner task must panic").is_panic());

        assert_eq!(
            wait_for_owner_unknown(waiter).await,
            EMBEDDED_MUTATION_OWNER_DROPPED
        );
        assert!(state.completed.load(Ordering::Acquire));
        assert!(
            !state.evictable(),
            "unknown panic receipt must remain absorbing"
        );
    }

    #[tokio::test]
    async fn embedded_mutation_owner_abort_settles_same_operation_replays() {
        let state = embedded_mutation_entry();
        let original = state.waiter();
        // An occupied same-operation replay subscribes to this same retained
        // entry in production; it must receive the identical terminal fact.
        let replay = state.waiter();
        let owner = EmbeddedMutationOwner::new(state.clone());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _owner = owner;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("owner task must start");

        task.abort();
        assert!(
            task.await
                .expect_err("owner task must be aborted")
                .is_cancelled()
        );

        let original = wait_for_owner_unknown(original).await;
        let replay = wait_for_owner_unknown(replay).await;
        assert_eq!(original, EMBEDDED_MUTATION_OWNER_DROPPED);
        assert_eq!(replay, original);
        assert!(state.completed.load(Ordering::Acquire));
        assert!(
            !state.evictable(),
            "unknown abort receipt must remain absorbing"
        );
    }

    #[test]
    fn enabled_interval_overflow_is_rejected_without_rejecting_manual_or_disabled_schedules() {
        let every = CronSchedule::Every {
            every_ms: i64::MAX,
            description: None,
        };
        assert!(matches!(
            validate_enabled_next_run(&every, true, None),
            Err(CronError::InvalidSchedule(_))
        ));
        assert!(validate_enabled_next_run(&every, false, None).is_ok());
        assert!(
            validate_enabled_next_run(
                &CronSchedule::Cron {
                    expr: String::new(),
                    tz: None,
                    description: None
                },
                true,
                None
            )
            .is_ok()
        );
        assert!(
            validate_enabled_next_run(
                &CronSchedule::At {
                    at_ms: 1,
                    description: None
                },
                true,
                Some(1)
            )
            .is_ok()
        );
    }

    // -- validate_skill_body_content -------------------------------------------

    #[test]
    fn validate_skill_empty_content() {
        let err = validate_skill_body_content("").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_whitespace_only() {
        let err = validate_skill_body_content("   \n  ").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_todo() {
        let err = validate_skill_body_content("TODO: fill in later").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_fill_in() {
        let err = validate_skill_body_content("Fill in your instructions here").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_placeholder_replace() {
        let err = validate_skill_body_content("Replace this with your skill").unwrap_err();
        assert!(matches!(err, CronError::InvalidSkillContent(_)));
    }

    #[test]
    fn validate_skill_valid_content() {
        assert!(validate_skill_body_content("---\nname: test\n---\nDo something useful").is_ok());
    }

    #[test]
    fn validate_skill_valid_short() {
        assert!(validate_skill_body_content("Run daily report").is_ok());
    }
}

#[cfg(test)]
#[path = "run_cancellation_tests.rs"]
mod run_cancellation_tests;

#[cfg(test)]
#[path = "run_resource_retirement_tests.rs"]
mod run_resource_retirement_tests;

#[cfg(test)]
mod reserved_receipt_worker_tests {
    use super::*;
    use crate::busy_guard::CronBusyGuard;
    use crate::session_port::{CronTurnDelivery, test_support::ReceiptOnlySessions};
    use nomifun_agent_contracts::{AgentTurnOutcome, AgentWaitReason};
    use nomifun_db::{SqliteCronRepository, sqlx};

    #[tokio::test]
    async fn restart_reattaches_the_original_live_or_suspended_receipt_and_settles_cancellation_once()
     {
        for suspended in [false, true] {
            let db = nomifun_db::init_database_memory().await.unwrap();
            let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
            let session = ConversationId::new().into_string();
            let job_id = CronJobId::new().into_string();
            let run_id = CronJobRunId::new().into_string();
            let owner_ref =
                serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string();
            sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','Original',0,0,'{}',1,1)")
                .bind(&session).bind(owner_ref).execute(db.pool()).await.unwrap();
            sqlx::query("INSERT INTO cron_jobs(cron_job_id,user_id,name,schedule_kind,schedule_value,payload_message,conversation_id,created_by,created_at,updated_at) VALUES(?,?,'Original job','every','60000','Original input',?,'user',1,1)")
                .bind(&job_id).bind(&owner).bind(&session).execute(db.pool()).await.unwrap();
            let repo = Arc::new(SqliteCronRepository::new(db.pool().clone()));
            repo.reserve_run(
                &owner,
                &ReserveCronRunParams {
                    cron_job_run_id: run_id.clone(),
                    cron_job_id: job_id.clone(),
                    trigger_kind: "run_now".into(),
                    operation_key: format!("original:{run_id}"),
                    request_fingerprint: "original-input".into(),
                    schedule_revision: None,
                    planned_at_ms: None,
                    next_run_at: None,
                    disable: false,
                    now: 1,
                },
            )
            .await
            .unwrap();
            repo.attach_run_conversation(&owner, &run_id, &session, 2)
                .await
                .unwrap();
            let active = CronTurnDelivery {
                message_id: "0190f5fe-7c00-7a00-8000-000000000011".into(),
                replayed: true,
                completed: false,
                turn_outcome: Some(AgentTurnOutcome::Waiting {
                    wait_seq: 41,
                    request_id: None,
                    reason: AgentWaitReason::ExternalResource,
                }),
                user_input_request: None,
                result_ok: None,
                result_text: None,
                result_error: None,
                result_error_code: None,
                result_error_retryable: None,
            };
            let initial = if suspended {
                CronTurnReceiptState::Suspended(active.clone())
            } else {
                CronTurnReceiptState::Accepted {
                    message_id: active.message_id.clone(),
                }
            };
            let sessions = Arc::new(ReceiptOnlySessions::new(initial));
            let executor = Arc::new(JobExecutor::new(
                Arc::from(owner.as_str()),
                sessions.clone(),
                Arc::new(CronBusyGuard::new()),
                PathBuf::new(),
            ));
            let service = CronService::new(
                Arc::from(owner.as_str()),
                repo.clone(),
                Arc::new(CronScheduler::new(Arc::new(|_, _, _, _, _| {}))),
                executor,
                CronEventEmitter::new(Arc::new(nomifun_realtime::BroadcastEventBus::new(16))),
                PathBuf::new(),
            );
            service.recover_interrupted_runs_on_boot().await;
            service.recover_interrupted_runs_on_boot().await;
            assert_eq!(service.active_runs.len(), 1);
            assert!(
                repo.list_runs_by_job(&owner, &job_id, 7)
                    .await
                    .unwrap()
                    .is_empty()
            );
            *sessions.receipt.lock().unwrap() = CronTurnReceiptState::Completed(CronTurnDelivery {
                completed: true,
                turn_outcome: Some(AgentTurnOutcome::Cancelled {
                    message_ids: vec![],
                }),
                result_ok: Some(false),
                result_error_code: Some("cancelled".into()),
                ..active
            });
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let row = repo
                        .get_by_cron_job_id(&owner, &job_id)
                        .await
                        .unwrap()
                        .unwrap();
                    if row.last_status.as_deref() == Some("cancelled")
                        && service.active_runs.is_empty()
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            service.recover_interrupted_runs_on_boot().await;
            let saved = repo
                .get_by_cron_job_id(&owner, &job_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.run_count, 1);

            assert_eq!(saved.last_error, None);
            assert_eq!(
                repo.list_runs_by_job(&owner, &job_id, 7)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                repo.list_reserved_runs_for_scheduler()
                    .await
                    .unwrap()
                    .is_empty()
            );
            repo.delete(&owner, &job_id).await.unwrap();
        }
    }
}
