//! Private bounded-parallel scheduler used only by `AgentExecutionEngine`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::hash::Hash;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use nomifun_api_types::{
    AgentErrorCode, AgentExecution, AgentExecutionDetail, ExecutionModelRef,
    ExecutionParticipant, ExecutionStep,
};
use nomifun_common::{
    AdaptationPolicy, AgentExecutionEventKind, AgentExecutionStatus, AgentStepMode, AppError,
    ExecutionAttemptStatus, ExecutionStepKind, ExecutionStepStatus, StepFailurePolicy,
    apply_agent_role_context, generate_id, now_ms,
};
use nomifun_db::{
    AgentExecutionAttemptRecoveryDisposition, AgentExecutionLeaseToken,
    AgentExecutionTurnAuthority, AttemptConversationEffectParams,
    CreateAgentExecutionAttemptParams, IAgentExecutionRepository, LoopRepeatResetParams,
    NewAgentExecutionEvent, RetryAgentExecutionStep,
    SettleAgentExecutionAttemptParams, UpdateAgentExecutionParams,
};
use serde_json::json;
use tokio::sync::{Notify, watch};

use crate::attempt_runner::{AttemptOutcome, AttemptRunner};
use crate::artifact_contract::{requires_artifact_delivery, validate_required_artifacts};
use crate::control_steps::{self, ControlResolution};
use crate::conversation_effect::{AttemptConversationEffects, PendingConversationEffect};
use crate::domain_mapper;
use crate::event_publisher::AgentExecutionEventPublisher;

pub(crate) const DEFAULT_MAX_PARALLEL: i64 = 4;
pub(crate) const DEFAULT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_PROVIDER_RETRIES: i64 = 2;
const MAX_TIMEOUT_RETRIES: i64 = 1;
const LEASE_DURATION_MS: i64 = 30_000;
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_ACQUIRE_RETRY_MAX: Duration = Duration::from_secs(1);
const LEASE_CAS_RETRY: Duration = Duration::from_millis(50);
const EFFECT_RETRY_MIN: Duration = Duration::from_secs(1);
const EFFECT_RETRY_MAX: Duration = Duration::from_secs(60);
const CLEANUP_EFFECT_TIMEOUT: Duration = Duration::from_secs(2);
const CLEANUP_PARALLELISM: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptRetryClass {
    Deterministic,
    Provider,
    RateLimited,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupValidation {
    Current,
    Stale,
    Retry,
}

/// F50: fence the conversation-scoped cancel to the validated link generation
/// as tightly as the effect API allows. `cancel_attempt` targets
/// (user, conversation) — it carries no attempt-generation parameter — so a
/// replacement active attempt admitted between the batch's validation and the
/// cancel dispatch would have its live turn killed by stale cleanup.
/// Revalidate the exact inactive generation in its own transaction
/// immediately before dispatch and SKIP the cancel when a replacement already
/// owns the Conversation; the row stays pending and reconciliation retries
/// once the replacement's link retires. A replacement that starts while the
/// cancel is already in flight remains exposed for up to
/// `CLEANUP_EFFECT_TIMEOUT`; the exact acknowledgement then refuses to retire
/// the row (see `mark_conversation_cleanup_completed_exact`).
async fn cancel_with_generation_fence<E, V, C>(revalidate: V, cancel: C) -> bool
where
    E: std::fmt::Display,
    V: Future<Output = Result<bool, E>>,
    C: AsyncFnOnce() -> bool,
{
    match revalidate.await {
        Ok(true) => cancel().await,
        Ok(false) => {
            tracing::debug!(
                "skipping Agent conversation cleanup cancel; a replacement attempt claimed the conversation after validation"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "could not refence Agent conversation cleanup before cancel; leaving it pending"
            );
            false
        }
    }
}

async fn reconcile_cleanup_batch<T, K, KeyFn, ValidateFn, CancelFn, AcknowledgeFn>(
    pending: Vec<T>,
    key: KeyFn,
    validate: ValidateFn,
    cancel: CancelFn,
    acknowledge: AcknowledgeFn,
) -> bool
where
    T: Send + Sync,
    K: Eq + Hash,
    KeyFn: Fn(&T) -> K,
    ValidateFn: Fn(&T) -> BoxFuture<'static, CleanupValidation> + Sync,
    CancelFn: Fn(&T) -> BoxFuture<'static, bool> + Sync,
    AcknowledgeFn: Fn(&T) -> BoxFuture<'static, bool> + Sync,
{
    let mut grouped = HashMap::<K, Vec<T>>::new();
    for cleanup in pending {
        grouped.entry(key(&cleanup)).or_default().push(cleanup);
    }

    let validate = &validate;
    let cancel = &cancel;
    let acknowledge = &acknowledge;
    let completed = futures::stream::iter(grouped.into_values().map(|cleanups| async move {
        let mut validation_failed = false;
        let mut cancel_attempted = false;
        let mut cancel_succeeded = false;
        let mut current_found = false;
        let mut acknowledgement_succeeded = false;

        for cleanup in &cleanups {
            match validate(cleanup).await {
                CleanupValidation::Stale => {}
                CleanupValidation::Retry => validation_failed = true,
                CleanupValidation::Current => {
                    current_found = true;
                    if acknowledgement_succeeded {
                        continue;
                    }
                    if !cancel_attempted {
                        cancel_attempted = true;
                        cancel_succeeded = cancel(cleanup).await;
                    }
                    if cancel_succeeded && acknowledge(cleanup).await {
                        // Exact acknowledgement retires every pending inactive
                        // generation for this Conversation. Continue only to
                        // validate the remaining records in the fetched batch.
                        acknowledgement_succeeded = true;
                    }
                }
            }
        }

        !validation_failed && (!current_found || acknowledgement_succeeded)
    }))
    .buffer_unordered(CLEANUP_PARALLELISM)
    .collect::<Vec<_>>()
    .await;
    completed.into_iter().all(|done| done)
}

#[derive(Debug, Clone, Copy)]
struct AttemptSettlementFence {
    step_version: i64,
    attempt_version: i64,
}

#[async_trait]
pub(crate) trait ConversationEffects: Send + Sync {
    async fn cancel_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError>;

    async fn steer_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        text: &str,
    ) -> Result<(), AppError>;

    async fn stop_attempt_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<(), AppError>;

    async fn report_lead(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        operation_id: &str,
    ) -> Result<(), AppError>;
}

pub(crate) struct ExecutionSchedulerDeps {
    pub repository: Arc<dyn IAgentExecutionRepository>,
    pub attempt_runner: Arc<dyn AttemptRunner>,
    pub publisher: AgentExecutionEventPublisher,
    pub conversation_effects: Arc<dyn ConversationEffects>,
    pub data_dir: PathBuf,
    pub attempt_timeout: Duration,
}

impl ExecutionSchedulerDeps {
    pub fn new(
        repository: Arc<dyn IAgentExecutionRepository>,
        attempt_runner: Arc<dyn AttemptRunner>,
        conversation_effects: Arc<dyn ConversationEffects>,
        publisher: AgentExecutionEventPublisher,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            repository,
            attempt_runner,
            publisher,
            conversation_effects,
            data_dir,
            attempt_timeout: DEFAULT_ATTEMPT_TIMEOUT,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ExecutionScheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    deps: ExecutionSchedulerDeps,
    instance_id: String,
    active: DashMap<String, ActiveHandle>,
    pending_lead_reports: DashMap<String, ()>,
    cleanup_reconciliation_running: DashMap<&'static str, ()>,
}

struct ActiveHandle {
    generation: String,
    cancel: watch::Sender<bool>,
    wake: Arc<Notify>,
    restart_requested: bool,
    lease: Option<AgentExecutionLeaseToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerLoopExit {
    Normal,
    LeaseLost,
}

impl ExecutionScheduler {
    pub fn new(deps: ExecutionSchedulerDeps) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                deps,
                instance_id: generate_id(),
                active: DashMap::new(),
                pending_lead_reports: DashMap::new(),
                cleanup_reconciliation_running: DashMap::new(),
            }),
        }
    }

    pub fn start(&self, owner_id: String, execution_id: String) {
        use dashmap::mapref::entry::Entry;
        let generation = generate_id();
        let (cancel, receiver) = watch::channel(false);
        match self.inner.active.entry(execution_id.clone()) {
            Entry::Occupied(mut entry) => {
                // A stop followed immediately by start (resume, replan,
                // adjust) must not lose the new scheduling request while the
                // prior generation is still unwinding. Remember one durable
                // wake-up; the exiting generation starts its successor only
                // after releasing the lease.
                let handle = entry.get_mut();
                handle.restart_requested = true;
                // A live scheduler may be asleep on an automatic retry
                // backoff. New work or a manual retry must wake it immediately
                // rather than waiting for the old timer to expire.
                handle.wake.notify_one();
                return;
            }
            Entry::Vacant(entry) => {
                let wake = Arc::new(Notify::new());
                entry.insert(ActiveHandle {
                    generation: generation.clone(),
                    cancel,
                    wake: wake.clone(),
                    restart_requested: false,
                    lease: None,
                });
                let scheduler = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = scheduler
                        .execute_loop(
                            &owner_id,
                            &execution_id,
                            &generation,
                            receiver,
                            wake,
                        )
                        .await
                    {
                        tracing::error!(%execution_id, %error, "Agent Execution scheduler stopped with an error");
                    }
                    // Read restart_requested and remove this exact generation while
                    // holding the same shard lock. A concurrent resume between a
                    // separate read/remove pair would otherwise be lost.
                    let restart = match scheduler.inner.active.entry(execution_id.clone()) {
                        Entry::Occupied(entry) if entry.get().generation == generation => {
                            entry.remove().restart_requested
                        }
                        _ => false,
                    };
                    if restart {
                        scheduler.start(owner_id, execution_id);
                    }
                });
            }
        }
    }

    pub fn stop(&self, execution_id: &str) {
        if let Some(handle) = self.inner.active.get(execution_id) {
            let _ = handle.cancel.send(true);
        }
    }

    /// Return the ownership proof held by this process's current generation.
    /// Out-of-band attempt callbacks use it to share the scheduler's DB fence.
    pub(crate) fn lease_token(&self, execution_id: &str) -> Option<AgentExecutionLeaseToken> {
        self.inner
            .active
            .get(execution_id)
            .and_then(|handle| handle.lease.clone())
    }

    fn publish_lease_token(
        &self,
        execution_id: &str,
        generation: &str,
        lease: AgentExecutionLeaseToken,
    ) -> bool {
        let Some(mut handle) = self.inner.active.get_mut(execution_id) else {
            return false;
        };
        if handle.generation != generation || *handle.cancel.borrow() {
            return false;
        }
        handle.lease = Some(lease);
        true
    }

    fn request_generation_restart(&self, execution_id: &str, generation: &str) {
        if let Some(mut handle) = self.inner.active.get_mut(execution_id)
            && handle.generation == generation
        {
            handle.restart_requested = true;
        }
    }

    pub async fn cancel_conversations(&self, _owner_id: &str, detail: &AgentExecutionDetail) {
        self.reconcile_conversation_cleanup(Some(&detail.execution.execution_id))
            .await;
    }

    pub async fn cancel_conversations_for_steps(
        &self,
        _owner_id: &str,
        detail: &AgentExecutionDetail,
        _step_ids: &HashSet<String>,
    ) {
        self.reconcile_conversation_cleanup(Some(&detail.execution.execution_id))
            .await;
    }

    /// Drain the durable cleanup outbox encoded by inactive attempt links.
    /// Cancellation and acknowledgement are deliberately separate: a crash
    /// between them repeats an idempotent cancel instead of losing cleanup.
    pub async fn reconcile_conversation_cleanup(&self, execution_id: Option<&str>) {
        if !self.reconcile_conversation_cleanup_once(execution_id).await {
            self.schedule_cleanup_reconciliation();
        }
    }

    async fn reconcile_conversation_cleanup_once(&self, execution_id: Option<&str>) -> bool {
        let pending = match self
            .inner
            .deps
            .repository
            .list_pending_conversation_cleanups(execution_id, 100)
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(%error, "failed to load pending Agent conversation cleanup");
                return false;
            }
        };
        if pending.is_empty() {
            return true;
        }
        let batch_is_full = pending.len() == 100;
        let validate_repository = self.inner.deps.repository.clone();
        let cancel_repository = self.inner.deps.repository.clone();
        let acknowledge_repository = self.inner.deps.repository.clone();
        let conversation_effects = self.inner.deps.conversation_effects.clone();
        let completed = reconcile_cleanup_batch(
            pending,
            |cleanup| (cleanup.user_id.clone(), cleanup.conversation_id.clone()),
            move |cleanup| {
                let repository = validate_repository.clone();
                let cleanup = cleanup.clone();
                Box::pin(async move {
                    match repository.validate_conversation_cleanup(&cleanup).await {
                        Ok(true) => CleanupValidation::Current,
                        Ok(false) => {
                            tracing::debug!(
                                link_id = cleanup.link_id,
                                execution_id = %cleanup.execution_id,
                                step_id = %cleanup.step_id,
                                attempt_id = %cleanup.attempt_id,
                                conversation_id = cleanup.conversation_id,
                                "skipping stale Agent conversation cleanup generation"
                            );
                            CleanupValidation::Stale
                        }
                        Err(error) => {
                            tracing::warn!(
                                link_id = cleanup.link_id,
                                execution_id = %cleanup.execution_id,
                                step_id = %cleanup.step_id,
                                attempt_id = %cleanup.attempt_id,
                                conversation_id = cleanup.conversation_id,
                                %error,
                                "failed to atomically revalidate Agent conversation cleanup"
                            );
                            CleanupValidation::Retry
                        }
                    }
                })
            },
            move |cleanup| {
                let repository = cancel_repository.clone();
                let conversation_effects = conversation_effects.clone();
                let cleanup = cleanup.clone();
                Box::pin(async move {
                    cancel_with_generation_fence(
                        repository.validate_conversation_cleanup(&cleanup),
                        async || {
                            let cancelled = tokio::time::timeout(
                                CLEANUP_EFFECT_TIMEOUT,
                                conversation_effects
                                    .cancel_attempt(&cleanup.user_id, &cleanup.conversation_id),
                            )
                            .await;
                            match cancelled {
                                Ok(Ok(())) => true,
                                Ok(Err(error)) => {
                                    tracing::warn!(
                                        link_id = cleanup.link_id,
                                        execution_id = %cleanup.execution_id,
                                        step_id = %cleanup.step_id,
                                        attempt_id = %cleanup.attempt_id,
                                        conversation_id = cleanup.conversation_id,
                                        %error,
                                        "Agent conversation cleanup remains pending"
                                    );
                                    false
                                }
                                Err(_) => {
                                    tracing::warn!(
                                        link_id = cleanup.link_id,
                                        execution_id = %cleanup.execution_id,
                                        step_id = %cleanup.step_id,
                                        attempt_id = %cleanup.attempt_id,
                                        conversation_id = cleanup.conversation_id,
                                        "Agent conversation cleanup timed out and remains pending"
                                    );
                                    false
                                }
                            }
                        },
                    )
                    .await
                })
            },
            move |cleanup| {
                let repository = acknowledge_repository.clone();
                let cleanup = cleanup.clone();
                Box::pin(async move {
                    match repository
                        .mark_conversation_cleanup_completed_exact(&cleanup, now_ms())
                        .await
                    {
                        Ok(true) => true,
                        Ok(false) => {
                            tracing::warn!(
                                link_id = cleanup.link_id,
                                execution_id = %cleanup.execution_id,
                                step_id = %cleanup.step_id,
                                attempt_id = %cleanup.attempt_id,
                                conversation_id = cleanup.conversation_id,
                                "Agent conversation cleanup acknowledgement lost exact-generation authority"
                            );
                            false
                        }
                        Err(error) => {
                            tracing::warn!(
                                link_id = cleanup.link_id,
                                execution_id = %cleanup.execution_id,
                                step_id = %cleanup.step_id,
                                attempt_id = %cleanup.attempt_id,
                                conversation_id = cleanup.conversation_id,
                                %error,
                                "Agent conversation cleanup acknowledgement remains pending"
                            );
                            false
                        }
                    }
                })
            },
        )
        .await;
        !batch_is_full && completed
    }

    fn schedule_cleanup_reconciliation(&self) {
        const KEY: &str = "all";
        if self
            .inner
            .cleanup_reconciliation_running
            .insert(KEY, ())
            .is_some()
        {
            return;
        }
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut delay = EFFECT_RETRY_MIN;
            loop {
                tokio::time::sleep(delay).await;
                if scheduler.reconcile_conversation_cleanup_once(None).await {
                    break;
                }
                delay = next_effect_retry_delay(delay);
            }
            scheduler.inner.cleanup_reconciliation_running.remove(KEY);
        });
    }

    pub async fn reconcile_lead_report(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
    ) -> Result<(), AppError> {
        if !self.reconcile_lead_report_once(owner_id, detail).await? {
            self.schedule_lead_report_reconciliation(
                owner_id.to_owned(),
                detail.execution.execution_id.clone(),
            );
        }
        Ok(())
    }

    /// Reopen commands must serialize terminal epochs into the lead
    /// Conversation before mutating the aggregate back to Running. With direct
    /// assistant projection there is no accepted/in-progress state: success
    /// means the durable row exists and its delivered event is committed.
    pub async fn ensure_terminal_projection_delivered(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
    ) -> Result<(), AppError> {
        if !detail.execution.status.is_terminal()
            || detail.execution.lead_conversation_id.is_none()
        {
            return Ok(());
        }
        if self.reconcile_lead_report_once(owner_id, detail).await? {
            Ok(())
        } else {
            Err(AppError::Conflict(
                "terminal Agent Execution result is still being projected".to_owned(),
            ))
        }
    }

    /// One post-commit path for every terminal transition. It publishes the
    /// terminal outbox, drains inactive Attempt links as durable cleanup work,
    /// then reloads canonical state and reconciles the idempotent lead report.
    pub async fn after_terminal_commit(&self, owner_id: &str, execution_id: &str) {
        self.publish().await;
        self.reconcile_conversation_cleanup(Some(execution_id)).await;
        let Ok(detail) = self.detail(owner_id, execution_id).await else {
            return;
        };
        if !detail.execution.status.is_terminal() {
            return;
        }
        if let Err(error) = self.reconcile_lead_report(owner_id, &detail).await {
            tracing::warn!(%execution_id, %error, "failed to reconcile terminal lead report");
        }
    }

    async fn reconcile_lead_report_once(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
    ) -> Result<bool, AppError> {
        if !detail.execution.status.is_terminal()
            || detail.execution.lead_conversation_id.is_none()
        {
            return Ok(true);
        }
        let mut after_sequence = 0;
        let mut requested_operation_id: Option<String> = None;
        let mut delivered_operation_ids = HashSet::new();
        loop {
            let events = self
                .inner
                .deps
                .repository
                .list_events(owner_id, &detail.execution.execution_id, after_sequence, 500)
                .await?;
            for event in &events {
                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload) {
                    if let Some(operation_id) = payload
                        .get("lead_report_operation_id")
                        .and_then(serde_json::Value::as_str)
                    {
                        // The newest terminal epoch supersedes an older
                        // unreported terminal state. Reopen commands ensure
                        // the previous epoch is delivered before mutation, so
                        // this normally advances one epoch at a time.
                        requested_operation_id = Some(operation_id.to_owned());
                    }
                    if
                        payload.get("change").and_then(serde_json::Value::as_str)
                            == Some("lead_report_delivered")
                            && let Some(operation_id) = payload
                                .get("operation_id")
                                .and_then(serde_json::Value::as_str)
                    {
                        delivered_operation_ids.insert(operation_id.to_owned());
                    }
                }
            }
            let Some(last) = events.last() else {
                break;
            };
            after_sequence = last.sequence;
            if events.len() < 500 {
                break;
            }
        }
        let Some(operation_id) = requested_operation_id else {
            return Ok(true);
        };
        if delivered_operation_ids.contains(&operation_id) {
            return Ok(true);
        }
        self.inner
            .deps
            .conversation_effects
            .report_lead(owner_id, detail, &operation_id)
            .await?;
        let current = self.detail(owner_id, &detail.execution.execution_id).await?;
        self.inner
            .deps
            .repository
            .append_event(
                owner_id,
                &detail.execution.execution_id,
                current.execution.version,
                &system_event(
                    AgentExecutionEventKind::StatusChanged,
                    None,
                    None,
                    json!({
                        "change":"lead_report_delivered",
                        "operation_id":operation_id,
                    }),
                ),
            )
            .await?;
        self.publish().await;
        Ok(true)
    }

    fn schedule_lead_report_reconciliation(&self, owner_id: String, execution_id: String) {
        if self
            .inner
            .pending_lead_reports
            .insert(execution_id.clone(), ())
            .is_some()
        {
            return;
        }
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut delay = EFFECT_RETRY_MIN;
            loop {
                tokio::time::sleep(delay).await;
                let completed = match scheduler.detail(&owner_id, &execution_id).await {
                    Ok(detail) => match scheduler
                        .reconcile_lead_report_once(&owner_id, &detail)
                        .await
                    {
                        Ok(completed) => completed,
                        Err(error) => {
                            tracing::warn!(
                                %execution_id,
                                %error,
                                "durable lead report reconciliation remains pending"
                            );
                            false
                        }
                    },
                    Err(AppError::NotFound(_)) => true,
                    Err(error) => {
                        tracing::warn!(
                            %execution_id,
                            %error,
                            "failed to reload execution for lead report reconciliation"
                        );
                        false
                    }
                };
                if completed {
                    break;
                }
                delay = next_effect_retry_delay(delay);
            }
            scheduler.inner.pending_lead_reports.remove(&execution_id);
        });
    }

    pub async fn steer_conversation(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        text: &str,
    ) -> Result<(), AppError> {
        self.inner
            .deps
            .conversation_effects
            .steer_attempt(owner_id, conversation_id, operation_id, text)
            .await
    }

    pub async fn stop_attempt_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<(), AppError> {
        self.inner
            .deps
            .conversation_effects
            .stop_attempt_turn(owner_id, conversation_id, operation_id)
            .await
    }

    pub async fn read_attempt_output(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Option<String> {
        self.inner
            .deps
            .attempt_runner
            .read_final_output(owner_id, conversation_id)
            .await
    }

    pub async fn read_attempt_output_files(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Vec<String> {
        self.inner
            .deps
            .attempt_runner
            .read_output_files(owner_id, conversation_id)
            .await
    }

    async fn execute_loop(
        &self,
        owner_id: &str,
        execution_id: &str,
        generation: &str,
        mut cancelled: watch::Receiver<bool>,
        wake: Arc<Notify>,
    ) -> Result<(), AppError> {
        let repository = &self.inner.deps.repository;
        let Some((lease, expiry)) = self
            .acquire_lease(owner_id, execution_id, generation, &mut cancelled)
            .await?
        else {
            return Ok(());
        };
        if !self.publish_lease_token(execution_id, generation, lease.clone()) {
            let _ = repository
                .release_lease(execution_id, lease.owner(), expiry.load(Ordering::SeqCst))
                .await;
            return Ok(());
        }
        let (lease_stop, lease_stopped) = watch::channel(false);
        let (lease_lost, mut lease_loss) = watch::channel(false);
        let heartbeat = self.spawn_lease_heartbeat(
            execution_id.to_owned(),
            lease.owner().to_owned(),
            expiry.clone(),
            lease_stopped,
            lease_lost,
        );

        let result: Result<SchedulerLoopExit, AppError> = async {
            // Decision answers and steers are write-ahead effects in attempt
            // runtime_state.  Recover them before classifying a running
            // attempt as process-interrupted.
            while self
                .process_one_pending_conversation_effect(owner_id, execution_id, &lease)
                .await?
            {}
            self.recover_interrupted(owner_id, execution_id, &lease).await?;
            let mut running_jobs = FuturesUnordered::new();
            let mut in_flight_step_ids = HashSet::new();
            let mut deferred_error: Option<AppError> = None;
            loop {
                if *cancelled.borrow() {
                    return Ok(SchedulerLoopExit::Normal);
                }
                if *lease_loss.borrow() {
                    return Ok(SchedulerLoopExit::LeaseLost);
                }
                // A failed job must not drop unrelated live model calls. Stop
                // dispatching new work, drain the already-reserved jobs, then
                // hand the first error to the normal recovery/fatal path.
                if deferred_error.is_some() {
                    if running_jobs.is_empty() {
                        return Err(deferred_error.take().expect("checked above"));
                    }
                    tokio::select! {
                        outcome = running_jobs.next() => {
                            if let Some((step_id, outcome)) = outcome {
                                in_flight_step_ids.remove(&step_id);
                                if let Err(error) = outcome {
                                    tracing::warn!(%execution_id, %step_id, %error, "additional Agent step failed while draining in-flight work");
                                }
                            }
                        }
                        changed = cancelled.changed() => {
                            if changed.is_err() || *cancelled.borrow() {
                                return Ok(SchedulerLoopExit::Normal);
                            }
                        }
                        changed = lease_loss.changed() => {
                            if changed.is_err() || *lease_loss.borrow() {
                                return Ok(SchedulerLoopExit::LeaseLost);
                            }
                        }
                        _ = wake.notified() => {}
                    }
                    continue;
                }
                let mut detail = self.detail(owner_id, execution_id).await?;
                match detail.execution.status {
                    AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput => {}
                    AgentExecutionStatus::Planning
                    | AgentExecutionStatus::AwaitingApproval
                    | AgentExecutionStatus::Paused
                    | AgentExecutionStatus::Completed
                    | AgentExecutionStatus::CompletedWithFailures
                    | AgentExecutionStatus::Failed
                    | AgentExecutionStatus::Cancelled => return Ok(SchedulerLoopExit::Normal),
                }
                if self
                    .process_one_pending_conversation_effect(owner_id, execution_id, &lease)
                    .await?
                {
                    continue;
                }
                if self.ensure_work_dir(owner_id, &mut detail, &lease).await? {
                    continue;
                }
                if self
                    .skip_one_blocked_step(owner_id, &detail, &lease)
                    .await?
                {
                    continue;
                }

                let ready = ready_steps(&detail, now_ms());
                // Control nodes depend only on their declared DAG blockers;
                // an unrelated long-running Agent is not an implicit global
                // barrier. Their evaluation is local/transactional, so run one
                // ready control inline and immediately reload canonical state.
                if let Some(control) = ready
                    .iter()
                    .find(|step| step.kind != ExecutionStepKind::Agent)
                    .copied()
                {
                    if let Err(error) = self
                        .execute_control_step(owner_id, &detail, control, &lease)
                        .await
                    {
                        deferred_error = Some(error);
                    }
                    continue;
                }
                let agent_steps = select_agent_steps(&detail, ready, &in_flight_step_ids);
                for step in agent_steps {
                    let step_id = step.step_id.clone();
                    // Reserve synchronously before the future is first polled.
                    // DB Queued/Running state alone cannot fence this window.
                    in_flight_step_ids.insert(step_id.clone());
                    let scheduler = self.clone();
                    let owner_id = owner_id.to_owned();
                    let execution_id = execution_id.to_owned();
                    let lease = lease.clone();
                    running_jobs.push(async move {
                        let outcome = scheduler
                            .execute_agent_step(&owner_id, &execution_id, step, &lease)
                            .await;
                        (step_id, outcome)
                    });
                }
                if !running_jobs.is_empty() {
                    tokio::select! {
                        outcome = running_jobs.next() => {
                            if let Some((step_id, outcome)) = outcome {
                                in_flight_step_ids.remove(&step_id);
                                if let Err(error) = outcome {
                                    deferred_error = Some(error);
                                }
                            }
                        }
                        changed = cancelled.changed() => {
                            if changed.is_err() || *cancelled.borrow() {
                                return Ok(SchedulerLoopExit::Normal);
                            }
                        }
                        changed = lease_loss.changed() => {
                            if changed.is_err() || *lease_loss.borrow() {
                                return Ok(SchedulerLoopExit::LeaseLost);
                            }
                        }
                        _ = wake.notified() => {}
                    }
                    continue;
                }

                if self.finalize_if_settled(owner_id, &detail, &lease).await? {
                    return Ok(SchedulerLoopExit::Normal);
                }
                if let Some(wake_at) = next_retry_at(&detail) {
                    let delay = (wake_at - now_ms()).clamp(1, 1_000) as u64;
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                        changed = cancelled.changed() => {
                            if changed.is_err() || *cancelled.borrow() {
                                return Ok(SchedulerLoopExit::Normal);
                            }
                        }
                        changed = lease_loss.changed() => {
                            if changed.is_err() || *lease_loss.borrow() {
                                return Ok(SchedulerLoopExit::LeaseLost);
                            }
                        }
                        _ = wake.notified() => {}
                    }
                    continue;
                }
                if detail.attempts.iter().any(|attempt| {
                    attempt.status == ExecutionAttemptStatus::WaitingInput
                }) {
                    // WaitingInput is an aggregate attention signal.  All
                    // independent runnable work above has been exhausted, so
                    // release the lease until a durable answer wakes us.
                    return Ok(SchedulerLoopExit::Normal);
                }
                self.fail_if_active(
                    owner_id,
                    execution_id,
                    "no schedulable active step",
                    &lease,
                )
                .await;
                return Ok(SchedulerLoopExit::Normal);
            }
        }
        .await;

        let lease_was_lost = matches!(&result, Ok(SchedulerLoopExit::LeaseLost))
            || *lease_loss.borrow();
        if let Err(error) = &result {
            tracing::warn!(%execution_id, %error, "Agent Execution scheduler iteration aborted");
            if scheduler_error_is_recoverable(error) || lease_was_lost {
                self.request_generation_restart(execution_id, generation);
            } else {
                self.fail_if_active(owner_id, execution_id, &error.to_string(), &lease)
                    .await;
            }
        } else if lease_was_lost {
            self.request_generation_restart(execution_id, generation);
        }
        let _ = lease_stop.send(true);
        if let Err(error) = heartbeat.await {
            tracing::warn!(%execution_id, %error, "execution lease heartbeat task failed");
            self.request_generation_restart(execution_id, generation);
        }
        let expected_expiry = expiry.load(Ordering::SeqCst);
        if let Err(error) = repository
            .release_lease(execution_id, lease.owner(), expected_expiry)
            .await
        {
            tracing::warn!(%execution_id, %error, "failed to release execution lease");
        }
        Ok(())
    }

    async fn acquire_lease(
        &self,
        owner_id: &str,
        execution_id: &str,
        generation: &str,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<Option<(AgentExecutionLeaseToken, Arc<AtomicI64>)>, AppError> {
        let repository = &self.inner.deps.repository;
        let lease = AgentExecutionLeaseToken::new(format!(
            "{}:{execution_id}:{generation}",
            self.inner.instance_id
        ));
        loop {
            if *cancelled.borrow() {
                return Ok(None);
            }
            let row = match repository.get_execution(owner_id, execution_id).await {
                Ok(Some(row)) => row,
                Ok(None) => return Ok(None),
                Err(error) => {
                    tracing::warn!(%execution_id, %error, "failed to inspect execution lease; retrying");
                    if wait_for_cancel(cancelled, LEASE_ACQUIRE_RETRY_MAX).await {
                        return Ok(None);
                    }
                    continue;
                }
            };
            let status = row.status.parse::<AgentExecutionStatus>().map_err(|error| {
                AppError::Internal(format!("invalid persisted execution status: {error}"))
            })?;
            if !matches!(
                status,
                AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput
            ) {
                return Ok(None);
            }
            let expires_at = now_ms() + LEASE_DURATION_MS;
            match repository
                .try_acquire_lease(execution_id, row.version, lease.owner(), expires_at)
                .await
            {
                Ok(Some(_)) => {
                    return Ok(Some((lease, Arc::new(AtomicI64::new(expires_at)))))
                }
                Ok(None) => {
                    let delay = lease_retry_delay(row.lease_owner.as_deref(), row.lease_expires_at);
                    if wait_for_cancel(cancelled, delay).await {
                        return Ok(None);
                    }
                }
                Err(error) => {
                    tracing::warn!(%execution_id, %error, "failed to acquire execution lease; retrying");
                    if wait_for_cancel(cancelled, LEASE_ACQUIRE_RETRY_MAX).await {
                        return Ok(None);
                    }
                }
            }
        }
    }

    fn spawn_lease_heartbeat(
        &self,
        execution_id: String,
        owner: String,
        expiry: Arc<AtomicI64>,
        mut stopped: watch::Receiver<bool>,
        lease_lost: watch::Sender<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let repository = self.inner.deps.repository.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() { return; }
                    }
                    _ = tokio::time::sleep(LEASE_RENEW_INTERVAL) => {}
                }
                let old = expiry.load(Ordering::SeqCst);
                let new = now_ms() + LEASE_DURATION_MS;
                let renew = repository.renew_lease(&execution_id, &owner, old, new);
                tokio::select! {
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() { return; }
                    }
                    result = renew => match result {
                        Ok(Some(_)) => expiry.store(new, Ordering::SeqCst),
                        Ok(None) => {
                            let _ = lease_lost.send(true);
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(%execution_id, %owner, %error, "execution lease heartbeat failed");
                            let _ = lease_lost.send(true);
                            return;
                        }
                    }
                }
            }
        })
    }

    async fn detail(&self, owner_id: &str, execution_id: &str) -> Result<AgentExecutionDetail, AppError> {
        let rows = self
            .inner
            .deps
            .repository
            .get_execution_detail(owner_id, execution_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Agent Execution {execution_id}")))?;
        domain_mapper::detail(rows)
    }

    async fn process_one_pending_conversation_effect(
        &self,
        owner_id: &str,
        execution_id: &str,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<bool, AppError> {
        let detail = self.detail(owner_id, execution_id).await?;
        let mut candidate = None;
        for attempt in &detail.attempts {
            if !matches!(
                attempt.status,
                ExecutionAttemptStatus::Running | ExecutionAttemptStatus::WaitingInput
            ) {
                continue;
            }
            let Some(step) = detail.steps.iter().find(|step| {
                step.step_id == attempt.step_id
                    && step.superseded_in_revision.is_none()
                    && step.kind == ExecutionStepKind::Agent
            }) else {
                continue;
            };
            let Some(raw) = attempt.runtime_state.clone() else {
                continue;
            };
            let effects = serde_json::from_value::<AttemptConversationEffects>(raw).map_err(
                |error| {
                    AppError::Internal(format!(
                        "attempt {} has malformed durable conversation effects: {error}",
                        attempt.attempt_id
                    ))
                },
            )?;
            if effects.review_blocked.is_some() {
                continue;
            }
            if !effects.pending_conversation_effects.is_empty() {
                candidate = Some((step.clone(), attempt.clone(), effects));
                break;
            }
        }
        let Some((step, attempt, mut effects)) = candidate else {
            return Ok(false);
        };
        let conversation_id = attempt.conversation_id.as_deref().ok_or_else(|| {
            AppError::Internal(format!(
                "attempt {} has durable conversation effects but no active conversation link",
                attempt.attempt_id
            ))
        })?;
        let effect = effects.pending_conversation_effects.remove(0);
        match effect {
            PendingConversationEffect::StopTurn { operation_id } => {
                self.inner
                    .deps
                    .conversation_effects
                    .stop_attempt_turn(owner_id, conversation_id, &operation_id)
                    .await
                    .map_err(|error| {
                        AppError::BadGateway(format!(
                            "durable turn stop {operation_id} failed: {error}"
                        ))
                    })?;
                let runtime_state = if effects.pending_conversation_effects.is_empty() {
                    None
                } else {
                    Some(effects.encode()?)
                };
                self.inner
                    .deps
                    .repository
                    .acknowledge_attempt_conversation_effect(
                        owner_id,
                        execution_id,
                        &step.step_id,
                        &attempt.attempt_id,
                        attempt.version,
                        &AttemptConversationEffectParams { runtime_state },
                        &system_event(
                            AgentExecutionEventKind::StepChanged,
                            Some(&step.step_id),
                            Some(&attempt.attempt_id),
                            json!({
                                "change":"conversation_effect_delivered",
                                "effect":"stop_turn",
                                "operation_id":operation_id,
                            }),
                        ),
                    )
                    .await?;
                self.publish().await;
            }
            PendingConversationEffect::DecisionInput {
                operation_id,
                content,
            } => {
                // A decision resumes the existing model turn.  Keep the
                // write-ahead state intact until attempt settlement; transport
                // failure is retried under the same stable operation identity.
                let outcome = self
                    .inner
                    .deps
                    .attempt_runner
                    .continue_with_input(
                        owner_id,
                        conversation_id,
                        &operation_id,
                        AgentExecutionTurnAuthority {
                            execution_id: execution_id.to_owned(),
                            step_id: step.step_id.clone(),
                            attempt_id: attempt.attempt_id.clone(),
                            expected_step_version: step.version,
                            expected_attempt_version: attempt.version,
                            lease_owner: lease.owner().to_owned(),
                        },
                        &content,
                        self.inner.deps.attempt_timeout,
                    )
                    .await
                    .map_err(|error| {
                        AppError::BadGateway(format!(
                            "durable decision delivery {operation_id} failed: {error}"
                        ))
                    })?;
                self.settle_agent_outcome(
                    owner_id,
                    execution_id,
                    &step.step_id,
                    &attempt.attempt_id,
                    Ok(outcome),
                    attempt.attempt_no,
                    AttemptSettlementFence {
                        step_version: step.version,
                        attempt_version: attempt.version,
                    },
                    Some(lease),
                )
                .await?;
            }
            PendingConversationEffect::Steer {
                operation_id,
                content,
            } => {
                self.inner
                    .deps
                    .conversation_effects
                    .steer_attempt(owner_id, conversation_id, &operation_id, &content)
                    .await
                    .map_err(|error| {
                        AppError::BadGateway(format!(
                            "durable steer delivery {operation_id} failed: {error}"
                        ))
                    })?;
                let runtime_state = if effects.pending_conversation_effects.is_empty() {
                    None
                } else {
                    Some(effects.encode()?)
                };
                self.inner
                    .deps
                    .repository
                    .acknowledge_attempt_conversation_effect(
                        owner_id,
                        execution_id,
                        &step.step_id,
                        &attempt.attempt_id,
                        attempt.version,
                        &AttemptConversationEffectParams { runtime_state },
                        &system_event(
                            AgentExecutionEventKind::StepChanged,
                            Some(&step.step_id),
                            Some(&attempt.attempt_id),
                            json!({
                                "change":"conversation_effect_delivered",
                                "effect":"steer",
                                "operation_id":operation_id,
                            }),
                        ),
                    )
                    .await?;
                self.publish().await;
            }
        }
        Ok(true)
    }

    async fn recover_interrupted(
        &self,
        owner_id: &str,
        execution_id: &str,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<(), AppError> {
        loop {
            let detail = self.detail(owner_id, execution_id).await?;
            let Some(attempt) = detail.attempts.iter().find(|attempt| {
                matches!(attempt.status, ExecutionAttemptStatus::Queued | ExecutionAttemptStatus::Running)
            }) else {
                return Ok(());
            };
            let Some(step) = detail.steps.iter().find(|step| step.step_id == attempt.step_id) else {
                return Err(AppError::Internal(format!("attempt {} has no step", attempt.attempt_id)));
            };
            // Queued means the concrete Agent invocation never started. It is
            // safe to cancel that reservation and reschedule the step under
            // both fixed and adaptive policies. Only an actually-running
            // invocation consumes the fixed policy's single attempt.
            let was_queued = attempt.status == ExecutionAttemptStatus::Queued;
            if was_queued {
                self.inner
                    .deps
                    .attempt_runner
                    .discard_unlinked_creation(owner_id, &attempt.attempt_id)
                    .await?;
            }
            let recovered = self.inner
                .deps
                .repository
                .reconcile_recovered_attempt(
                    owner_id,
                    execution_id,
                    &step.step_id,
                    step.version,
                    &attempt.attempt_id,
                    attempt.version,
                    lease,
                    &system_event(
                        AgentExecutionEventKind::AttemptChanged,
                        Some(&step.step_id),
                        Some(&attempt.attempt_id),
                        json!({
                            "reason": if was_queued { "queued_before_restart" } else { "process_restart" },
                            "reconciliation":"initial_turn_receipt",
                        }),
                    ),
                )
                .await?;
            match recovered.disposition {
                AgentExecutionAttemptRecoveryDisposition::QueuedRescheduled => {}
                AgentExecutionAttemptRecoveryDisposition::CompletedReceiptAdopted => {
                    tracing::info!(
                        %execution_id,
                        step_id = %step.step_id,
                        attempt_id = %attempt.attempt_id,
                        "adopted completed initial-turn receipt during recovery"
                    );
                }
                AgentExecutionAttemptRecoveryDisposition::ReviewBlocked => {
                    tracing::warn!(
                        %execution_id,
                        step_id = %step.step_id,
                        attempt_id = %attempt.attempt_id,
                        "interrupted initial turn was parked for review; automatic retry is blocked"
                    );
                }
            }
            self.publish().await;
            self.reconcile_conversation_cleanup(Some(execution_id)).await;
        }
    }

    /// Resolve and provision the execution workspace before the first Attempt
    /// is created.
    ///
    /// Conversation workspaces are normally absolute, backend-created paths.
    /// Agent Execution also accepts an explicit `work_dir`, though, and older
    /// callers were able to persist a relative/non-existent value verbatim.
    /// That value later reached the knowledge broker and failed during
    /// `canonicalize`, after the scheduler had already created retries.  Make
    /// the boundary deterministic here:
    ///
    /// * omitted paths use the isolated execution root;
    /// * relative paths are rooted below that execution root (never the
    ///   process' current directory);
    /// * every path is created, canonicalized, and persisted before dispatch;
    /// * parent traversal and cross-platform path spellings are rejected.
    ///
    /// `true` means the execution row changed and the caller should reload its
    /// detail before making a scheduling decision.
    async fn ensure_work_dir(
        &self,
        owner_id: &str,
        detail: &mut AgentExecutionDetail,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<bool, AppError> {
        let execution_root = self
            .inner
            .deps
            .data_dir
            .join("agent-executions")
            .join(&detail.execution.execution_id);
        let requested = detail
            .execution
            .work_dir
            .as_deref()
            .filter(|value| !value.is_empty());
        let path = match requested {
            None => execution_root.clone(),
            Some(raw) => resolve_requested_work_dir(&execution_root, raw)?,
        };
        if nomifun_common::workspace_path_has_edge_whitespace_segment(&path) {
            return Err(AppError::BadRequest(format!(
                "Agent Execution work_dir contains a directory name with leading/trailing whitespace: {}",
                path.display()
            )));
        }

        // Do not let `create_dir_all` follow an existing symlink/junction
        // beneath an execution root before the containment check runs.  Find
        // the nearest existing ancestor first, canonicalize that ancestor,
        // and reject a relative request whose already-materialized prefix
        // leaves the isolated execution root.  The final canonical check
        // below still handles races and the newly-created suffix.
        let canonical_root = if requested.is_some_and(|raw| !Path::new(raw).is_absolute()) {
            tokio::fs::create_dir_all(&execution_root)
                .await
                .map_err(|error| {
                    AppError::BadRequest(format!(
                        "Agent Execution work root cannot be created '{}': {error}",
                        execution_root.display()
                    ))
                })?;
            Some(
                nomifun_common::paths::canonicalize_simplified(&execution_root).map_err(
                    |error| {
                        AppError::Internal(format!(
                            "canonicalize execution work root '{}': {error}",
                            execution_root.display()
                        ))
                    },
                )?,
            )
        } else {
            None
        };
        if let Some(canonical_root) = canonical_root.as_ref() {
            let existing_ancestor = nearest_existing_ancestor(&path).ok_or_else(|| {
                AppError::BadRequest(format!(
                    "relative Agent Execution work_dir has no usable ancestor: {}",
                    path.display()
                ))
            })?;
            let canonical_ancestor =
                nomifun_common::paths::canonicalize_simplified(existing_ancestor).map_err(
                    |error| {
                        AppError::BadRequest(format!(
                            "relative Agent Execution work_dir cannot be canonicalized '{}': {error}",
                            existing_ancestor.display()
                        ))
                    },
                )?;
            if !canonical_ancestor.starts_with(canonical_root) {
                return Err(AppError::BadRequest(format!(
                    "relative Agent Execution work_dir resolves outside its execution root: {}",
                    canonical_ancestor.display()
                )));
            }
        }

        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|error| {
                AppError::BadRequest(format!(
                    "Agent Execution work_dir cannot be created '{}': {error}",
                    path.display()
                ))
            })?;
        let canonical = nomifun_common::paths::canonicalize_simplified(&path).map_err(|error| {
            AppError::BadRequest(format!(
                "Agent Execution work_dir cannot be canonicalized '{}': {error}",
                path.display()
            ))
        })?;
        let metadata = tokio::fs::metadata(&canonical).await.map_err(|error| {
            AppError::BadRequest(format!(
                "Agent Execution work_dir cannot be inspected '{}': {error}",
                canonical.display()
            ))
        })?;
        if !metadata.is_dir() {
            return Err(AppError::BadRequest(format!(
                "Agent Execution work_dir is not a directory: {}",
                canonical.display()
            )));
        }
        if let Some(canonical_root) = canonical_root {
            if !canonical.starts_with(&canonical_root) {
                return Err(AppError::BadRequest(format!(
                    "relative Agent Execution work_dir resolves outside its execution root: {}",
                    canonical.display()
                )));
            }
        }
        let canonical_string = canonical.to_string_lossy().into_owned();
        if detail.execution.work_dir.as_deref() == Some(canonical_string.as_str()) {
            return Ok(false);
        }
        self.inner
            .deps
            .repository
            .update_execution(
                owner_id,
                &detail.execution.execution_id,
                detail.execution.version,
                Some(lease),
                &UpdateAgentExecutionParams {
                    work_dir: Some(Some(canonical_string)),
                    ..Default::default()
                },
                &system_event(
                    AgentExecutionEventKind::StatusChanged,
                    None,
                    None,
                    json!({"change":"work_dir_ready"}),
                ),
            )
            .await?;
        self.publish().await;
        Ok(true)
    }

    async fn skip_one_blocked_step(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<bool, AppError> {
        let active: HashMap<&str, &ExecutionStep> = detail
            .steps
            .iter()
            .filter(|step| step.superseded_in_revision.is_none())
            .map(|step| (step.step_id.as_str(), step))
            .collect();
        for step in active.values().filter(|step| step.status == ExecutionStepStatus::Pending) {
            let blocked = detail.dependencies.iter().any(|dependency| {
                dependency.superseded_in_revision.is_none()
                    && dependency.blocked_step_id == step.step_id
                    && active
                        .get(dependency.blocker_step_id.as_str())
                        .is_some_and(|blocker| {
                        matches!(
                            blocker.status,
                            ExecutionStepStatus::Failed
                                | ExecutionStepStatus::Skipped
                                | ExecutionStepStatus::Cancelled
                        )
                    })
            });
            if blocked {
                self.inner
                    .deps
                    .repository
                    .transition_step_status(
                        owner_id,
                        &detail.execution.execution_id,
                        &step.step_id,
                        detail.execution.version,
                        step.version,
                        Some(lease),
                        ExecutionStepStatus::Skipped,
                        &system_event(
                            AgentExecutionEventKind::StepChanged,
                            Some(&step.step_id),
                            None,
                            json!({"status":"skipped","reason":"dependency_failed"}),
                        ),
                    )
                    .await?;
                self.publish().await;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn execute_agent_step(
        &self,
        owner_id: &str,
        execution_id: &str,
        step: ExecutionStep,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<(), AppError> {
        let detail = self.detail(owner_id, execution_id).await?;
        if !matches!(
            detail.execution.status,
            AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput
        ) {
            return Ok(());
        }
        let Some(current_step) = detail
            .steps
            .iter()
            .find(|candidate| candidate.step_id == step.step_id && candidate.superseded_in_revision.is_none())
        else {
            return Ok(());
        };
        if current_step.status != ExecutionStepStatus::Pending {
            return Ok(());
        }
        let persisted_step = self
            .inner
            .deps
            .repository
            .get_step(owner_id, execution_id, &step.step_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Execution step {}", step.step_id)))?;
        if persisted_step.version != current_step.version
            || persisted_step.superseded_in_revision.is_some()
        {
            // The immutable private recursion marker belongs to the same Step
            // generation as the DTO snapshot. Reload on any race instead of
            // borrowing depth from a replacement generation.
            return Ok(());
        }
        let delegation_depth = persisted_step.delegation_depth;
        let participant = detail
            .participants
            .iter()
            .find(|participant| {
                current_step.assigned_participant_id.as_deref()
                    == Some(participant.participant_id.as_str())
                    && participant.retired_in_revision.is_none()
            })
            .cloned()
            .ok_or_else(|| AppError::BadRequest(format!("step {} has no active participant", step.step_id)))?;
        let model_pool = execution_model_pool(&detail.participants);
        let previous_attempts = detail
            .attempts
            .iter()
            .filter(|attempt| attempt.step_id == step.step_id)
            .count() as i64;
        let brief = compose_brief(&detail, current_step);
        let effective_config = json!({
            "participant_id": &participant.participant_id,
            "provider_id": &participant.provider_id,
            "model": &participant.model,
            "role": &current_step.role,
            "tool_policy": current_step.tool_policy,
            "delegation_policy": detail.execution.delegation_policy,
            "decision_policy": detail.execution.decision_policy,
            "timeout_ms": self.inner.deps.attempt_timeout.as_millis(),
        });
        let created = self
            .inner
            .deps
            .repository
            .create_attempt(
                owner_id,
                execution_id,
                &step.step_id,
                current_step.version,
                Some(lease),
                &CreateAgentExecutionAttemptParams {
                    participant_id: Some(participant.participant_id.clone()),
                    start_immediately: false,
                    trigger_reason: if previous_attempts == 0 { "initial" } else { "retry" }.to_owned(),
                    effective_config: effective_config.to_string(),
                    retry_after: None,
                    runtime_state: None,
                },
                &system_event(
                    AgentExecutionEventKind::AttemptChanged,
                    Some(&step.step_id),
                    None,
                    json!({"status":"queued"}),
                ),
            )
            .await?;
        self.publish().await;
        let created_attempt = created
            .current_attempt
            .as_ref()
            .ok_or_else(|| AppError::Internal("create_attempt returned no attempt".to_owned()))?;
        let attempt_id = created_attempt.attempt.attempt_id.clone();
        let conversation_slot = Arc::new(Mutex::new(None::<String>));
        let slot = conversation_slot.clone();
        let settlement_step_version = Arc::new(AtomicI64::new(created.step.version));
        let settlement_attempt_version =
            Arc::new(AtomicI64::new(created_attempt.attempt.version));
        let callback_step_version = settlement_step_version.clone();
        let callback_attempt_version = settlement_attempt_version.clone();
        let repository = self.inner.deps.repository.clone();
        let publisher = self.inner.deps.publisher.clone();
        let owner = owner_id.to_owned();
        let execution = execution_id.to_owned();
        let step_id = step.step_id.clone();
        let callback_attempt_id = attempt_id.clone();
        let expected_step_version = created.step.version;
        let expected_attempt_version = created_attempt.attempt.version;
        let callback_lease = lease.clone();
        let on_started = Box::new(move |conversation_id: String| {
            if let Ok(mut stored) = slot.lock() {
                *stored = Some(conversation_id.clone());
            }
            Box::pin(async move {
                let started = repository
                    .start_attempt(
                        &owner,
                        &execution,
                        &step_id,
                        expected_step_version,
                        &callback_attempt_id,
                        expected_attempt_version,
                        &conversation_id,
                        Some(&callback_lease),
                        &system_event(
                            AgentExecutionEventKind::AttemptChanged,
                            Some(&step_id),
                            Some(&callback_attempt_id),
                            json!({"status":"running"}),
                        ),
                    )
                    .await?;
                callback_step_version.store(started.step.version, Ordering::SeqCst);
                let started_attempt = started.current_attempt.as_ref().ok_or_else(|| {
                    nomifun_db::DbError::Conflict(
                        "started Agent attempt is missing from its step detail".to_owned(),
                    )
                })?;
                callback_attempt_version
                    .store(started_attempt.attempt.version, Ordering::SeqCst);
                publisher.drain(repository.clone()).await;
                Ok(AgentExecutionTurnAuthority {
                    execution_id: execution.clone(),
                    step_id: step_id.clone(),
                    attempt_id: callback_attempt_id.clone(),
                    expected_step_version: started.step.version,
                    expected_attempt_version: started_attempt.attempt.version,
                    lease_owner: callback_lease.owner().to_owned(),
                })
            }) as _
        });

        let outcome = self
            .inner
            .deps
            .attempt_runner
            .execute(
                owner_id,
                &participant,
                &model_pool,
                detail.execution.work_dir.as_deref(),
                &step.title,
                step.tool_policy,
                detail.execution.delegation_policy,
                delegation_depth,
                detail.execution.decision_policy,
                &attempt_id,
                &brief,
                &step.spec,
                self.inner.deps.attempt_timeout,
                on_started,
            )
            .await;
        let conversation_id = conversation_slot
            .lock()
            .ok()
            .and_then(|stored| stored.clone());
        if let Err(error) = &outcome
            && conversation_id.is_none()
        {
            tracing::warn!(%execution_id, step_id = %step.step_id, %error, "Agent attempt failed before starting");
        }
        if let Err(error) = &outcome {
            tracing::warn!(
                %execution_id,
                step_id = %step.step_id,
                attempt_id = %attempt_id,
                attempt_no = previous_attempts + 1,
                conversation_id = ?conversation_id,
                error = %error,
                error_code = error.error_code(),
                "Agent attempt execution returned an error"
            );
        }
        self.settle_agent_outcome(
            owner_id,
            execution_id,
            &step.step_id,
            &attempt_id,
            outcome,
            previous_attempts + 1,
            AttemptSettlementFence {
                step_version: settlement_step_version.load(Ordering::SeqCst),
                attempt_version: settlement_attempt_version.load(Ordering::SeqCst),
            },
            Some(lease),
        )
        .await
    }

    async fn settle_agent_outcome(
        &self,
        owner_id: &str,
        execution_id: &str,
        step_id: &str,
        attempt_id: &str,
        outcome: Result<AttemptOutcome, AppError>,
        attempt_no: i64,
        settlement_fence: AttemptSettlementFence,
        lease: Option<&AgentExecutionLeaseToken>,
    ) -> Result<(), AppError> {
        let detail = self.detail(owner_id, execution_id).await?;
        if detail.execution.status.is_terminal() {
            return Ok(());
        }
        let step = detail
            .steps
            .iter()
            .find(|step| step.step_id == step_id)
            .ok_or_else(|| AppError::NotFound(format!("Execution step {step_id}")))?;
        let attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .ok_or_else(|| AppError::NotFound(format!("Execution attempt {attempt_id}")))?;
        if attempt.status.is_terminal() || attempt.status == ExecutionAttemptStatus::WaitingInput {
            return Ok(());
        }
        // A concrete model turn owns exactly the Step/Attempt generations it
        // started with. A question, answer, pause, retry, or replacement bumps
        // either version; its late callback must never settle that successor.
        if step.version != settlement_fence.step_version
            || attempt.version != settlement_fence.attempt_version
        {
            return Ok(());
        }

        let (attempt_status, step_status, error, output, output_files, tokens, retry_after) = match outcome {
            Ok(outcome) if agent_outcome_can_complete(&outcome, &step.spec) => (
                ExecutionAttemptStatus::Completed,
                ExecutionStepStatus::Completed,
                None,
                outcome.text,
                outcome.output_files,
                outcome.tokens,
                None,
            ),
            Ok(outcome) => {
                let artifact_contract_error = outcome
                    .ok
                    .then(|| validate_required_artifacts(&step.spec, &outcome.output_files).err())
                    .flatten();
                let (retryable, has_marker, reason) = if let Some(error) =
                    artifact_contract_error
                {
                    // The turn itself finished, but its verified delivery did
                    // not satisfy the immutable Step requirement. This is a
                    // deterministic contract violation: replaying the same
                    // Step only creates another Attempt and can duplicate
                    // side effects without changing the contract.
                    (false, true, format!("Agent artifact delivery failed: {error}"))
                } else {
                    let retryable = match outcome.error_retryable {
                        Some(value) => value,
                        None => {
                            self.inner
                                .deps
                                .attempt_runner
                                .last_error_retryable(owner_id, &outcome.conversation_id)
                                .await
                        }
                    };
                    let has_marker = outcome.error.is_some()
                        || outcome.error_code.is_some()
                        || self
                            .inner
                            .deps
                            .attempt_runner
                            .last_error_present(owner_id, &outcome.conversation_id)
                            .await;
                    let reason = if let Some(error) = outcome.error.clone() {
                        error
                    } else if let Some(code) = outcome.error_code.clone() {
                        format!("Agent attempt failed ({code})")
                    } else if let Some(summary) = self
                        .inner
                        .deps
                        .attempt_runner
                        .last_error_summary(owner_id, &outcome.conversation_id)
                        .await
                    {
                        summary
                    } else if has_marker {
                        "Agent attempt failed".to_owned()
                    } else {
                        "Agent attempt timed out".to_owned()
                    };
                    let retry_class = attempt_outcome_retry_class(&outcome, has_marker, retryable);
                    let retryable = has_marker
                        && matches!(
                            retry_class,
                            AttemptRetryClass::Provider
                                | AttemptRetryClass::RateLimited
                                | AttemptRetryClass::Timeout
                        );
                    (
                        retryable,
                        has_marker,
                        reason,
                    )
                };
                tracing::warn!(
                    %execution_id,
                    %step_id,
                    %attempt_id,
                    attempt_no,
                    retryable,
                    has_marker,
                    reason = %reason,
                    "classifying Agent attempt outcome for settlement"
                );
                let can_retry = detail.execution.adaptation_policy == AdaptationPolicy::Adaptive
                    && ((retryable && attempt_no <= MAX_PROVIDER_RETRIES)
                        || (!has_marker && attempt_no <= MAX_TIMEOUT_RETRIES));
                (
                    ExecutionAttemptStatus::Failed,
                    if can_retry { ExecutionStepStatus::Pending } else { ExecutionStepStatus::Failed },
                    Some(reason),
                    None,
                    Vec::new(),
                    outcome.tokens,
                    can_retry.then(|| now_ms() + retry_backoff_ms(attempt_no)),
                )
            }
            Err(error) => {
                let (attempt_status, step_status, can_retry) = attempt_error_transition(
                    attempt.status,
                    detail.execution.adaptation_policy,
                    attempt_no,
                    attempt_error_is_retryable(&error),
                );
                (
                    attempt_status,
                    step_status,
                    Some(error.to_string()),
                    None,
                    Vec::new(),
                    None,
                    can_retry.then(|| now_ms() + retry_backoff_ms(attempt_no)),
                )
            }
        };
        let output_files = serde_json::to_string(&output_files)
            .map_err(|error| AppError::Internal(format!("encode verified attempt output files: {error}")))?;
        let settled = self.inner
            .deps
            .repository
            .settle_attempt(
                owner_id,
                execution_id,
                step_id,
                settlement_fence.step_version,
                attempt_id,
                settlement_fence.attempt_version,
                lease,
                &SettleAgentExecutionAttemptParams {
                    attempt_status,
                    step_status,
                    execution_status: None,
                    question: Some(None),
                    error: Some(error),
                    output_summary: Some(output),
                    output_files: Some(output_files),
                    tokens: Some(tokens),
                    retry_after: Some(retry_after),
                    runtime_state: Some(None),
                    started_at: None,
                    finished_at: Some(Some(now_ms())),
                    loop_repeat_reset: None,
                },
                &system_event(
                    AgentExecutionEventKind::AttemptChanged,
                    Some(step_id),
                    Some(attempt_id),
                    json!({"attempt_status":attempt_status,"step_status":step_status}),
                ),
            )
            .await;
        if let Err(nomifun_db::DbError::Conflict(_)) = &settled {
            let current = self
                .inner
                .deps
                .repository
                .get_step_detail(owner_id, execution_id, step_id)
                .await?;
            if current.as_ref().is_some_and(|current| {
                current.step.version != settlement_fence.step_version
                    || current.current_attempt.as_ref().is_none_or(|attempt| {
                        attempt.attempt.attempt_id != attempt_id
                            || attempt.attempt.version != settlement_fence.attempt_version
                    })
            }) {
                return Ok(());
            }
        }
        settled?;
        self.publish().await;
        self.reconcile_conversation_cleanup(Some(execution_id)).await;
        Ok(())
    }

    async fn execute_control_step(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        step: &ExecutionStep,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<(), AppError> {
        let dependencies: Vec<&ExecutionStep> = detail
            .dependencies
            .iter()
            .filter(|dependency| {
                dependency.superseded_in_revision.is_none() && dependency.blocked_step_id == step.step_id
            })
            .filter_map(|dependency| {
                detail
                    .steps
                    .iter()
                    .find(|candidate| candidate.step_id == dependency.blocker_step_id)
            })
            .collect();
        let resolution = control_steps::evaluate(step, &dependencies, &detail.attempts);
        let created = self
            .inner
            .deps
            .repository
            .create_attempt(
                owner_id,
                &detail.execution.execution_id,
                &step.step_id,
                step.version,
                Some(lease),
                &CreateAgentExecutionAttemptParams {
                    participant_id: None,
                    start_immediately: true,
                    trigger_reason: "control_evaluation".to_owned(),
                    effective_config: serde_json::to_string(&step.control_policy).map_err(|error| {
                        AppError::Internal(format!("encode control policy: {error}"))
                    })?,
                    retry_after: None,
                    runtime_state: None,
                },
                &system_event(
                    AgentExecutionEventKind::AttemptChanged,
                    Some(&step.step_id),
                    None,
                    json!({"status":"running","control":step.kind}),
                ),
            )
            .await?;
        self.publish().await;
        let current = created
            .current_attempt
            .as_ref()
            .ok_or_else(|| AppError::Internal("control attempt missing after create".to_owned()))?;
        let (attempt_status, step_status, summary, error, runtime_state, repeat_body) = match resolution {
            ControlResolution::Complete { summary, runtime_state } => (
                ExecutionAttemptStatus::Completed,
                ExecutionStepStatus::Completed,
                Some(summary),
                None,
                runtime_state,
                None,
            ),
            ControlResolution::Fail { summary, error, runtime_state } => (
                ExecutionAttemptStatus::Failed,
                ExecutionStepStatus::Failed,
                Some(summary),
                Some(error),
                runtime_state,
                None,
            ),
            ControlResolution::Repeat { body_step_id, runtime_state } => (
                ExecutionAttemptStatus::Completed,
                ExecutionStepStatus::Pending,
                Some("循环继续下一轮".to_owned()),
                None,
                Some(runtime_state),
                Some(body_step_id),
            ),
        };
        let loop_repeat_reset = repeat_body
            .map(|body_step_id| {
                build_loop_repeat_reset(detail, &step.step_id, &body_step_id)
            })
            .transpose()?;
        self.inner
            .deps
            .repository
            .settle_attempt(
                owner_id,
                &detail.execution.execution_id,
                &step.step_id,
                created.step.version,
                &current.attempt.attempt_id,
                current.attempt.version,
                Some(lease),
                &SettleAgentExecutionAttemptParams {
                    attempt_status,
                    step_status,
                    execution_status: None,
                    question: Some(None),
                    error: Some(error),
                    output_summary: Some(summary),
                    output_files: Some("[]".to_owned()),
                    tokens: Some(None),
                    retry_after: Some(None),
                    runtime_state: Some(
                        runtime_state
                            .map(|value| value.to_string()),
                    ),
                    started_at: None,
                    finished_at: Some(Some(now_ms())),
                    loop_repeat_reset,
                },
                &system_event(
                    AgentExecutionEventKind::AttemptChanged,
                    Some(&step.step_id),
                    Some(&current.attempt.attempt_id),
                    json!({"attempt_status":attempt_status,"step_status":step_status}),
                ),
            )
            .await?;
        self.publish().await;
        Ok(())
    }

    async fn finalize_if_settled(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        lease: &AgentExecutionLeaseToken,
    ) -> Result<bool, AppError> {
        let active: Vec<&ExecutionStep> = detail
            .steps
            .iter()
            .filter(|step| step.superseded_in_revision.is_none())
            .collect();
        if active.is_empty() || active.iter().any(|step| !step.status.is_terminal()) {
            return Ok(false);
        }
        let status = if active.iter().any(|step| {
            step.status == ExecutionStepStatus::Failed
                && step.failure_policy == StepFailurePolicy::FailExecution
        }) {
            AgentExecutionStatus::Failed
        } else if active.iter().any(|step| {
            matches!(
                step.status,
                ExecutionStepStatus::Failed
                    | ExecutionStepStatus::Skipped
                    | ExecutionStepStatus::Cancelled
            )
        }) {
            AgentExecutionStatus::CompletedWithFailures
        } else {
            AgentExecutionStatus::Completed
        };
        // Final-answer ownership is singular: reuse a completed synthesis, then
        // a single business step, otherwise persist a deterministic digest.
        // The Engine never performs an extra LLM summary. A lead Conversation,
        // when present, receives one idempotent assistant-message projection;
        // Remote callers read this persisted summary directly.
        let summary = terminal_summary(detail);
        self.inner
            .deps
            .repository
            .update_execution(
                owner_id,
                &detail.execution.execution_id,
                detail.execution.version,
                Some(lease),
                &UpdateAgentExecutionParams {
                    status: Some(status),
                    summary: Some(Some(summary)),
                    ..Default::default()
                },
                &system_event(
                    AgentExecutionEventKind::StatusChanged,
                    None,
                    None,
                    terminal_transition_payload(&detail.execution, status, None),
                ),
            )
            .await?;
        self.after_terminal_commit(owner_id, &detail.execution.execution_id).await;
        Ok(true)
    }

    async fn fail_if_active(
        &self,
        owner_id: &str,
        execution_id: &str,
        reason: &str,
        lease: &AgentExecutionLeaseToken,
    ) {
        if reason.trim().is_empty() {
            tracing::warn!(%execution_id, "refusing to persist an empty scheduler failure");
            return;
        }
        loop {
            let Ok(detail) = self.detail(owner_id, execution_id).await else {
                return;
            };
            if !status_is_active_for_scheduler_failure(detail.execution.status) {
                return;
            }
            match self
                .inner
                .deps
                .repository
                .fail_active_execution(
                    owner_id,
                    execution_id,
                    detail.execution.version,
                    lease,
                    reason,
                    &system_event(
                        AgentExecutionEventKind::StatusChanged,
                        None,
                        None,
                        terminal_transition_payload(
                            &detail.execution,
                            AgentExecutionStatus::Failed,
                            Some(reason),
                        ),
                    ),
                )
                .await
            {
                Ok(_) => break,
                Err(nomifun_db::DbError::Conflict(error)) => {
                    let current = match self
                        .inner
                        .deps
                        .repository
                        .get_execution(owner_id, execution_id)
                        .await
                    {
                        Ok(Some(current)) => current,
                        Ok(None) => return,
                        Err(load_error) => {
                            tracing::warn!(
                                %execution_id,
                                %load_error,
                                "failed to reload scheduler failure conflict"
                            );
                            return;
                        }
                    };
                    let status = match current.status.parse::<AgentExecutionStatus>() {
                        Ok(status) => status,
                        Err(status_error) => {
                            tracing::warn!(
                                %execution_id,
                                %status_error,
                                "scheduler failure reload found an invalid aggregate status"
                            );
                            return;
                        }
                    };
                    if !status_is_active_for_scheduler_failure(status) {
                        return;
                    }
                    if current.lease_owner.as_deref() != Some(lease.owner())
                        || current
                            .lease_expires_at
                            .is_none_or(|expires_at| expires_at <= now_ms())
                    {
                        tracing::warn!(
                            %execution_id,
                            "scheduler failure lost lease authority before commit"
                        );
                        return;
                    }
                    if current.version == detail.execution.version {
                        tracing::warn!(
                            %execution_id,
                            %error,
                            "scheduler failure conflicted without aggregate version drift"
                        );
                        return;
                    }
                    tracing::debug!(
                        %execution_id,
                        old_version = detail.execution.version,
                        new_version = current.version,
                        "retrying scheduler failure after concurrent active aggregate update"
                    );
                }
                Err(error) => {
                    tracing::warn!(%execution_id, %error, "failed to persist scheduler failure");
                    return;
                }
            }
        }
        self.after_terminal_commit(owner_id, execution_id).await;
    }

    async fn publish(&self) {
        self.inner
            .deps
            .publisher
            .drain(self.inner.deps.repository.clone())
            .await;
    }
}

async fn wait_for_cancel(cancelled: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    if *cancelled.borrow() {
        return true;
    }
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        changed = cancelled.changed() => changed.is_err() || *cancelled.borrow(),
    }
}

fn lease_retry_delay(owner: Option<&str>, expires_at: Option<i64>) -> Duration {
    let now = now_ms();
    if owner.is_some()
        && let Some(expires_at) = expires_at
        && expires_at > now
    {
        return Duration::from_millis(
            (expires_at - now + 25).clamp(
                LEASE_CAS_RETRY.as_millis() as i64,
                LEASE_ACQUIRE_RETRY_MAX.as_millis() as i64,
            ) as u64,
        );
    }
    LEASE_CAS_RETRY
}

fn next_effect_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(EFFECT_RETRY_MAX)
}

fn resolve_requested_work_dir(root: &Path, raw: &str) -> Result<PathBuf, AppError> {
    let original = raw;
    let raw = original.trim();
    if raw.is_empty() {
        return Err(AppError::BadRequest(
            "Agent Execution work_dir must not be empty".to_owned(),
        ));
    }
    if raw != original {
        return Err(AppError::BadRequest(
            "Agent Execution work_dir must not contain surrounding whitespace".to_owned(),
        ));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    // A Windows path supplied to a Unix host (or vice versa) must not become
    // an ordinary filename containing backslashes. Reject it explicitly
    // instead of silently creating the wrong directory.
    if (!cfg!(windows) && raw.contains('\\'))
        || raw
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':')
    {
        return Err(AppError::BadRequest(format!(
            "Agent Execution work_dir uses an incompatible path spelling: {raw}"
        )));
    }

    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(AppError::BadRequest(
                "relative Agent Execution work_dir must not escape its execution root".to_owned(),
            ));
        }
    }
    Ok(root.join(path))
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = path;
    loop {
        if candidate.exists() {
            return Some(candidate);
        }
        candidate = candidate.parent()?;
    }
}

fn scheduler_error_is_recoverable(error: &AppError) -> bool {
    match error {
        // Aggregate/step CAS drift, deletion, and lease fencing are ordinary
        // concurrent state changes. The successor must reload, never turn
        // them into a business failure.
        AppError::Conflict(_) | AppError::NotFound(_) => true,
        // Provider/transient transport errors are normally settled at the
        // attempt boundary; if one escapes before an attempt starts, reload.
        AppError::RateLimited | AppError::BadGateway(_) | AppError::Timeout(_) => true,
        // DB connectivity is infrastructure state, not execution outcome.
        AppError::Internal(message) => {
            message.starts_with("Database error:")
                || message.starts_with("Database init error:")
        }
        _ => false,
    }
}

fn status_is_active_for_scheduler_failure(status: AgentExecutionStatus) -> bool {
    matches!(
        status,
        AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput
    )
}

fn attempt_error_transition(
    status: ExecutionAttemptStatus,
    adaptation: AdaptationPolicy,
    attempt_no: i64,
    retryable_error: bool,
) -> (ExecutionAttemptStatus, ExecutionStepStatus, bool) {
    if status == ExecutionAttemptStatus::Queued {
        // No Conversation/model turn was started, so this is a dispatch
        // failure rather than a consumed model attempt. Retry it under a
        // small bounded start budget even for Fixed executions; exhausting
        // that budget fails the Step instead of spinning forever.
        let can_retry = retryable_error && attempt_no <= MAX_PROVIDER_RETRIES;
        return (
            ExecutionAttemptStatus::Cancelled,
            if can_retry {
                ExecutionStepStatus::Pending
            } else {
                ExecutionStepStatus::Failed
            },
            can_retry,
        );
    }
    let can_retry = retryable_error
        && adaptation == AdaptationPolicy::Adaptive
        && attempt_no <= MAX_PROVIDER_RETRIES;
    (
        ExecutionAttemptStatus::Failed,
        if can_retry {
            ExecutionStepStatus::Pending
        } else {
            ExecutionStepStatus::Failed
        },
        can_retry,
    )
}

fn attempt_error_is_retryable(error: &AppError) -> bool {
    match error {
        AppError::RateLimited | AppError::Timeout(_) => true,
        AppError::BadGateway(_) => {
            let stream_error = nomifun_ai_agent::AgentSendError::from_app_error_ref(error);
            stream_error.stream_error().retryable == Some(true)
                && stream_error
                    .stream_error()
                    .code
                    .is_some_and(is_transient_agent_error_code)
        }
        // Invalid workspace/model/input/config errors are deterministic. A
        // retry would create another Attempt with the same guaranteed failure
        // and is exactly the repeated-node behavior seen in the field report.
        _ => false,
    }
}

fn attempt_outcome_retry_class(
    outcome: &AttemptOutcome,
    has_marker: bool,
    retryable: bool,
) -> AttemptRetryClass {
    if !has_marker {
        // A completed provider turn without a terminal error marker has no
        // durable evidence of a deterministic rejection. Treat it as the
        // bounded timeout path, and never as an open-ended provider retry.
        return AttemptRetryClass::Timeout;
    }
    if !retryable {
        return AttemptRetryClass::Deterministic;
    }
    match outcome.error_code.as_deref() {
        Some("USER_LLM_PROVIDER_RATE_LIMITED") => AttemptRetryClass::RateLimited,
        Some("USER_LLM_PROVIDER_TIMEOUT") => AttemptRetryClass::Timeout,
        Some(code) if is_transient_agent_error_code_name(code) => AttemptRetryClass::Provider,
        _ if outcome
            .error
            .as_deref()
            .is_some_and(is_transient_provider_message) =>
        {
            AttemptRetryClass::Provider
        }
        _ => AttemptRetryClass::Deterministic,
    }
}

fn is_transient_agent_error_code(code: AgentErrorCode) -> bool {
    matches!(
        code,
        AgentErrorCode::UserLlmProviderGatewayError
            | AgentErrorCode::UserLlmProviderNetworkError
            | AgentErrorCode::UserLlmProviderEmptyResponse
            | AgentErrorCode::UserLlmProviderRateLimited
            | AgentErrorCode::UserLlmProviderTimeout
    )
}

fn is_transient_agent_error_code_name(code: &str) -> bool {
    matches!(
        code,
        "USER_LLM_PROVIDER_GATEWAY_ERROR"
            | "USER_LLM_PROVIDER_NETWORK_ERROR"
            | "USER_LLM_PROVIDER_EMPTY_RESPONSE"
            | "USER_LLM_PROVIDER_RATE_LIMITED"
            | "USER_LLM_PROVIDER_TIMEOUT"
    )
}

fn is_transient_provider_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "rate limit",
        "rate_limit",
        "quota",
        "timeout",
        "timed out",
        "deadline exceeded",
        "gateway",
        "network",
        "connection",
        "provider stream truncated",
        "empty response",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn agent_outcome_can_complete(outcome: &AttemptOutcome, step_spec: &str) -> bool {
    if !outcome.ok || validate_required_artifacts(step_spec, &outcome.output_files).is_err() {
        return false;
    }
    let has_text = outcome
        .text
        .as_ref()
        .is_some_and(|text| !text.trim().is_empty());
    has_text || (requires_artifact_delivery(step_spec) && !outcome.output_files.is_empty())
}

fn ready_steps(detail: &AgentExecutionDetail, now: i64) -> Vec<&ExecutionStep> {
    let active: HashMap<&str, &ExecutionStep> = detail
        .steps
        .iter()
        .filter(|step| step.superseded_in_revision.is_none())
        .map(|step| (step.step_id.as_str(), step))
        .collect();
    let mut ready: Vec<&ExecutionStep> = active
        .values()
        .filter(|step| step.status == ExecutionStepStatus::Pending)
        .filter(|step| step.dispatch_after.is_none_or(|ready_at| ready_at <= now))
        .filter(|step| {
            detail
                .dependencies
                .iter()
                .filter(|dependency| {
                    dependency.superseded_in_revision.is_none()
                        && dependency.blocked_step_id == step.step_id
                })
                .all(|dependency| {
                    active
                        .get(dependency.blocker_step_id.as_str())
                        .is_some_and(|blocker| blocker.status == ExecutionStepStatus::Completed)
                })
        })
        .copied()
        .collect();
    // HashMap traversal order must never decide which Agent receives one of the
    // bounded parallel slots. Persisted creation order plus id is stable across
    // process restarts and makes replay/recovery deterministic.
    ready.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.step_id.cmp(&right.step_id))
    });
    ready
}

fn build_loop_repeat_reset(
    detail: &AgentExecutionDetail,
    controller_step_id: &str,
    body_step_id: &str,
) -> Result<LoopRepeatResetParams, AppError> {
    let active: HashMap<&str, &ExecutionStep> = detail
        .steps
        .iter()
        .filter(|step| step.superseded_in_revision.is_none())
        .map(|step| (step.step_id.as_str(), step))
        .collect();
    if !active.contains_key(body_step_id) {
        return Err(AppError::Internal(format!(
            "Loop controller {controller_step_id} references missing body {body_step_id}"
        )));
    }
    let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
    for dependency in detail
        .dependencies
        .iter()
        .filter(|dependency| dependency.superseded_in_revision.is_none())
    {
        outgoing
            .entry(dependency.blocker_step_id.as_str())
            .or_default()
            .push(dependency.blocked_step_id.as_str());
    }
    if !outgoing
        .get(body_step_id)
        .is_some_and(|blocked| blocked.contains(&controller_step_id))
    {
        return Err(AppError::Internal(format!(
            "Loop body {body_step_id} is not a dependency of controller {controller_step_id}"
        )));
    }

    let mut closure = HashSet::from([body_step_id]);
    let mut queue = VecDeque::from([body_step_id]);
    while let Some(step_id) = queue.pop_front() {
        for downstream in outgoing.get(step_id).into_iter().flatten().copied() {
            if closure.insert(downstream) {
                queue.push_back(downstream);
            }
        }
    }
    closure.remove(controller_step_id);
    let mut expected_steps: Vec<RetryAgentExecutionStep> = closure
        .into_iter()
        .map(|step_id| {
            let step = active.get(step_id).ok_or_else(|| {
                AppError::Internal(format!(
                    "Loop reset closure references inactive step {step_id}"
                ))
            })?;
            Ok(RetryAgentExecutionStep {
                step_id: step.step_id.clone(),
                expected_step_version: step.version,
            })
        })
        .collect::<Result<_, AppError>>()?;
    expected_steps.sort_by(|left, right| left.step_id.cmp(&right.step_id));
    Ok(LoopRepeatResetParams {
        body_step_id: body_step_id.to_owned(),
        expected_steps,
    })
}

fn select_agent_steps(
    detail: &AgentExecutionDetail,
    ready: Vec<&ExecutionStep>,
    in_flight_step_ids: &HashSet<String>,
) -> Vec<ExecutionStep> {
    let participants: HashMap<&str, &ExecutionParticipant> = detail
        .participants
        .iter()
        .filter(|participant| participant.retired_in_revision.is_none())
        .map(|participant| (participant.participant_id.as_str(), participant))
        .collect();
    let current_steps: HashMap<&str, &ExecutionStep> = detail
        .steps
        .iter()
        .filter(|step| step.superseded_in_revision.is_none())
        .map(|step| (step.step_id.as_str(), step))
        .collect();
    let mut selected_per_participant: HashMap<&str, i64> = HashMap::new();
    let mut active_count = 0usize;
    let mut active_step_ids: HashSet<&str> = HashSet::new();
    for attempt in detail.attempts.iter().filter(|attempt| {
        matches!(
            attempt.status,
            ExecutionAttemptStatus::Queued | ExecutionAttemptStatus::Running
        )
    }) {
        let Some(participant_id) = current_steps
            .get(attempt.step_id.as_str())
            .and_then(|step| step.assigned_participant_id.as_deref())
        else {
            continue;
        };
        if !active_step_ids.insert(attempt.step_id.as_str()) {
            continue;
        }
        active_count += 1;
        *selected_per_participant.entry(participant_id).or_default() += 1;
    }
    // Futures are reserved before their first poll, so a just-pushed Step may
    // not have a Queued attempt in the freshly reloaded DB yet. Count that
    // process-local reservation exactly once and exclude it from selection.
    for step_id in in_flight_step_ids {
        if active_step_ids.contains(step_id.as_str()) {
            continue;
        }
        let Some(participant_id) = current_steps
            .get(step_id.as_str())
            .and_then(|step| step.assigned_participant_id.as_deref())
        else {
            continue;
        };
        active_count += 1;
        *selected_per_participant.entry(participant_id).or_default() += 1;
    }
    let mut selected = Vec::new();
    // Domain mapping rejects persisted values outside 1..=64. Do not silently
    // clamp corruption into a different execution policy here.
    let global_limit = detail.execution.max_parallel as usize;
    let available = global_limit.saturating_sub(active_count);
    if available == 0 {
        return selected;
    }
    for step in ready
        .into_iter()
        .filter(|step| step.kind == ExecutionStepKind::Agent)
        .filter(|step| !in_flight_step_ids.contains(&step.step_id))
    {
        let Some(participant_id) = step.assigned_participant_id.as_deref() else {
            continue;
        };
        let Some(participant) = participants.get(participant_id) else {
            continue;
        };
        let limit = participant
            .constraints
            .as_ref()
            .and_then(|constraints| constraints.max_concurrency)
            .unwrap_or(i64::MAX);
        let count = selected_per_participant.entry(participant_id).or_default();
        if *count >= limit {
            continue;
        }
        *count += 1;
        selected.push(step.clone());
        if selected.len() == available {
            break;
        }
    }
    selected
}

fn next_retry_at(detail: &AgentExecutionDetail) -> Option<i64> {
    detail
        .steps
        .iter()
        .filter(|step| {
            step.superseded_in_revision.is_none()
                && step.status == ExecutionStepStatus::Pending
        })
        .filter_map(|step| step.dispatch_after)
        .filter(|dispatch_after| *dispatch_after > now_ms())
        .min()
}

fn retry_backoff_ms(attempt_no: i64) -> i64 {
    1_000_i64.saturating_mul(1_i64 << attempt_no.clamp(0, 6))
}

fn lead_report_operation_id(execution_id: &str, terminal_event_sequence: i64) -> String {
    format!("exec-lead-report:{execution_id}:event:{terminal_event_sequence}")
}

pub(crate) fn terminal_transition_payload(
    execution: &AgentExecution,
    status: AgentExecutionStatus,
    reason: Option<&str>,
) -> serde_json::Value {
    let mut payload = json!({
        "status": status,
        "lead_report_operation_id": execution
            .lead_conversation_id
            .as_ref()
            .map(|_| lead_report_operation_id(&execution.execution_id, execution.event_sequence + 1)),
    });
    if let Some(reason) = reason {
        payload["reason"] = json!(reason);
    }
    payload
}

fn compose_brief(detail: &AgentExecutionDetail, step: &ExecutionStep) -> String {
    let mut brief = format!(
        "You are an Agent participating in a shared execution.\nGOAL: {}\nYOUR STEP: {}\n",
        detail.execution.goal, step.title
    );
    let mut blockers: Vec<&str> = detail
        .dependencies
        .iter()
        .filter(|dependency| {
            dependency.superseded_in_revision.is_none() && dependency.blocked_step_id == step.step_id
        })
        .map(|dependency| dependency.blocker_step_id.as_str())
        .collect();
    blockers.sort_unstable();
    blockers.dedup();
    if !blockers.is_empty() {
        brief.push_str("\nUPSTREAM RESULTS:\n");
        for blocker in blockers {
            let title = detail
                .steps
                .iter()
                .find(|candidate| candidate.step_id == blocker)
                .map(|candidate| candidate.title.as_str())
                .unwrap_or("unknown step");
            let output = detail
                .attempts
                .iter()
                .filter(|attempt| attempt.step_id == blocker)
                .max_by_key(|attempt| attempt.attempt_no)
                .and_then(|attempt| attempt.output_summary.as_deref())
                .unwrap_or("(no output)");
            brief.push_str(&format!("- {title}: {output}\n"));
        }
    }
    if let Some(previous) = detail
        .attempts
        .iter()
        .filter(|attempt| attempt.step_id == step.step_id)
        .max_by_key(|attempt| attempt.attempt_no)
        .and_then(|attempt| attempt.output_summary.as_deref())
    {
        brief.push_str("\nYOUR PREVIOUS ITERATION:\n");
        brief.push_str(previous);
        brief.push('\n');
    }
    if step.agent_mode == Some(AgentStepMode::Synthesis) {
        brief.push_str("\nSynthesize the upstream results into one coherent deliverable.\n");
    }
    if let Some(prompt) = step.preset_prompt.as_deref() {
        brief.push_str("\nSTEP-SPECIFIC RULES:\n");
        brief.push_str(prompt);
        brief.push('\n');
    }
    apply_agent_role_context(brief, step.role.as_deref())
}

fn terminal_summary(detail: &AgentExecutionDetail) -> String {
    let current_steps: Vec<&ExecutionStep> = detail
        .steps
        .iter()
        .filter(|step| step.superseded_in_revision.is_none())
        .collect();
    let latest_output = |step: &ExecutionStep| {
        detail
            .attempts
            .iter()
            .filter(|attempt| attempt.step_id == step.step_id)
            .max_by_key(|attempt| attempt.attempt_no)
            .and_then(|attempt| attempt.output_summary.as_deref())
            .map(str::trim)
            .filter(|output| !output.is_empty())
    };
    let business_step_ids: Vec<&str> = current_steps
        .iter()
        .filter(|step| step.kind == ExecutionStepKind::Agent)
        .filter(|step| step.agent_mode != Some(AgentStepMode::Synthesis))
        .map(|step| step.step_id.as_str())
        .collect();
    let active_edges: Vec<(&str, &str)> = detail
        .dependencies
        .iter()
        .filter(|dependency| dependency.superseded_in_revision.is_none())
        .map(|dependency| {
            (
                dependency.blocker_step_id.as_str(),
                dependency.blocked_step_id.as_str(),
            )
        })
        .collect();
    if let Some(summary) = current_steps
        .iter()
        .rev()
        .find(|step| {
            step.status == ExecutionStepStatus::Completed
                && step.agent_mode == Some(AgentStepMode::Synthesis)
                && dependency_ancestors_cover(
                    &step.step_id,
                    &business_step_ids,
                    &active_edges,
                )
        })
        .and_then(|step| latest_output(step))
    {
        return summary.to_owned();
    }
    let business_steps: Vec<&ExecutionStep> = current_steps
        .into_iter()
        .filter(|step| step.kind == ExecutionStepKind::Agent)
        .filter(|step| step.agent_mode != Some(AgentStepMode::Synthesis))
        .collect();
    if business_steps.len() == 1
        && let Some(summary) = latest_output(business_steps[0])
    {
        return summary.to_owned();
    }
    aggregate_summary(detail)
}

/// A synthesis owns the final answer only when its transitive input closure
/// contains every current business Agent step. This matters for dynamic
/// delegation: work appended after an older synthesis must not disappear from
/// the terminal projection merely because that synthesis completed earlier.
fn dependency_ancestors_cover(
    sink_id: &str,
    required_ids: &[&str],
    edges: &[(&str, &str)],
) -> bool {
    let mut ancestors = HashSet::new();
    let mut frontier = vec![sink_id];
    while let Some(blocked_id) = frontier.pop() {
        for (blocker_id, candidate_blocked_id) in edges {
            if *candidate_blocked_id == blocked_id && ancestors.insert(*blocker_id) {
                frontier.push(*blocker_id);
            }
        }
    }
    required_ids.iter().all(|id| ancestors.contains(*id))
}

fn aggregate_summary(detail: &AgentExecutionDetail) -> String {
    let mut lines = Vec::new();
    for step in detail
        .steps
        .iter()
        .filter(|step| step.superseded_in_revision.is_none())
    {
        let output = detail
            .attempts
            .iter()
            .filter(|attempt| attempt.step_id == step.step_id)
            .max_by_key(|attempt| attempt.attempt_no)
            .and_then(|attempt| attempt.output_summary.as_deref())
            .unwrap_or("-");
        lines.push(format!(
            "{} | {} | {}",
            step.title,
            step.status,
            output.chars().take(800).collect::<String>()
        ));
    }
    lines.join("\n")
}

fn execution_model_pool(participants: &[ExecutionParticipant]) -> Vec<ExecutionModelRef> {
    let mut seen = HashSet::new();
    participants
        .iter()
        .filter(|participant| participant.retired_in_revision.is_none())
        .filter_map(|participant| {
            let provider_id = participant.provider_id.as_ref()?;
            let model = participant.model.as_ref()?;
            let key = (provider_id.clone(), model.clone());
            seen.insert(key.clone()).then_some(ExecutionModelRef {
                provider_id: key.0,
                model: key.1,
            })
        })
        .collect()
}

pub(crate) fn system_event(
    kind: AgentExecutionEventKind,
    step_id: Option<&str>,
    attempt_id: Option<&str>,
    payload: serde_json::Value,
) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent {
        event_type: kind,
        step_id: step_id.map(str::to_owned),
        attempt_id: attempt_id.map(str::to_owned),
        actor: nomifun_common::AgentExecutionActor::system(),
        payload: payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Instant as StdInstant;

    use async_trait::async_trait;
    use nomifun_api_types::WebSocketMessage;
    use nomifun_common::{AgentToolPolicy, DecisionPolicy, DelegationPolicy};
    use nomifun_db::{
        CreateAgentExecutionParams, NewAgentExecutionParticipant, NewAgentExecutionStep,
        NewAgentExecutionStepDependency, ReconcileAgentExecutionPlanParams,
        SqliteAgentExecutionRepository, SqliteConversationRepository, IConversationRepository,
    };
    use nomifun_db::models::ConversationRow;
    use nomifun_realtime::UserEventSink;
    use tempfile::{TempDir, tempdir};
    use tokio::sync::Barrier;

    #[derive(Debug)]
    struct TestCleanup {
        link_id: i64,
        user_id: String,
        conversation_id: String,
    }

    #[tokio::test]
    async fn cleanup_batch_keeps_current_generation_when_stale_generation_is_listed_first() {
        let validated = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(Mutex::new(Vec::new()));
        let acknowledged = Arc::new(Mutex::new(Vec::new()));

        assert!(
            reconcile_cleanup_batch(
                vec![
                    TestCleanup {
                        link_id: 1,
                        user_id: "user-1".to_owned(),
                        conversation_id: "conversation-1".to_owned(),
                    },
                    TestCleanup {
                        link_id: 2,
                        user_id: "user-1".to_owned(),
                        conversation_id: "conversation-1".to_owned(),
                    },
                ],
                |cleanup| (cleanup.user_id.clone(), cleanup.conversation_id.clone()),
                {
                    let validated = validated.clone();
                    move |cleanup: &TestCleanup| {
                        validated.lock().unwrap().push(cleanup.link_id);
                        let validation = if cleanup.link_id == 2 {
                            CleanupValidation::Current
                        } else {
                            CleanupValidation::Stale
                        };
                        Box::pin(async move { validation })
                    }
                },
                {
                    let cancelled = cancelled.clone();
                    move |cleanup: &TestCleanup| {
                        let cancelled = cancelled.clone();
                        let key = (cleanup.user_id.clone(), cleanup.conversation_id.clone());
                        Box::pin(async move {
                            cancelled.lock().unwrap().push(key);
                            true
                        })
                    }
                },
                {
                    let acknowledged = acknowledged.clone();
                    move |cleanup: &TestCleanup| {
                        let acknowledged = acknowledged.clone();
                        let link_id = cleanup.link_id;
                        Box::pin(async move {
                            acknowledged.lock().unwrap().push(link_id);
                            true
                        })
                    }
                },
            )
            .await
        );

        assert_eq!(*validated.lock().unwrap(), vec![1, 2]);
        assert_eq!(
            *cancelled.lock().unwrap(),
            vec![("user-1".to_owned(), "conversation-1".to_owned())]
        );
        assert_eq!(*acknowledged.lock().unwrap(), vec![2]);
    }

    #[tokio::test]
    async fn generation_fence_dispatches_cancel_only_while_the_link_is_still_current() {
        let cancelled = Arc::new(Mutex::new(0_u32));

        // Still current at dispatch time: the cancel effect runs.
        let fence_cancelled = cancelled.clone();
        assert!(
            cancel_with_generation_fence(async { Ok::<bool, AppError>(true) }, async || {
                *fence_cancelled.lock().unwrap() += 1;
                true
            })
            .await
        );
        assert_eq!(*cancelled.lock().unwrap(), 1);

        // A replacement claimed the Conversation after batch validation: the
        // conversation-scoped cancel must be skipped, not delivered into the
        // replacement's live turn.
        let fence_cancelled = cancelled.clone();
        assert!(
            !cancel_with_generation_fence(async { Ok::<bool, AppError>(false) }, async || {
                *fence_cancelled.lock().unwrap() += 1;
                true
            })
            .await
        );
        assert_eq!(*cancelled.lock().unwrap(), 1);

        // Revalidation failure leaves the row pending without dispatching.
        let fence_cancelled = cancelled.clone();
        assert!(
            !cancel_with_generation_fence(
                async { Err::<bool, AppError>(AppError::Internal("fixture".to_owned())) },
                async || {
                    *fence_cancelled.lock().unwrap() += 1;
                    true
                },
            )
            .await
        );
        assert_eq!(*cancelled.lock().unwrap(), 1);
    }

    #[test]
    fn persistent_role_context_path_uses_the_shared_prompt_contract() {
        let source = include_str!("scheduler.rs");
        // Split the needle so the assertion cannot satisfy itself merely by
        // embedding the exact production call as a test string literal.
        let required_call = [
            "apply_agent_role_context",
            "(brief, step.role.as_deref())",
        ]
        .concat();
        assert_eq!(source.matches(&required_call).count(), 1);
    }

    #[test]
    fn pre_start_errors_retry_without_consuming_fixed_model_policy() {
        let (attempt, step, retry) = attempt_error_transition(
            ExecutionAttemptStatus::Queued,
            AdaptationPolicy::Fixed,
            1,
            true,
        );
        assert_eq!(attempt, ExecutionAttemptStatus::Cancelled);
        assert_eq!(step, ExecutionStepStatus::Pending);
        assert!(retry);

        let (attempt, step, retry) = attempt_error_transition(
            ExecutionAttemptStatus::Queued,
            AdaptationPolicy::Fixed,
            MAX_PROVIDER_RETRIES + 1,
            true,
        );
        assert_eq!(attempt, ExecutionAttemptStatus::Cancelled);
        assert_eq!(step, ExecutionStepStatus::Failed);
        assert!(!retry);
    }

    #[test]
    fn errors_after_start_follow_the_adaptation_policy() {
        let (_, fixed_step, fixed_retry) = attempt_error_transition(
            ExecutionAttemptStatus::Running,
            AdaptationPolicy::Fixed,
            1,
            true,
        );
        assert_eq!(fixed_step, ExecutionStepStatus::Failed);
        assert!(!fixed_retry);

        let (_, adaptive_step, adaptive_retry) = attempt_error_transition(
            ExecutionAttemptStatus::Running,
            AdaptationPolicy::Adaptive,
            1,
            true,
        );
        assert_eq!(adaptive_step, ExecutionStepStatus::Pending);
        assert!(adaptive_retry);
    }

    #[test]
    fn deterministic_dispatch_errors_do_not_repeat_the_same_step() {
        let (_, step, retry) = attempt_error_transition(
            ExecutionAttemptStatus::Queued,
            AdaptationPolicy::Adaptive,
            1,
            false,
        );
        assert_eq!(step, ExecutionStepStatus::Failed);
        assert!(!retry);
        assert!(!attempt_error_is_retryable(&AppError::BadRequest(
            "workspace does not exist".to_owned()
        )));
        assert!(attempt_error_is_retryable(&AppError::BadGateway(
            "provider stream protocol violation".to_owned()
        )));
        assert!(!attempt_error_is_retryable(&AppError::BadGateway(
            "Nomi agent error: API error: provider stream protocol violation: tool progress 'Bash' was not advertised in this request"
                .to_owned()
        )));
        assert!(attempt_error_is_retryable(&AppError::RateLimited));
        assert!(attempt_error_is_retryable(&AppError::Timeout(
            "provider did not respond".to_owned()
        )));
        assert!(!attempt_error_is_retryable(&AppError::BadGateway(
            "invalid provider request".to_owned()
        )));
    }

    #[test]
    fn artifact_contract_mismatch_is_deterministic_and_never_retryable() {
        let outcome = AttemptOutcome {
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".to_owned(),
            text: Some("done".to_owned()),
            output_files: vec!["/workspace/result.jpg".to_owned()],
            ok: true,
            tokens: Some(1),
            error: None,
            error_code: None,
            error_retryable: None,
        };
        assert!(!agent_outcome_can_complete(&outcome, "Generate 2 PNG images"));
        assert_eq!(
            attempt_outcome_retry_class(&outcome, true, true),
            AttemptRetryClass::Deterministic
        );
    }

    #[test]
    fn only_explicit_transient_provider_outcomes_are_retryable() {
        let transient = |error_code: &str| AttemptOutcome {
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000202".to_owned(),
            text: None,
            output_files: Vec::new(),
            ok: false,
            tokens: None,
            error: Some("provider failed".to_owned()),
            error_code: Some(error_code.to_owned()),
            error_retryable: Some(true),
        };
        assert_eq!(
            attempt_outcome_retry_class(
                &transient("USER_LLM_PROVIDER_GATEWAY_ERROR"),
                true,
                true
            ),
            AttemptRetryClass::Provider
        );
        assert_eq!(
            attempt_outcome_retry_class(&transient("USER_LLM_PROVIDER_RATE_LIMITED"), true, true),
            AttemptRetryClass::RateLimited
        );
        assert_eq!(
            attempt_outcome_retry_class(&transient("USER_LLM_PROVIDER_TIMEOUT"), true, true),
            AttemptRetryClass::Timeout
        );
        assert_eq!(
            attempt_outcome_retry_class(
                &transient("USER_LLM_PROVIDER_INVALID_REQUEST"),
                true,
                true
            ),
            AttemptRetryClass::Deterministic
        );
        assert_eq!(
            attempt_outcome_retry_class(
                &transient("NOMIFUN_TOOL_RESULT_ENCODING_ERROR"),
                true,
                true
            ),
            AttemptRetryClass::Deterministic
        );

        let timeout_without_marker = transient("USER_LLM_PROVIDER_TIMEOUT");
        assert_eq!(
            attempt_error_transition(
                ExecutionAttemptStatus::Failed,
                AdaptationPolicy::Adaptive,
                1,
                false,
            ),
            (
                ExecutionAttemptStatus::Failed,
                ExecutionStepStatus::Failed,
                false
            )
        );
        assert_eq!(
            attempt_outcome_retry_class(&timeout_without_marker, false, true),
            AttemptRetryClass::Timeout
        );
    }

    #[test]
    fn failed_or_textless_agent_outcome_can_never_complete() {
        let outcome = |ok, text: Option<&str>| AttemptOutcome {
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".to_owned(),
            text: text.map(str::to_owned),
            output_files: vec!["/untrusted/stale-output.png".to_owned()],
            ok,
            tokens: None,
            error: None,
            error_code: None,
            error_retryable: None,
        };

        // Even a stale/concurrent assistant result cannot override ok=false.
        let non_artifact_spec = "Analyze the issue and answer in chat";
        assert!(!agent_outcome_can_complete(
            &outcome(false, Some("another turn completed")),
            non_artifact_spec,
        ));
        assert!(!agent_outcome_can_complete(
            &outcome(true, None),
            non_artifact_spec,
        ));
        assert!(!agent_outcome_can_complete(
            &outcome(true, Some("  \n")),
            non_artifact_spec,
        ));
        assert!(agent_outcome_can_complete(
            &outcome(true, Some("authoritative receipt output")),
            non_artifact_spec,
        ));
    }

    #[test]
    fn relative_execution_workspaces_are_rooted_and_traversal_is_rejected() {
        let root = Path::new("/tmp/agent-execution-root");
        assert_eq!(
            resolve_requested_work_dir(root, "multi-agent-test").unwrap(),
            root.join("multi-agent-test")
        );
        assert_eq!(
            resolve_requested_work_dir(root, "./multi-agent-test").unwrap(),
            root.join("./multi-agent-test")
        );
        assert!(resolve_requested_work_dir(root, "../outside").is_err());
        assert!(resolve_requested_work_dir(root, "nested/../outside").is_err());
        if cfg!(windows) {
            assert_eq!(
                resolve_requested_work_dir(root, r"nested\outside").unwrap(),
                root.join(r"nested\outside")
            );
            assert_eq!(
                resolve_requested_work_dir(root, r"C:\outside").unwrap(),
                PathBuf::from(r"C:\outside")
            );
        } else {
            assert!(resolve_requested_work_dir(root, r"nested\outside").is_err());
            assert!(resolve_requested_work_dir(root, r"C:\outside").is_err());
        }
    }

    #[test]
    fn artifact_step_cannot_complete_on_text_or_insufficient_files() {
        let outcome = |output_files: Vec<String>| AttemptOutcome {
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".to_owned(),
            text: Some("done".to_owned()),
            output_files,
            ok: true,
            tokens: None,
            error: None,
            error_code: None,
            error_retryable: None,
        };

        assert!(!agent_outcome_can_complete(
            &outcome(Vec::new()),
            "Generate 2 PNG images",
        ));
        assert!(!agent_outcome_can_complete(
            &outcome(vec!["/workspace/one.png".to_owned(), "/workspace/two.jpg".to_owned()]),
            "Generate 2 PNG images",
        ));
        assert!(agent_outcome_can_complete(
            &outcome(vec!["/workspace/one.png".to_owned(), "/workspace/two.png".to_owned()]),
            "Generate 2 PNG images",
        ));
    }

    #[test]
    fn lease_retry_is_bounded_and_conflicts_are_recoverable() {
        assert_eq!(lease_retry_delay(None, None), LEASE_CAS_RETRY);
        assert!(
            lease_retry_delay(
                Some("0190f5fe-7c00-7a00-8000-000000000301"),
                Some(now_ms() + 60_000)
            )
                <= LEASE_ACQUIRE_RETRY_MAX
        );
        assert!(scheduler_error_is_recoverable(&AppError::Conflict(
            "lease changed".to_owned()
        )));
        assert!(!scheduler_error_is_recoverable(&AppError::BadRequest(
            "invalid persisted graph".to_owned()
        )));
    }

    #[test]
    fn fatal_scheduler_errors_settle_every_active_aggregate_state() {
        assert!(status_is_active_for_scheduler_failure(
            AgentExecutionStatus::Running
        ));
        assert!(status_is_active_for_scheduler_failure(
            AgentExecutionStatus::WaitingInput
        ));
        for inactive in [
            AgentExecutionStatus::Planning,
            AgentExecutionStatus::AwaitingApproval,
            AgentExecutionStatus::Paused,
            AgentExecutionStatus::Completed,
            AgentExecutionStatus::CompletedWithFailures,
            AgentExecutionStatus::Failed,
            AgentExecutionStatus::Cancelled,
        ] {
            assert!(!status_is_active_for_scheduler_failure(inactive));
        }
    }

    #[test]
    fn synthesis_must_cover_work_appended_after_it() {
        let business = [
            "0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-7a00-8000-000000000002",
            "0190f5fe-7c00-7a00-8000-000000000003",
        ];
        let synthesis = "0190f5fe-7c00-7a00-8000-000000000004";
        let replacement = "0190f5fe-7c00-7a00-8000-000000000005";
        let old_edges = [(business[0], synthesis), (business[1], synthesis)];
        assert!(!dependency_ancestors_cover(
            synthesis,
            &business,
            &old_edges,
        ));

        let complete_edges = [
            (business[0], replacement),
            (business[1], replacement),
            (business[2], replacement),
            (replacement, synthesis),
        ];
        assert!(dependency_ancestors_cover(
            synthesis,
            &business,
            &complete_edges,
        ));
    }

    // ---------------------------------------------------------------------
    // Deterministic scheduler harness
    //
    // These tests deliberately exercise the real SQLite repository and the
    // real scheduler loop. Only the external model/conversation boundary is
    // replaced, so a green result proves the durable DAG/lease/attempt
    // transitions rather than only the pure selection helpers.

    const HARNESS_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000902";
    const HARNESS_SOURCE_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";

    struct NoopUserEventSink;

    impl UserEventSink for NoopUserEventSink {
        fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
    }

    struct NoopConversationEffects;

    #[async_trait]
    impl ConversationEffects for NoopConversationEffects {
        async fn cancel_attempt(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn steer_attempt(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _text: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn stop_attempt_turn(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn report_lead(
            &self,
            _owner_id: &str,
            _detail: &AgentExecutionDetail,
            _operation_id: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[derive(Clone)]
    enum HarnessRunnerMode {
        ParallelRoots {
            barrier: Arc<Barrier>,
            downstream_started_too_early: Arc<AtomicBool>,
        },
        DeterministicFailure {
            failed_step: String,
        },
        RetryOnce {
            remaining_failures: Arc<AtomicUsize>,
        },
    }

    #[derive(Clone)]
    struct HarnessAttemptRunner {
        mode: HarnessRunnerMode,
        calls: Arc<Mutex<Vec<String>>>,
        workspace_dirs: Arc<Mutex<Vec<Option<String>>>>,
        completed_successes: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        conversation_repo: Arc<Mutex<Option<SqliteConversationRepository>>>,
        owner_id: Arc<Mutex<Option<String>>>,
    }

    impl HarnessAttemptRunner {
        fn parallel() -> Self {
            Self {
                mode: HarnessRunnerMode::ParallelRoots {
                    barrier: Arc::new(Barrier::new(2)),
                    downstream_started_too_early: Arc::new(AtomicBool::new(false)),
                },
                calls: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                conversation_repo: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn deterministic_failure(failed_step: &str) -> Self {
            Self {
                mode: HarnessRunnerMode::DeterministicFailure {
                    failed_step: failed_step.to_owned(),
                },
                calls: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                conversation_repo: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn retry_once() -> Self {
            Self {
                mode: HarnessRunnerMode::RetryOnce {
                    remaining_failures: Arc::new(AtomicUsize::new(1)),
                },
                calls: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                conversation_repo: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn bind_conversation_repo(&self, repository: SqliteConversationRepository) {
            *self
                .conversation_repo
                .lock()
                .expect("harness conversation repository is not poisoned") = Some(repository);
        }

        fn bind_owner(&self, owner_id: String) {
            *self
                .owner_id
                .lock()
                .expect("harness owner is not poisoned") = Some(owner_id);
        }

        fn call_count(&self, title: &str) -> usize {
            self.calls
                .lock()
                .expect("harness call log is not poisoned")
                .iter()
                .filter(|called| called.as_str() == title)
                .count()
        }

        fn workspace_dirs(&self) -> Vec<Option<String>> {
            self.workspace_dirs
                .lock()
                .expect("harness workspace log is not poisoned")
                .clone()
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        fn downstream_started_too_early(&self) -> bool {
            match &self.mode {
                HarnessRunnerMode::ParallelRoots {
                    downstream_started_too_early,
                    ..
                } => downstream_started_too_early.load(Ordering::SeqCst),
                _ => false,
            }
        }
    }

    fn update_max_active(max_active: &AtomicUsize, candidate: usize) {
        let mut observed = max_active.load(Ordering::SeqCst);
        while candidate > observed {
            match max_active.compare_exchange(
                observed,
                candidate,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }

    #[async_trait]
    impl AttemptRunner for HarnessAttemptRunner {
        #[allow(clippy::too_many_arguments)]
        async fn execute(
            &self,
            _owner_id: &str,
            _participant: &ExecutionParticipant,
            _execution_model_pool: &[ExecutionModelRef],
            workspace_dir: Option<&str>,
            step_title: &str,
            _tool_policy: AgentToolPolicy,
            _delegation_policy: DelegationPolicy,
            _delegation_depth: i64,
            _decision_policy: DecisionPolicy,
            _attempt_creation_key: &str,
            _brief: &str,
            _step_spec: &str,
            _timeout: Duration,
            on_started: crate::attempt_runner::AttemptStarted,
        ) -> Result<AttemptOutcome, AppError> {
            self.workspace_dirs
                .lock()
                .expect("harness workspace log is not poisoned")
                .push(workspace_dir.map(str::to_owned));
            let conversation_id = nomifun_common::ConversationId::new().into_string();
            let now = now_ms();
            let owner_id = self
                .owner_id
                .lock()
                .expect("harness owner is not poisoned")
                .clone()
                .ok_or_else(|| AppError::Internal("scheduler harness owner is missing".into()))?;
            let conversation = ConversationRow {
                id: 0,
                conversation_id: conversation_id.clone(),
                user_id: owner_id,
                name: format!("Scheduler harness · {step_title}"),
                r#type: "nomi".to_owned(),
                extra: "{}".to_owned(),
                delegation_policy: "automatic".to_owned(),
                execution_model_pool: None,
                decision_policy: "automatic".to_owned(),
                execution_template_id: None,
                model: None,
                status: Some("pending".to_owned()),
                source: Some("nomifun".to_owned()),
                channel_chat_id: None,
                pinned: false,
                pinned_at: None,
                cron_job_id: None,
                preset_id: None,
                preset_revision: None,
                preset_snapshot: None,
                created_at: now,
                updated_at: now,
            };
            let repository = self
                .conversation_repo
                .lock()
                .expect("harness conversation repository is not poisoned")
                .clone()
                .ok_or_else(|| {
                    AppError::Internal("scheduler harness conversation repository is missing".into())
                })?;
            repository
                .create(&conversation)
                .await
                .map_err(|error| AppError::Internal(format!("create harness conversation: {error}")))?;
            on_started(conversation_id.clone()).await?;

            self.calls
                .lock()
                .expect("harness call log is not poisoned")
                .push(step_title.to_owned());
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            update_max_active(&self.max_active, active);

            let finish = |runner: &HarnessAttemptRunner| {
                runner.active.fetch_sub(1, Ordering::SeqCst);
            };
            let failure = match &self.mode {
                HarnessRunnerMode::DeterministicFailure { failed_step } => {
                    step_title == failed_step
                }
                HarnessRunnerMode::RetryOnce {
                    remaining_failures,
                } => remaining_failures
                    .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok(),
                HarnessRunnerMode::ParallelRoots { .. } => false,
            };
            if failure {
                let (error, error_code, retryable) = match &self.mode {
                    HarnessRunnerMode::RetryOnce { .. } => (
                        "temporary deterministic harness provider failure",
                        "USER_LLM_PROVIDER_GATEWAY_ERROR",
                        true,
                    ),
                    _ => (
                        "The model returned a tool that was not advertised for this request",
                        "USER_LLM_PROVIDER_GATEWAY_ERROR",
                        false,
                    ),
                };
                finish(self);
                return Ok(AttemptOutcome {
                    conversation_id: format!("harness-{step_title}"),
                    text: None,
                    output_files: Vec::new(),
                    ok: false,
                    tokens: None,
                    error: Some(error.to_owned()),
                    error_code: Some(error_code.to_owned()),
                    error_retryable: Some(retryable),
                });
            }

            if let HarnessRunnerMode::ParallelRoots {
                barrier,
                downstream_started_too_early,
            } = &self.mode
            {
                if step_title == "downstream" {
                    if self.completed_successes.load(Ordering::SeqCst) < 2 {
                        downstream_started_too_early.store(true, Ordering::SeqCst);
                    }
                } else if matches!(step_title, "upstream-a" | "upstream-b") {
                    barrier.wait().await;
                }
            }

            self.completed_successes.fetch_add(1, Ordering::SeqCst);
            finish(self);
            Ok(AttemptOutcome {
                conversation_id: format!("harness-{step_title}"),
                text: Some(format!("completed {step_title}")),
                output_files: Vec::new(),
                ok: true,
                tokens: Some(1),
                error: None,
                error_code: None,
                error_retryable: None,
            })
        }
    }

    fn harness_participant(participant_id: String) -> NewAgentExecutionParticipant {
        NewAgentExecutionParticipant {
            participant_id,
            source_agent_id: HARNESS_SOURCE_AGENT_ID.to_owned(),
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            provider_id: Some(HARNESS_PROVIDER_ID.to_owned()),
            model: Some("harness-model".to_owned()),
            role: Some("builder".to_owned()),
            capability: None,
            constraints: None,
            description: Some("deterministic scheduler harness".to_owned()),
            system_prompt: None,
            enabled_skills: "[]".to_owned(),
            disabled_builtin_skills: "[]".to_owned(),
            sort_order: 0,
        }
    }

    fn harness_step(step_id: String, participant_id: &str, title: &str) -> NewAgentExecutionStep {
        NewAgentExecutionStep {
            step_id,
            title: title.to_owned(),
            spec: format!("Complete {title} in text."),
            role: Some("builder".to_owned()),
            tool_policy: AgentToolPolicy::Full,
            kind: ExecutionStepKind::Agent,
            agent_mode: Some(AgentStepMode::Normal),
            profile: Some(
                r#"{"kind":"research","needs_vision":false,"needs_web_search":false,"needs_long_context":false,"needs_high_reasoning":false,"bulk":false}"#
                    .to_owned(),
            ),
            fanout_group: None,
            control_policy: None,
            status: ExecutionStepStatus::Pending,
            assigned_participant_id: Some(participant_id.to_owned()),
            assignment_score: Some(1.0),
            assignment_rationale: Some("deterministic harness".to_owned()),
            assignment_source: Some(nomifun_common::ParticipantAssignmentSource::Planner),
            assignment_locked: false,
            failure_policy: StepFailurePolicy::FailExecution,
            preset_prompt: None,
            graph_x: None,
            graph_y: None,
        }
    }

    async fn make_scheduler_harness(
        runner: Arc<HarnessAttemptRunner>,
        titles: &[&str],
        dependencies: &[(&str, &str)],
        max_parallel: i64,
        adaptation_policy: AdaptationPolicy,
        work_dir: Option<&str>,
    ) -> (
        ExecutionScheduler,
        Arc<SqliteAgentExecutionRepository>,
        String,
        TempDir,
        String,
    ) {
        let data_dir = tempdir().expect("scheduler data directory");
        let database = nomifun_db::init_database(&data_dir.path().join("harness.sqlite"))
            .await
            .expect("file database");
        let owner = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        runner.bind_owner(owner.clone());
        sqlx::query(
            "INSERT INTO providers (\
                provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                created_at, updated_at\
             ) VALUES (?, 'openai', 'Scheduler harness provider', 'https://example.invalid', \
                       'bearer', '', 1, 1, 1)",
        )
        .bind(HARNESS_PROVIDER_ID)
        .execute(database.pool())
        .await
        .expect("provider fixture");

        let repository = Arc::new(SqliteAgentExecutionRepository::new(database.pool().clone()));
        runner.bind_conversation_repo(SqliteConversationRepository::new(database.pool().clone()));
        let participant_id = generate_id();
        let created = repository
            .create_execution_with_participants(
                &owner,
                &CreateAgentExecutionParams {
                    goal: "deterministic scheduler integration".to_owned(),
                    status: AgentExecutionStatus::Planning,
                    plan_gate: nomifun_common::PlanGate::Automatic,
                    adaptation_policy,
                    decision_policy: nomifun_common::DecisionPolicy::Automatic,
                    delegation_policy: DelegationPolicy::Automatic,
                    max_parallel,
                    work_dir: work_dir.map(str::to_owned),
                    lead_conversation_id: None,
                    initial_plan_input: r#"{"mode":"explicit"}"#.to_owned(),
                },
                &[harness_participant(participant_id.clone())],
                &system_event(AgentExecutionEventKind::Created, None, None, json!({})),
            )
            .await
            .expect("execution fixture");
        let step_ids = titles
            .iter()
            .map(|_| generate_id())
            .collect::<Vec<_>>();
        let new_steps = step_ids
            .iter()
            .zip(titles.iter())
            .map(|(step_id, title)| harness_step(step_id.clone(), &participant_id, title))
            .collect::<Vec<_>>();
        let id_by_title = titles
            .iter()
            .zip(step_ids.iter())
            .map(|(title, id)| (*title, id.as_str()))
            .collect::<HashMap<_, _>>();
        let new_dependencies = dependencies
            .iter()
            .map(|(blocker, blocked)| NewAgentExecutionStepDependency {
                blocker_step_id: id_by_title[blocker].to_owned(),
                blocked_step_id: id_by_title[blocked].to_owned(),
            })
            .collect::<Vec<_>>();
        repository
            .reconcile_plan(
                &owner,
                &created.execution_id,
                created.version,
                &ReconcileAgentExecutionPlanParams {
                    goal: None,
                    plan_gate: None,
                    adaptation_policy: None,
                    decision_policy: None,
                    delegation_policy: None,
                    keep_step_ids: Vec::new(),
                    new_participants: Vec::new(),
                    retire_participant_ids: Vec::new(),
                    new_steps,
                    new_dependencies,
                    execution_status: AgentExecutionStatus::Running,
                },
                &system_event(
                    AgentExecutionEventKind::PlanChanged,
                    None,
                    None,
                    json!({"change":"harness_plan"}),
                ),
            )
            .await
            .expect("materialize harness plan");

        let publisher = AgentExecutionEventPublisher::new(Arc::new(NoopUserEventSink));
        let mut deps = ExecutionSchedulerDeps::new(
            repository.clone(),
            runner,
            Arc::new(NoopConversationEffects),
            publisher,
            data_dir.path().to_path_buf(),
        );
        deps.attempt_timeout = Duration::from_secs(5);
        (
            ExecutionScheduler::new(deps),
            repository,
            created.execution_id,
            data_dir,
            owner,
        )
    }

    async fn wait_for_terminal(
        repository: &SqliteAgentExecutionRepository,
        owner_id: &str,
        execution_id: &str,
    ) -> nomifun_db::AgentExecutionDetailRows {
        let deadline = StdInstant::now() + Duration::from_secs(5);
        loop {
            let detail = repository
                .get_execution_detail(owner_id, execution_id)
                .await
                .expect("load harness detail")
                .expect("harness execution exists");
            let status = detail
                .execution
                .status
                .parse::<AgentExecutionStatus>()
                .expect("canonical harness status");
            if status.is_terminal() {
                return detail;
            }
            if StdInstant::now() >= deadline {
                eprintln!(
                    "scheduler harness timeout: status={}, version={}, steps={:?}, attempts={:?}",
                    detail.execution.status,
                    detail.execution.version,
                    detail.steps.iter().map(|step| (&step.title, &step.status)).collect::<Vec<_>>(),
                    detail
                        .attempts
                        .iter()
                        .map(|attempt| (
                            &attempt.attempt.attempt_id,
                            &attempt.attempt.status,
                            &attempt.attempt.error,
                            &attempt.attempt.output_summary,
                        ))
                        .collect::<Vec<_>>(),
                );
                panic!("harness execution should settle");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_runs_ready_roots_in_parallel_then_unblocks_downstream() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let downstream_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) = make_scheduler_harness(
            runner,
            &["upstream-a", "upstream-b", "downstream"],
            &[("upstream-a", "downstream"), ("upstream-b", "downstream")],
            2,
            AdaptationPolicy::Fixed,
            None,
        )
        .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "completed");
        assert!(detail
            .steps
            .iter()
            .all(|step| step.status == ExecutionStepStatus::Completed.to_string()));
        assert_eq!(downstream_guard.call_count("upstream-a"), 1);
        assert_eq!(downstream_guard.call_count("upstream-b"), 1);
        assert_eq!(downstream_guard.call_count("downstream"), 1);
        assert!(
            downstream_guard.max_active() >= 2,
            "two independent ready steps must overlap"
        );
        assert!(
            !downstream_guard.downstream_started_too_early(),
            "downstream work started before both blockers completed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_provisions_relative_workspace_before_dispatch() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let runner_guard = runner.clone();
        let (scheduler, repository, execution_id, data_dir, owner_id) = make_scheduler_harness(
            runner,
            &["workspace-root"],
            &[],
            1,
            AdaptationPolicy::Fixed,
            Some("multi-agent-test"),
        )
        .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "completed");
        let persisted_workspace = detail
            .execution
            .work_dir
            .as_deref()
            .expect("relative workspace must be persisted as an absolute path");
        let canonical_workspace =
            nomifun_common::paths::canonicalize_simplified(Path::new(persisted_workspace))
                .expect("persisted workspace exists");
        assert!(canonical_workspace.is_dir());
        assert_eq!(
            canonical_workspace.file_name().and_then(|name| name.to_str()),
            Some("multi-agent-test")
        );
        assert!(
            canonical_workspace
                .parent()
                .is_some_and(|parent| parent.ends_with(&execution_id))
        );
        let observed = runner_guard
            .workspace_dirs()
            .into_iter()
            .flatten()
            .next()
            .expect("attempt runner receives the prepared workspace");
        assert_eq!(
            nomifun_common::paths::canonicalize_simplified(Path::new(&observed))
                .expect("runner workspace exists"),
            canonical_workspace
        );
        assert_eq!(runner_guard.call_count("workspace-root"), 1);
        assert!(data_dir.path().join("agent-executions").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_fails_invalid_workspace_before_creating_attempt() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let runner_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) = make_scheduler_harness(
            runner,
            &["invalid-workspace"],
            &[],
            1,
            AdaptationPolicy::Adaptive,
            Some("../outside"),
        )
        .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "failed");
        assert_eq!(runner_guard.call_count("invalid-workspace"), 0);
        assert!(
            detail.attempts.is_empty(),
            "deterministic workspace preparation must fail the Execution before Attempt creation"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_does_not_repeat_a_deterministic_provider_failure() {
        let runner = Arc::new(HarnessAttemptRunner::deterministic_failure("failed-root"));
        let runner_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) = make_scheduler_harness(
            runner,
            &["failed-root", "dependent"],
            &[("failed-root", "dependent")],
            2,
            AdaptationPolicy::Adaptive,
            None,
        )
        .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "failed");
        assert_eq!(runner_guard.call_count("failed-root"), 1);
        assert_eq!(runner_guard.call_count("dependent"), 0);
        let failed = detail
            .steps
            .iter()
            .find(|step| step.title == "failed-root")
            .expect("failed root");
        let dependent = detail
            .steps
            .iter()
            .find(|step| step.title == "dependent")
            .expect("dependent");
        assert_eq!(failed.status, ExecutionStepStatus::Failed.to_string());
        assert_eq!(dependent.status, ExecutionStepStatus::Skipped.to_string());
        let attempts = detail
            .attempts
            .iter()
            .filter(|attempt| attempt.attempt.step_id == failed.step_id)
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1, "deterministic provider errors must not retry");
        assert_eq!(
            attempts[0].attempt.error.as_deref(),
            Some("The model returned a tool that was not advertised for this request")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_start_wakes_retry_backoff_without_waiting_for_timer() {
        let runner = Arc::new(HarnessAttemptRunner::retry_once());
        let runner_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) = make_scheduler_harness(
            runner,
            &["retry-root"],
            &[],
            1,
            AdaptationPolicy::Adaptive,
            None,
        )
        .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if runner_guard.call_count("retry-root") == 1 {
                    let detail = repository
                        .get_execution_detail(&owner_id, &execution_id)
                        .await
                        .expect("load retry detail")
                        .expect("retry execution exists");
                    if detail.steps.iter().any(|step| {
                        step.title == "retry-root"
                            && step.status == ExecutionStepStatus::Pending.to_string()
                            && step.dispatch_after.is_some_and(|retry_after| retry_after > now_ms())
                    }) {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("first retry attempt should start");

        // A manual retry clears the persisted backoff before asking the
        // already-running scheduler to wake. This is the real command path:
        // `start()` is a scheduling nudge, while the repository mutation is
        // what makes the step immediately runnable.
        let pending = repository
            .get_execution_detail(&owner_id, &execution_id)
            .await
            .expect("load pending retry detail")
            .expect("retry execution exists");
        let retry_step = pending
            .steps
            .iter()
            .find(|step| step.title == "retry-root")
            .map(|step| (step.step_id.clone(), step.version))
            .expect("retry step exists");
        drop(pending);
        repository
            .reset_steps_for_retry(
                &owner_id,
                &execution_id,
                repository
                    .get_execution(&owner_id, &execution_id)
                    .await
                    .expect("reload retry execution")
                    .expect("retry execution exists")
                    .version,
                &[RetryAgentExecutionStep {
                    step_id: retry_step.0,
                    expected_step_version: retry_step.1,
                }],
                &system_event(
                    AgentExecutionEventKind::StepChanged,
                    None,
                    None,
                    json!({"change":"manual_retry"}),
                ),
            )
            .await
            .expect("clear retry backoff");

        let wake_requested_at = StdInstant::now();
        scheduler.start(owner_id.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if runner_guard.call_count("retry-root") == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("manual start should wake the retry loop");
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "completed");
        assert_eq!(runner_guard.call_count("retry-root"), 2);
        assert!(
            wake_requested_at.elapsed() < Duration::from_secs(1),
            "manual start should wake the retry loop instead of waiting for backoff"
        );
    }
}
