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
use futures::FutureExt;
use futures::stream::{FuturesUnordered, StreamExt};
use nomifun_api_types::{
    AgentErrorCode, AgentExecution, AgentExecutionDetail, ExecutionModelRef,
    ExecutionParticipant, ExecutionStep,
};
use nomifun_common::{
    AdaptationPolicy, AgentExecutionEventKind, AgentExecutionStatus, AgentStepMode,
    AgentToolPolicy, AppError, ExecutionAttemptStatus, ExecutionStepKind, ExecutionStepStatus,
    StepFailurePolicy, apply_agent_role_context, generate_id, now_ms,
};
use nomifun_db::{
    AgentExecutionAttemptRecoveryDisposition, AgentExecutionAttemptSessionKind,
    AgentExecutionLeaseToken,
    AgentExecutionTurnAuthority, AgentExecutionTerminalTurnGuard, AttemptConversationEffectParams,
    ConversationCleanupCursor, CreateAgentExecutionAttemptParams, IAgentExecutionRepository, LoopRepeatResetParams,
    NewAgentExecutionEvent, RetryAgentExecutionStep,
    RecoveredAgentExecutionAttemptOutput,
    SettleAgentExecutionAttemptParams, UpdateAgentExecutionParams,
};
use serde_json::json;
use tokio::sync::{Notify, watch};

use crate::attempt_runner::{
    AttemptDisposition, AttemptOutcome, AttemptRunner, AttemptSessionTarget, RecoveredAttemptOutcome, MISSING_DELIVERY_RECEIPT_CODE,
};
use crate::control_steps::{self, ControlResolution};
use crate::conversation_effect::{AttemptConversationEffects, PendingConversationEffect};
use crate::domain_mapper;
use crate::event_publisher::AgentExecutionEventPublisher;
use crate::engine::is_automation_initial_plan;
use crate::lifecycle::AgentExecutionLifecycle;

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

fn attempt_session_target(
    initial_plan_input: &str,
    automation_control: Option<&nomifun_api_types::AutomationExecutionControl>,
    step_id: &str,
) -> Result<(AttemptSessionTarget, AgentExecutionAttemptSessionKind), AppError> {
    if crate::engine::is_retired_directed_plan(initial_plan_input)? {
        return Err(AppError::Conflict("retired product-directed execution cannot resume".into()));
    }
    if is_automation_initial_plan(initial_plan_input)? {
        let control = automation_control.ok_or_else(|| {
            AppError::Internal("AutoWork AgentExecution has no original control identity".to_owned())
        })?;
        let root_step_id = control.root_step_id.as_deref().ok_or_else(|| {
            AppError::Internal("AutoWork AgentExecution has no materialized original root Step".to_owned())
        })?;
        // The shared read-only projection identifies the immutable original
        // root. An appended Step is a separate Attempt even when its title,
        // role or participant matches that root; it must not borrow the lead
        // Session or create a second automation relation to it.
        if step_id == root_step_id {
            Ok((
                AttemptSessionTarget::AutomationLead {
                    conversation_id: control.lead_conversation_id.clone(),
                },
                AgentExecutionAttemptSessionKind::AutomationLead,
            ))
        } else {
            Ok((AttemptSessionTarget::ChildAttempt, AgentExecutionAttemptSessionKind::ChildAttempt))
        }
    } else if let Some(source) = crate::engine::execution_source_from_initial_plan(initial_plan_input)? {
        Ok((AttemptSessionTarget::SourceChildAttempt { source }, AgentExecutionAttemptSessionKind::ChildAttempt))
    } else {
        Ok((
            AttemptSessionTarget::ChildAttempt,
            AgentExecutionAttemptSessionKind::ChildAttempt,
        ))
    }
}

fn should_project_lead_report(initial_plan_input: &str) -> Result<bool, AppError> {
    Ok(!is_automation_initial_plan(initial_plan_input)?)
}

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

struct CleanupPageProgress {
    empty: bool,
    completed: bool,
    next_cursor: Option<ConversationCleanupCursor>,
}

/// Revalidate the exact inactive link before dispatch. The native cleanup
/// additionally carries the immutable Attempt invocation, so a successor
/// admitted after this check cannot become the cancellation target. The final
/// exact acknowledgement still proves this original link's cleanup authority.
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
                        // Exact acknowledgement retires duplicate links of
                        // this same Attempt only. Continue only to
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

fn cleanup_unit_key(cleanup:&nomifun_db::PendingConversationCleanup)->(String,String,String,String,String) {
    (cleanup.user_id.clone(),cleanup.conversation_id.clone(),cleanup.execution_id.clone(),
        cleanup.step_id.clone(),cleanup.attempt_id.clone())
}

#[derive(Debug, Clone, Copy)]
struct AttemptSettlementFence {
    step_version: i64,
    attempt_version: i64,
}

type ScheduledAgentJobs = FuturesUnordered<BoxFuture<'static, (String, Result<(), AppError>)>>;

#[async_trait]
pub(crate) trait ConversationEffects: Send + Sync {
    async fn cancel_attempt(
        &self,
        cleanup: &nomifun_db::PendingConversationCleanup,
    ) -> Result<(), AppError>;

    async fn steer_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
        text: &str,
    ) -> Result<(), AppError>;

    async fn stop_attempt_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
    ) -> Result<(), AppError>;

    async fn report_lead(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        operation_id: &str,
    ) -> Result<(), AppError>;

    async fn report_lead_input(&self, _owner_id: &str, _detail: &AgentExecutionDetail,
        _request: &nomifun_api_types::AgentExecutionEvent) -> Result<(),AppError> { Ok(()) }
}

pub(crate) struct ExecutionSchedulerDeps {
    pub repository: Arc<dyn IAgentExecutionRepository>,
    pub attempt_runner: Arc<dyn AttemptRunner>,
    pub publisher: AgentExecutionEventPublisher,
    pub conversation_effects: Arc<dyn ConversationEffects>,
    pub data_dir: PathBuf,
    pub attempt_timeout: Duration,
    pub lifecycle: AgentExecutionLifecycle,
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
            lifecycle: AgentExecutionLifecycle::new(),
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
    /// Process-local single-flight for the existing durable effect queue.
    /// Tasks remain owned by the lifecycle when a scheduler generation exits.
    conversation_controls: DashMap<String, ()>,
    conversation_control_changed: Notify,
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
                conversation_controls: DashMap::new(),
                conversation_control_changed: Notify::new(),
            }),
        }
    }

    pub fn start(&self, owner_id: String, execution_id: String) {
        if self.inner.deps.lifecycle.is_cancelled() {
            return;
        }
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
                let lifecycle = scheduler.inner.deps.lifecycle.clone();
                lifecycle.spawn(async move {
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

    pub(crate) fn generation(&self, execution_id: &str) -> Option<String> {
        self.inner.active.get(execution_id).map(|handle|handle.generation.clone())
    }

    /// A committed cancellation cannot stop a replacement scheduler generation
    /// that was admitted while the database receipt was returning to its caller.
    pub(crate) fn stop_generation(&self, execution_id:&str, generation:&str) {
        if let Some(handle)=self.inner.active.get(execution_id)
            && handle.generation==generation {
            let _=handle.cancel.send(true);
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

    /// Drain the durable cleanup outbox encoded by inactive attempt links.
    /// Cancellation and acknowledgement are deliberately separate: a crash
    /// between them repeats an idempotent cancel instead of losing cleanup.
    pub async fn reconcile_conversation_cleanup(&self, execution_id: Option<&str>) {
        if !self.reconcile_conversation_cleanup_once(execution_id).await {
            self.schedule_cleanup_reconciliation();
        }
    }

    /// User Stop is already durable. Its host signal runs independently of
    /// ordinary control delivery, including a stalled Steer on the same task.
    pub(crate) fn request_conversation_cleanup(&self, execution_id: String) {
        let scheduler = self.clone();
        self.inner.deps.lifecycle.spawn(async move {
            scheduler.reconcile_conversation_cleanup(Some(&execution_id)).await;
        });
    }

    pub(crate) async fn reconcile_conversation_cleanup_strict(
        &self,
        execution_id: &str,
    ) -> Result<(), AppError> {
        if self
            .reconcile_conversation_cleanup_once(Some(execution_id))
            .await
        {
            Ok(())
        } else {
            self.schedule_cleanup_reconciliation();
            Err(AppError::Conflict(format!(
                "AgentExecution {execution_id} still has unacknowledged runtime cleanup"
            )))
        }
    }

    async fn reconcile_conversation_cleanup_once(&self, execution_id: Option<&str>) -> bool {
        self.reconcile_conversation_cleanup_page(execution_id, None).await
            .is_some_and(|page| page.completed && page.next_cursor.is_none())
    }

    async fn reconcile_conversation_cleanup_page(
        &self,
        execution_id: Option<&str>,
        cursor: Option<ConversationCleanupCursor>,
    ) -> Option<CleanupPageProgress> {
        let page = match self
            .inner
            .deps
            .repository
            .list_pending_conversation_cleanups(execution_id, cursor, 100)
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(%error, "failed to load pending Agent conversation cleanup");
                return None;
            }
        };
        let empty = page.cleanups.is_empty();
        let validate_repository = self.inner.deps.repository.clone();
        let cancel_repository = self.inner.deps.repository.clone();
        let acknowledge_repository = self.inner.deps.repository.clone();
        let conversation_effects = self.inner.deps.conversation_effects.clone();
        let completed = reconcile_cleanup_batch(
            page.cleanups,
            cleanup_unit_key,
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
                                    .cancel_attempt(&cleanup),
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
        Some(CleanupPageProgress { empty, completed, next_cursor: page.next_cursor })
    }

    fn schedule_cleanup_reconciliation(&self) {
        if self.inner.deps.lifecycle.is_cancelled() {
            return;
        }
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
        self.inner.deps.lifecycle.spawn(async move {
            let mut cursor = None;
            let mut round_failed = false;
            let mut retry_delay = EFFECT_RETRY_MIN;
            let mut delay = EFFECT_RETRY_MIN;
            loop {
                tokio::select! {
                    _ = scheduler.inner.deps.lifecycle.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let Some(page) = scheduler.reconcile_conversation_cleanup_page(None, cursor).await else {
                    // A failed read must not skip the page it could not load.
                    delay = retry_delay;
                    retry_delay = next_effect_retry_delay(retry_delay);
                    continue;
                };
                if page.empty && cursor.is_none() {
                    break;
                }
                round_failed |= !page.completed;
                cursor = page.next_cursor;
                if cursor.is_some() {
                    // One bounded page per turn, including past failed rows.
                    delay = EFFECT_RETRY_MIN;
                } else {
                    // The snapshot high watermark makes this a finite sweep.
                    // Start another sweep to retry failures and include new
                    // arrivals, even when every observed item succeeded.
                    delay = if round_failed { retry_delay } else { EFFECT_RETRY_MIN };
                    retry_delay = if round_failed {
                        next_effect_retry_delay(retry_delay)
                    } else {
                        EFFECT_RETRY_MIN
                    };
                    round_failed = false;
                }
            }
            scheduler.inner.cleanup_reconciliation_running.remove(KEY);
        });
    }

    /// Reopen commands must deliver the previous terminal epoch before mutation.
    /// Errors retain the durable operation for the existing background retry.
    pub async fn reconcile_lead_report(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
    ) -> Result<(), AppError> {
        let result = self.reconcile_lead_report_once(owner_id, detail).await;
        if result.is_err() {
            self.schedule_lead_report_reconciliation(
                owner_id.to_owned(),
                detail.execution.execution_id.clone(),
            );
        }
        result
    }

    /// One post-commit path for every terminal transition. It publishes the
    /// terminal outbox and reconciles the exact lead report immediately. Native
    /// Attempt cleanup remains durable, independent work and does not postpone
    /// an already verified result while an inactive Session is being stopped.
    pub async fn after_terminal_commit(&self, owner_id: &str, execution_id: &str) {
        self.publish().await;
        if let Ok(detail) = self.detail(owner_id, execution_id).await {
            if detail.execution.status.is_terminal() {
                if let Err(error) = self.reconcile_lead_report(owner_id, &detail).await {
                    tracing::warn!(%execution_id, %error, "failed to reconcile terminal lead report");
                }
            }
        }
        self.reconcile_conversation_cleanup(Some(execution_id)).await;
    }

    async fn reconcile_lead_report_once(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
    ) -> Result<(), AppError> {
        if detail.execution.status==AgentExecutionStatus::WaitingInput {
            return self.reconcile_lead_input_notice_once(owner_id,detail).await;
        }
        if !detail.execution.status.is_terminal()
            || detail.execution.lead_conversation_id.is_none()
        {
            return Ok(());
        }
        let execution_row = self
            .inner
            .deps
            .repository
            .get_execution(owner_id, &detail.execution.execution_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Agent Execution {}",
                    detail.execution.execution_id
                ))
            })?;
        // An AutoWork Attempt already ran in the lead AgentSession. Projecting
        // the aggregate summary back into that same Session would duplicate the
        // assistant result that the user just watched in the main conversation.
        let suppress_projection =
            !should_project_lead_report(&execution_row.initial_plan_input)?;
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
            return Ok(());
        };
        if delivered_operation_ids.contains(&operation_id) {
            return Ok(());
        }
        if !suppress_projection {
            self.inner
                .deps
                .conversation_effects
                .report_lead(owner_id, detail, &operation_id)
                .await?;
        }
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
                        "projection_suppressed": suppress_projection,
                    }),
                ),
            )
            .await?;
        self.publish().await;
        Ok(())
    }

    async fn reconcile_lead_input_notice_once(&self,owner_id:&str,detail:&AgentExecutionDetail)->Result<(),AppError> {
        let row=self.inner.deps.repository.get_execution(owner_id,&detail.execution.execution_id).await?
            .ok_or_else(||AppError::NotFound("input notice Execution is missing".into()))?;
        if crate::engine::execution_source_from_initial_plan(&row.initial_plan_input)?.is_none() { return Ok(()); }
        let mut requests=Vec::new();let mut reconciled=HashSet::new();let mut after=0;
        loop {
            let events=self.inner.deps.repository.list_events(owner_id,&detail.execution.execution_id,after,500).await?;
            for event in &events {
                let payload:serde_json::Value=serde_json::from_str(&event.payload)
                    .map_err(|error|AppError::Internal(format!("invalid input notice event: {error}")))?;
                if event.event_type==AgentExecutionEventKind::DecisionRequested.as_str() {
                    requests.push(domain_mapper::event(event.clone())?);
                }
                if payload.get("change").and_then(serde_json::Value::as_str)==Some("lead_input_notice_reconciled")
                    && let Some(sequence)=payload.get("request_event_sequence").and_then(serde_json::Value::as_i64) { reconciled.insert(sequence); }
            }
            let Some(last)=events.last() else { break; };after=last.sequence;if events.len()<500 {break;}
        }
        for attempt in detail.attempts.iter().filter(|attempt|attempt.status==ExecutionAttemptStatus::WaitingInput) {
            let Some(request)=requests.iter().rev().find(|request|request.step_id.as_deref()==Some(attempt.step_id.as_str())
                &&request.attempt_id.as_deref()==Some(attempt.attempt_id.as_str())
                &&request.payload.get("question").and_then(serde_json::Value::as_str)==attempt.question.as_deref()) else {continue;};
            if reconciled.contains(&request.sequence) {continue;}
            self.inner.deps.conversation_effects.report_lead_input(owner_id,detail,request).await?;
            let current=self.detail(owner_id,&detail.execution.execution_id).await?;
            self.inner.deps.repository.append_event(owner_id,&detail.execution.execution_id,current.execution.version,
                &system_event(AgentExecutionEventKind::StatusChanged,Some(&attempt.step_id),Some(&attempt.attempt_id),
                    json!({"change":"lead_input_notice_reconciled","request_event_sequence":request.sequence}))).await?;
            self.publish().await;
        }
        Ok(())
    }

    fn schedule_lead_report_reconciliation(&self, owner_id: String, execution_id: String) {
        if self.inner.deps.lifecycle.is_cancelled() {
            return;
        }
        if self
            .inner
            .pending_lead_reports
            .insert(execution_id.clone(), ())
            .is_some()
        {
            return;
        }
        let scheduler = self.clone();
        self.inner.deps.lifecycle.spawn(async move {
            let mut delay = EFFECT_RETRY_MIN;
            loop {
                tokio::select! {
                    _ = scheduler.inner.deps.lifecycle.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let completed = match scheduler.detail(&owner_id, &execution_id).await {
                    Ok(detail) => match scheduler
                        .reconcile_lead_report_once(&owner_id, &detail)
                        .await
                    {
                        Ok(()) => true,
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

    pub(crate) async fn read_adoptable_attempt_output(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<Option<AttemptOutcome>, AppError> {
        self.inner
            .deps
            .attempt_runner
            .read_adoptable_output(owner_id, conversation_id)
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
        let row = repository.get_execution(owner_id, execution_id).await?
            .ok_or_else(|| AppError::NotFound(format!("Agent Execution {execution_id}")))?;
        if crate::engine::is_retired_directed_plan(&row.initial_plan_input)? {
            tracing::warn!(%execution_id, "retired product-directed execution cannot run");
            return Ok(());
        }
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
            let mut running_jobs = ScheduledAgentJobs::new();
            let mut in_flight_step_ids = HashSet::new();
            // Decision answers and steers are write-ahead effects in attempt
            // runtime_state.  Recover them before classifying a running
            // attempt as process-interrupted.
            loop {
                if self.inner.deps.lifecycle.is_cancelled() || *cancelled.borrow() {
                    return Ok(SchedulerLoopExit::Normal);
                }
                if *lease_loss.borrow() {
                    return Ok(SchedulerLoopExit::LeaseLost);
                }
                // Session control can itself stall while the native owner is
                // stopping. Keep its write-ahead intent pending, and allow
                // this scheduler generation to stop and release its lease.
                let delivered = tokio::select! {
                    biased;
                    _ = cancelled.changed() => return Ok(SchedulerLoopExit::Normal),
                    _ = self.inner.deps.lifecycle.cancelled() => return Ok(SchedulerLoopExit::Normal),
                    _ = lease_loss.changed() => return Ok(SchedulerLoopExit::LeaseLost),
                    result = self.process_one_pending_conversation_effect(
                        owner_id, execution_id, &lease, &mut running_jobs, &mut in_flight_step_ids,
                    ) => result?,
                };
                if !delivered {
                    break;
                }
            }
            self.recover_interrupted(owner_id, execution_id, &lease, &mut running_jobs, &mut in_flight_step_ids).await?;
            let mut deferred_error: Option<AppError> = None;
            loop {
                if self.inner.deps.lifecycle.is_cancelled() || *cancelled.borrow() {
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
                        _ = self.inner.deps.lifecycle.cancelled() => {
                            return Ok(SchedulerLoopExit::Normal);
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
                    AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput | AgentExecutionStatus::Paused => {}
                    AgentExecutionStatus::Planning
                    | AgentExecutionStatus::Completed
                    | AgentExecutionStatus::CompletedWithFailures
                    | AgentExecutionStatus::Failed
                    | AgentExecutionStatus::Cancelled => return Ok(SchedulerLoopExit::Normal),
                }
                let delivered = tokio::select! {
                    biased;
                    _ = cancelled.changed() => return Ok(SchedulerLoopExit::Normal),
                    _ = self.inner.deps.lifecycle.cancelled() => return Ok(SchedulerLoopExit::Normal),
                    _ = lease_loss.changed() => return Ok(SchedulerLoopExit::LeaseLost),
                    result = self.process_one_pending_conversation_effect(
                        owner_id, execution_id, &lease, &mut running_jobs, &mut in_flight_step_ids,
                    ) => result?,
                };
                if delivered {
                    continue;
                }
                self.recover_interrupted(owner_id, execution_id, &lease, &mut running_jobs, &mut in_flight_step_ids).await?;
                detail = self.detail(owner_id, execution_id).await?;
                if detail.execution.status == AgentExecutionStatus::Paused {
                    if running_jobs.is_empty() { return Ok(SchedulerLoopExit::Normal); }
                    // This is a dispatch pause. Already admitted work keeps
                    // its original future/owner and settles under this lease.
                    tokio::select! {
                        outcome = running_jobs.next() => {
                            if let Some((step_id, outcome)) = outcome {
                                in_flight_step_ids.remove(&step_id);
                                if let Err(error) = outcome { deferred_error = Some(error); }
                            }
                        }
                        _ = cancelled.changed() => {},
                        _ = self.inner.deps.lifecycle.cancelled() => {},
                        _ = lease_loss.changed() => {},
                        _ = wake.notified() => {},
                    }
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
                let ready_capacity_wait = ready.iter().any(|step| step.kind == ExecutionStepKind::Agent);
                let reservations=repository.attempt_reservations(execution_id).await?;
                let agent_steps = select_agent_steps_with_reservations(&detail, ready, &in_flight_step_ids, &reservations);
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
                    }.boxed());
                }
                if !running_jobs.is_empty() {
                    let queued_input = detail.attempts.iter().any(|attempt| attempt.status == ExecutionAttemptStatus::WaitingInput
                        && attempt.runtime_state.as_ref().is_some_and(|state| state.get("pending_conversation_effects").and_then(serde_json::Value::as_array)
                            .is_some_and(|effects| effects.iter().any(|effect| effect.get("kind").and_then(serde_json::Value::as_str) == Some("decision_input")))));
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
                        _ = self.inner.deps.lifecycle.cancelled() => {
                            return Ok(SchedulerLoopExit::Normal);
                        }
                        changed = lease_loss.changed() => {
                            if changed.is_err() || *lease_loss.borrow() {
                                return Ok(SchedulerLoopExit::LeaseLost);
                            }
                        }
                        _ = wake.notified() => {}
                        // A native owner can become safely parked without
                        // completing its receipt. Recheck only domain/owner
                        // reservations so lost wakeups cannot strand ready work.
                        _ = tokio::time::sleep(EFFECT_RETRY_MIN), if queued_input || ready_capacity_wait => {}
                    }
                    continue;
                }

                if self.finalize_if_settled(owner_id, &detail, &lease).await? {
                    return Ok(SchedulerLoopExit::Normal);
                }
                if detail.attempts.iter().any(|attempt| self.inner.conversation_controls.contains_key(&attempt.attempt_id)) {
                    // Native control remains under its original tracked owner.
                    // Other jobs and lease/cancel notifications keep advancing.
                    tokio::select! {
                        _ = wake.notified() => {},
                        _ = cancelled.changed() => {},
                        _ = self.inner.deps.lifecycle.cancelled() => {},
                        _ = lease_loss.changed() => {},
                    }
                    continue;
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
                        _ = self.inner.deps.lifecycle.cancelled() => {
                            return Ok(SchedulerLoopExit::Normal);
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
                    if let Err(error)=self.reconcile_lead_report(owner_id,&detail).await {
                        tracing::warn!(%execution_id,%error,"execution input notice remains in its existing outbox");
                    }
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
            if self.inner.deps.lifecycle.is_cancelled() || *cancelled.borrow() {
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
                AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput | AgentExecutionStatus::Paused
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
        let lifecycle = self.inner.deps.lifecycle.clone();
        lifecycle.clone().spawn(async move {
            loop {
                tokio::select! {
                    _ = lifecycle.cancelled() => return,
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

    pub async fn shutdown(&self) -> Result<(), AppError> {
        for handle in self.inner.active.iter() {
            let _ = handle.cancel.send(true);
        }
        self.inner.deps.lifecycle.shutdown().await
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
        running_jobs: &mut ScheduledAgentJobs,
        in_flight_step_ids: &mut HashSet<String>,
    ) -> Result<bool, AppError> {
        let detail = self.detail(owner_id, execution_id).await?;
        let mut candidate = None;
        for attempt in &detail.attempts {
            if self.inner.conversation_controls.contains_key(&attempt.attempt_id) {
                continue;
            }
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
            if detail.execution.status == AgentExecutionStatus::Paused && effects.pending_conversation_effects.first()
                .is_some_and(|effect| matches!(effect, PendingConversationEffect::DecisionInput { .. })) {
                continue;
            }
            if effects.pending_conversation_effects.first().is_some_and(|effect| {
                matches!(effect, PendingConversationEffect::DecisionInput { .. })
            }) && in_flight_step_ids.contains(&step.step_id) {
                // Let an earlier invocation of this Step finish unwinding
                // before reserving its continuation. Stop/steer effects may
                // still be delivered while the original job is in flight.
                continue;
            }
            if attempt.status == ExecutionAttemptStatus::WaitingInput && matches!(effects.pending_conversation_effects.first(),
                Some(PendingConversationEffect::DecisionInput { .. })) {
                if self.inner.deps.repository.admit_waiting_attempt_input(owner_id, execution_id, &step.step_id, &attempt.attempt_id, lease,
                    &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), Some(&attempt.attempt_id), json!({"change":"decision_input_admitted"})),
                ).await?.is_some() { self.publish().await; return Ok(true); }
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
            PendingConversationEffect::DecisionInput {
                operation_id,
                content,
            } => {
                // A decision resumes the existing model turn.  Keep the
                // write-ahead state intact until attempt settlement; transport
                // failure is retried under the same stable operation identity.
                let scheduler = self.clone();
                let owner_id = owner_id.to_owned();
                let execution_id = execution_id.to_owned();
                let conversation_id = conversation_id.to_owned();
                let lease = lease.clone();
                let step_id = step.step_id.clone();
                in_flight_step_ids.insert(step_id.clone());
                running_jobs.push(async move {
                    let outcome = scheduler.inner.deps.attempt_runner
                        .continue_with_input(
                            &owner_id,
                            &conversation_id,
                            &operation_id,
                            AgentExecutionTurnAuthority {
                                execution_id: execution_id.clone(),
                                step_id: step.step_id.clone(),
                                attempt_id: attempt.attempt_id.clone(),
                                expected_step_version: step.version,
                                expected_attempt_version: attempt.version,
                                lease_owner: lease.owner().to_owned(),
                            },
                            &content,
                            scheduler.inner.deps.attempt_timeout,
                        )
                        .await
                        .map_err(|error| {
                            AppError::BadGateway(format!(
                                "durable decision delivery {operation_id} failed: {error}"
                            ))
                        });
                    let result = match outcome {
                        Ok(outcome) => scheduler.settle_agent_outcome(
                            &owner_id,
                            &execution_id,
                            &step.step_id,
                            &attempt.attempt_id,
                            Ok(outcome),
                            attempt.attempt_no,
                            AttemptSettlementFence {
                                step_version: step.version,
                                attempt_version: attempt.version,
                            },
                            Some(&lease),
                        ).await,
                        Err(error) => {
                            tracing::warn!(%execution_id, %step_id, %error, "durable decision remains pending; unrelated Attempts continue");
                            tokio::time::sleep(EFFECT_RETRY_MIN).await;
                            Ok(())
                        },
                    };
                    (step_id, result)
                }.boxed());
                return Ok(true);
            }
            PendingConversationEffect::StopTurn { .. } | PendingConversationEffect::Steer { .. } => {
                self.start_conversation_control(owner_id, execution_id, &attempt.attempt_id);
                Ok(true)
            }
        }
    }

    fn start_conversation_control(&self, owner_id: &str, execution_id: &str, attempt_id: &str) {
        use dashmap::mapref::entry::Entry;
        match self.inner.conversation_controls.entry(attempt_id.to_owned()) {
            Entry::Occupied(_) => return,
            Entry::Vacant(entry) => { entry.insert(()); }
        }
        let scheduler = self.clone();
        let owner_id = owner_id.to_owned();
        let execution_id = execution_id.to_owned();
        let attempt_id = attempt_id.to_owned();
        self.inner.deps.lifecycle.spawn(async move {
            let mut delivered = None;
            let mut delay = EFFECT_RETRY_MIN;
            loop {
                if scheduler.inner.deps.lifecycle.is_cancelled() { break; }
                match scheduler.deliver_conversation_control(&owner_id, &execution_id, &attempt_id, &mut delivered).await {
                    Ok(true) => { delay = EFFECT_RETRY_MIN; }
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(%execution_id, %attempt_id, %error, "durable control remains pending; unrelated Attempts continue");
                        tokio::select! {
                            _ = scheduler.inner.deps.lifecycle.cancelled() => break,
                            _ = tokio::time::sleep(delay) => {},
                        }
                        delay = next_effect_retry_delay(delay);
                    }
                }
            }
            scheduler.inner.conversation_controls.remove(&attempt_id);
            scheduler.inner.conversation_control_changed.notify_waiters();
            if let Some(handle) = scheduler.inner.active.get(&execution_id) {
                handle.wake.notify_one();
            }
        });
    }

    /// Return true after one exact queue head is acknowledged. A successful
    /// native call is retained locally across CAS/read errors so appending a
    /// later command cannot cause another delivery or lose that later intent.
    async fn deliver_conversation_control(
        &self,
        owner_id: &str,
        execution_id: &str,
        attempt_id: &str,
        delivered: &mut Option<PendingConversationEffect>,
    ) -> Result<bool, AppError> {
        let detail = self.detail(owner_id, execution_id).await?;
        if !matches!(detail.execution.status, AgentExecutionStatus::Running | AgentExecutionStatus::WaitingInput | AgentExecutionStatus::Paused) {
            return Ok(false);
        }
        let Some(attempt) = detail.attempts.iter().find(|attempt| attempt.attempt_id == attempt_id
            && matches!(attempt.status, ExecutionAttemptStatus::Running | ExecutionAttemptStatus::WaitingInput)) else {
            return Ok(false);
        };
        let Some(step) = detail.steps.iter().find(|step| step.step_id == attempt.step_id && step.superseded_in_revision.is_none()) else {
            return Ok(false);
        };
        let mut effects = attempt.runtime_state.clone().map(serde_json::from_value::<AttemptConversationEffects>)
            .transpose().map_err(|error| AppError::Internal(format!("invalid durable conversation controls: {error}")))?
            .unwrap_or_default();
        if effects.review_blocked.is_some() { return Ok(false); }
        let Some(effect) = effects.pending_conversation_effects.first().cloned() else { return Ok(false); };
        let (operation_id, effect_name) = match &effect {
            PendingConversationEffect::StopTurn { operation_id, .. } => (operation_id, "stop_turn"),
            PendingConversationEffect::Steer { operation_id, .. } => (operation_id, "steer"),
            PendingConversationEffect::DecisionInput { .. } => return Ok(false),
        };
        let conversation_id = attempt.conversation_id.as_deref()
            .ok_or_else(|| AppError::Conflict("durable control has no exact Session link".into()))?;
        if delivered.as_ref() != Some(&effect) {
            // No select/timeout drops this future. Even after a user stop or
            // scheduler replacement, this original owner keeps its real result.
            match &effect {
                PendingConversationEffect::StopTurn { operation_id, target_operation_id } => {
                    self.inner.deps.conversation_effects.stop_attempt_turn(owner_id, conversation_id, operation_id, target_operation_id).await?;
                }
                PendingConversationEffect::Steer { operation_id, target_operation_id, content } => {
                    self.inner.deps.conversation_effects.steer_attempt(owner_id, conversation_id, operation_id, target_operation_id, content).await?;
                }
                PendingConversationEffect::DecisionInput { .. } => unreachable!(),
            }
            *delivered = Some(effect.clone());
            // The native wait can overlap enqueue/cancel. Reread before the
            // acknowledgement instead of replacing a stale copy of the queue.
            return Ok(true);
        }
        effects.pending_conversation_effects.remove(0);
        let runtime_state = if effects == AttemptConversationEffects::default() { None } else { Some(effects.encode()?) };
        self.inner.deps.repository.acknowledge_attempt_conversation_effect(
            owner_id, execution_id, &step.step_id, attempt_id, attempt.version,
            &AttemptConversationEffectParams { runtime_state },
            &system_event(AgentExecutionEventKind::StepChanged, Some(&step.step_id), Some(attempt_id),
                json!({"change":"conversation_effect_delivered","effect":effect_name,"operation_id":operation_id})),
        ).await?;
        *delivered = None;
        self.inner.conversation_control_changed.notify_waiters();
        self.publish().await;
        Ok(true)
    }

    async fn recover_interrupted(
        &self,
        owner_id: &str,
        execution_id: &str,
        lease: &AgentExecutionLeaseToken,
        running_jobs: &mut ScheduledAgentJobs,
        in_flight_step_ids: &mut HashSet<String>,
    ) -> Result<(), AppError> {
        loop {
            let detail = self.detail(owner_id, execution_id).await?;
            let Some(attempt) = detail.attempts.iter().find(|attempt| {
                matches!(attempt.status, ExecutionAttemptStatus::Queued | ExecutionAttemptStatus::Running)
                    && !in_flight_step_ids.contains(&attempt.step_id)
                    && !self.inner.conversation_controls.contains_key(&attempt.attempt_id)
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
            let recovered_output = if was_queued {
                None
            } else {
                self.recover_attempt_output(owner_id, attempt).await?
            };
            let recovered_turn_key = attempt.runtime_state.clone()
                .map(serde_json::from_value::<AttemptConversationEffects>).transpose()
                .map_err(|error| AppError::Internal(format!("invalid recovered Attempt effects: {error}")))?
                .unwrap_or_default().current_turn_operation_key(&attempt.attempt_id)?;
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
                    recovered_output.as_ref(),
                    &system_event(
                        AgentExecutionEventKind::AttemptChanged,
                        Some(&step.step_id),
                        Some(&attempt.attempt_id),
                        json!({
                            "reason": if was_queued { "queued_before_restart" } else { "process_restart" },
                            "reconciliation":"initial_turn_receipt",
                            "turn_operation_key": recovered_turn_key,
                        }),
                    ),
                )
                .await?;
            match recovered.disposition {
                AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved => {
                    let conversation_id = attempt.conversation_id.clone().ok_or_else(||
                        AppError::Conflict("ongoing Attempt has no exact Session".into()))?;
                    let scheduler = self.clone();
                    let owner_id = owner_id.to_owned();
                    let execution_id = execution_id.to_owned();
                    let step_id = step.step_id.clone();
                    let attempt_id = attempt.attempt_id.clone();
                    let attempt_no = attempt.attempt_no;
                    let fence = AttemptSettlementFence { step_version: step.version, attempt_version: attempt.version };
                    let lease = lease.clone();
                    in_flight_step_ids.insert(step_id.clone());
                    running_jobs.push(async move {
                        let observed = scheduler.inner.deps.attempt_runner.observe_existing_turn(
                            &owner_id, &conversation_id, &recovered_turn_key,
                        ).await;
                        let result = match observed {
                            Ok(outcome) => scheduler.settle_agent_outcome(&owner_id, &execution_id,
                                &step_id, &attempt_id, Ok(outcome), attempt_no, fence, Some(&lease)).await,
                            // Observation failure is infrastructure/reconciliation
                            // state. Never turn it into retryable business output.
                            Err(error) => Err(error),
                        };
                        (step_id, result)
                    }.boxed());
                    continue;
                }
                AgentExecutionAttemptRecoveryDisposition::QueuedRescheduled => {}
                AgentExecutionAttemptRecoveryDisposition::CompletedReceiptAdopted => {
                    tracing::info!(
                        %execution_id,
                        step_id = %step.step_id,
                        attempt_id = %attempt.attempt_id,
                        "adopted completed initial-turn receipt during recovery"
                    );
                }
                AgentExecutionAttemptRecoveryDisposition::CancelledReceiptAdopted => {},
                AgentExecutionAttemptRecoveryDisposition::UserInputRequested => {}
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

    pub(crate) async fn recover_attempt_output(
        &self,
        owner_id: &str,
        attempt: &nomifun_api_types::ExecutionAttempt,
    ) -> Result<Option<RecoveredAgentExecutionAttemptOutput>, AppError> {
        let effects = attempt.runtime_state.clone()
            .map(serde_json::from_value::<AttemptConversationEffects>)
            .transpose().map_err(|error| AppError::Internal(format!(
                "attempt {} has malformed durable conversation effects: {error}", attempt.attempt_id
            )))?;
        if effects.as_ref().is_some_and(|effects| effects.pending_conversation_effects.iter()
            .any(|effect| matches!(effect, PendingConversationEffect::Steer { .. }))) {
            return Ok(None);
        }
        let operation_key = effects.unwrap_or_default().current_turn_operation_key(&attempt.attempt_id)?;
        let Some(conversation_id) = attempt.conversation_id.as_deref() else {
            return Ok(None);
        };
        let Some(recovered) = self.inner.deps.attempt_runner.recover_outcome(
            owner_id, conversation_id, &operation_key,
        ).await? else {
            return Ok(None);
        };
        Ok(Some(recovered_delivery_output(&attempt.attempt_id, recovered)))
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
        let execution_row = self
            .inner
            .deps
            .repository
            .get_execution(owner_id, execution_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Agent Execution {execution_id}")))?;
        let (session_target, session_kind) = attempt_session_target(
            &execution_row.initial_plan_input,
            detail.automation_control.as_ref(),
            &current_step.step_id,
        )?;
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
            "session_kind": session_kind.relation(),
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
                        session_kind,
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
                session_target,
                &participant,
                &model_pool,
                detail.execution.work_dir.as_deref(),
                &step.title,
                step.tool_policy,
                step.profile
                    .as_ref()
                    .is_some_and(|profile| profile.managed_process_only),
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
        let outcome = match outcome {
            // Once a Session link was admitted, a plain transport/query error
            // is not a canonical terminal failure. Preserve the invocation for
            // exact receipt observation; only structured terminal outcomes can
            // authorize business retry or retirement.
            Err(error) if conversation_id.is_some() => return Err(error),
            outcome => outcome,
        };
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
        let detail = loop {
            let changed = self.inner.conversation_control_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let detail = self.detail(owner_id, execution_id).await?;
            let current_step = detail.steps.iter().find(|step| step.step_id == step_id);
            let current_attempt = detail.attempts.iter().find(|attempt| attempt.attempt_id == attempt_id);
            let pending_steer = if let Some(attempt) = current_attempt {
                attempt.runtime_state.clone().map(serde_json::from_value::<AttemptConversationEffects>)
                    .transpose().map_err(|error| AppError::Internal(format!("invalid durable controls before settlement: {error}")))?
                    .is_some_and(|effects| effects.pending_conversation_effects.iter()
                        .any(|effect| matches!(effect, PendingConversationEffect::Steer { .. })))
            } else { false };
            if !detail.execution.status.is_terminal()
                && current_step.is_some_and(|step| step.version == settlement_fence.step_version)
                && current_attempt.is_some_and(|attempt| attempt.status == ExecutionAttemptStatus::Running)
                && pending_steer
            {
                // Keep this exact already-computed outcome while its original
                // correction is delivered. Sibling jobs remain independently
                // polled and user Stop goes directly to its canonical owner.
                self.start_conversation_control(owner_id, execution_id, attempt_id);
                tokio::select! {
                    _ = &mut changed => {},
                    _ = tokio::time::sleep(EFFECT_RETRY_MIN) => {},
                    _ = self.inner.deps.lifecycle.cancelled() => return Ok(()),
                }
                continue;
            }
            break detail;
        };
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
        // Step version and immutable Attempt identity are the invocation
        // generation. Question/answer/pause/retry/replacement change that
        // Step generation. Attempt version also CASes outbox metadata: a
        // successful steer and its acknowledgement advance it without
        // replacing this Turn, so they must not discard its valid callback.
        if step.version != settlement_fence.step_version
            || attempt.version < settlement_fence.attempt_version
            || attempt.step_id != step_id
        {
            return Ok(());
        }
        let expected_attempt_version = attempt.version;

        if let Ok(delivery) = &outcome && delivery.disposition == AttemptDisposition::Cancelled {
            let (canonical_operation_id, terminal_event_id) = delivery.canonical_terminal.as_ref()
                .ok_or_else(|| AppError::Conflict("cancelled Attempt has no exact canonical terminal".into()))?;
            self.inner.deps.repository.settle_attempt(
                owner_id, execution_id, step_id, settlement_fence.step_version, attempt_id, expected_attempt_version, lease,
                &SettleAgentExecutionAttemptParams {
                    expected_active_session_turn: None,
                    expected_terminal_turn: Some(AgentExecutionTerminalTurnGuard::Cancelled {
                        conversation_id: delivery.conversation_id.clone(), canonical_operation_id: canonical_operation_id.clone(),
                        terminal_event_id: terminal_event_id.clone(),
                    }),
                    attempt_status: ExecutionAttemptStatus::Cancelled, step_status: ExecutionStepStatus::Cancelled,
                    execution_status: None, question: Some(None), error: Some(None),
                    output_summary: Some(delivery.text.clone()), output_files: Some(serde_json::to_string(&delivery.output_files)
                        .map_err(|error| AppError::Internal(error.to_string()))?),
                    tokens: Some(delivery.tokens), retry_after: Some(None), runtime_state: Some(None),
                    started_at: None, finished_at: Some(Some(now_ms())), loop_repeat_reset: None,
                },
                &system_event(AgentExecutionEventKind::AttemptChanged, Some(step_id), Some(attempt_id),
                    json!({"attempt_status":"cancelled","step_status":"cancelled",
                        "canonical_operation_id":canonical_operation_id,"terminal_event_id":terminal_event_id})),
            ).await?;
            self.publish().await;
            self.reconcile_conversation_cleanup(Some(execution_id)).await;
            return Ok(());
        }

        if let Ok(delivery) = &outcome && delivery.disposition == AttemptDisposition::InputRequired {
            let request = delivery.user_input_request.as_ref().ok_or_else(|| AppError::Conflict("canonical human handoff has no typed source question".into()))?;
            request.validate().map_err(AppError::Conflict)?;
                let (canonical_operation_id, terminal_event_id) = delivery.canonical_terminal.as_ref()
                    .ok_or_else(|| AppError::Conflict("user input handoff has no exact canonical terminal".into()))?;
                self.inner.deps.repository.settle_attempt(
                    owner_id, execution_id, step_id, settlement_fence.step_version,
                    attempt_id, expected_attempt_version, lease,
                    &SettleAgentExecutionAttemptParams {
                        expected_active_session_turn: None,
                        expected_terminal_turn: Some(AgentExecutionTerminalTurnGuard::InputRequired {
                            conversation_id: delivery.conversation_id.clone(),
                            canonical_operation_id: canonical_operation_id.clone(),
                            terminal_event_id: terminal_event_id.clone(),
                            user_input_request: request.clone(),
                        }),
                        attempt_status: ExecutionAttemptStatus::WaitingInput,
                        step_status: ExecutionStepStatus::WaitingInput,
                        execution_status: Some(AgentExecutionStatus::WaitingInput),
                        question: Some(Some(request.question.clone())), error: Some(None),
                        output_summary: Some(delivery.text.clone()),
                        output_files: Some(serde_json::to_string(&delivery.output_files)
                            .map_err(|error| AppError::Internal(error.to_string()))?),
                        tokens: Some(delivery.tokens), retry_after: Some(None),
                        runtime_state: Some(None), started_at: None, finished_at: None,
                        loop_repeat_reset: None,
                    },
                    &system_event(AgentExecutionEventKind::DecisionRequested,
                        Some(step_id), Some(attempt_id), json!({
                            "canonical_operation_id":canonical_operation_id,
                            "terminal_event_id":terminal_event_id,"question":request.question,"user_input_request":request,
                        })),
                ).await?;
                self.publish().await;
                return Ok(());
        }

        let (attempt_status, step_status, error, output, output_files, tokens, retry_after) = match outcome {
            Ok(outcome) if agent_outcome_can_complete(&outcome) => (
                ExecutionAttemptStatus::Completed,
                ExecutionStepStatus::Completed,
                None,
                outcome.text,
                outcome.output_files,
                outcome.tokens,
                None,
            ),
            Ok(outcome) => {
                let (retryable, has_marker, mut reason) = if outcome.disposition == AttemptDisposition::Delivered {
                    // A closed canonical Turn with no public answer or
                    // verified output is an invalid delivery, not a timeout.
                    // Replaying its settled effects cannot repair that fact.
                    (false, true, "Agent turn completed without a public answer or verified output".to_owned())
                } else {
                    let retryable = outcome.error_retryable.unwrap_or(false);
                    let has_marker = outcome.error.is_some()
                        || outcome.error_code.is_some();
                    let reason = if let Some(error) = outcome.error.clone() {
                        error
                    } else if let Some(code) = outcome.error_code.clone() {
                        format!("Agent attempt failed ({code})")
                    } else if has_marker {
                        "Agent attempt failed".to_owned()
                    } else {
                        "Agent turn failed without a structured terminal reason".to_owned()
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
                reason = durable_attempt_failure_reason(&outcome, reason);
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
                let retry_limit = match attempt_outcome_retry_class(&outcome, has_marker, retryable) {
                    AttemptRetryClass::Timeout => MAX_TIMEOUT_RETRIES,
                    _ => MAX_PROVIDER_RETRIES,
                };
                let can_retry = detail.execution.adaptation_policy == AdaptationPolicy::Adaptive
                    && retryable && attempt_no <= retry_limit;
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
        let settled_turn_key = attempt.runtime_state.clone()
            .map(serde_json::from_value::<AttemptConversationEffects>).transpose()
            .map_err(|error| AppError::Internal(format!("invalid settled Attempt effects: {error}")))?
            .unwrap_or_default().current_turn_operation_key(attempt_id)?;
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
                expected_attempt_version,
                lease,
                &SettleAgentExecutionAttemptParams {
                    expected_terminal_turn: None,
                    expected_active_session_turn: None,
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
                    json!({"attempt_status":attempt_status,"step_status":step_status,
                        "turn_operation_key":settled_turn_key}),
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
                    })
            }) {
                return Ok(());
            }
        }
        settled?;
        self.publish().await;
        // The Attempt's canonical output is already settled. Native cleanup
        // remains in its durable inactive-link outbox, while this job returns
        // so the aggregate can finalize and deliver that verified result.
        if !self.inner.deps.lifecycle.is_cancelled() {
            let scheduler = self.clone();
            let cleanup_execution_id = execution_id.to_owned();
            self.inner.deps.lifecycle.spawn(async move {
                scheduler.reconcile_conversation_cleanup(Some(&cleanup_execution_id)).await;
            });
        }
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
                    expected_terminal_turn: None,
                    expected_active_session_turn: None,
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
        AppError::Conflict(_) | AppError::NotFound(_) | AppError::ExecutionAdmissionPending => true,
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
        // Missing evidence is not a proven timeout and never authorizes
        // replay of an already-admitted Turn's tools or model effects.
        return AttemptRetryClass::Deterministic;
    }
    if !retryable {
        return AttemptRetryClass::Deterministic;
    }
    match outcome.error_code.as_deref() {
        Some("USER_LLM_PROVIDER_RATE_LIMITED") => AttemptRetryClass::RateLimited,
        Some("USER_LLM_PROVIDER_TIMEOUT") => AttemptRetryClass::Timeout,
        Some(code) if is_transient_agent_error_code_name(code) => AttemptRetryClass::Provider,
        _ => AttemptRetryClass::Deterministic,
    }
}

fn durable_attempt_failure_reason(outcome: &AttemptOutcome, reason: String) -> String {
    if outcome.error_code.as_deref() == Some(MISSING_DELIVERY_RECEIPT_CODE) {
        // Preserve this uncertainty as a durable, machine-readable terminal
        // marker. The automation receipt mapper consumes it after a restart
        // and queue policy parks the exact claim instead of treating it as a
        // retryable or ordinary failure.
        format!("{MISSING_DELIVERY_RECEIPT_CODE}: {reason}")
    } else {
        reason
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

pub(crate) fn agent_outcome_can_complete(outcome: &AttemptOutcome) -> bool {
    if outcome.disposition != AttemptDisposition::Delivered || outcome.user_input_request.is_some() {
        return false;
    }
    let has_text = outcome
        .text
        .as_ref()
        .is_some_and(|text| !text.trim().is_empty());
    // The canonical Runtime owns accepted requirements and delivery review.
    // A prose spec is task data, never a second inferred completion contract.
    has_text || !outcome.output_files.is_empty()
}

fn recovered_delivery_output(attempt_id: &str, recovered: RecoveredAttemptOutcome) -> RecoveredAgentExecutionAttemptOutput {
    let outcome = recovered.outcome;
    let disposition = match outcome.disposition {
        AttemptDisposition::Delivered if !agent_outcome_can_complete(&outcome) => AttemptDisposition::Failed,
        disposition => disposition,
    };
    let error = (disposition == AttemptDisposition::Failed).then(|| durable_attempt_failure_reason(&outcome,
        outcome.error.clone().unwrap_or_else(|| "Canonical Agent Turn ended without a successful public delivery".into())));
    RecoveredAgentExecutionAttemptOutput {
        attempt_id:attempt_id.into(), conversation_id:outcome.conversation_id,
        canonical_operation_id:recovered.canonical_operation_id,terminal_event_id:recovered.terminal_event_id,
        user_input_request:outcome.user_input_request,disposition,text:outcome.text,output_files:outcome.output_files,error,tokens:outcome.tokens,
    }
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

fn select_agent_steps_with_reservations(
    detail: &AgentExecutionDetail, ready: Vec<&ExecutionStep>, in_flight_step_ids: &HashSet<String>,
    reservations:&[nomifun_db::AgentExecutionReservation],
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
    let mut workspace_reservation_count = 0usize;
    let mut workspace_mutator_active = false;
    let mut active_step_ids: HashSet<&str> = HashSet::new();
    for attempt in detail.attempts.iter().filter(|attempt| {
        matches!(
            attempt.status,
            ExecutionAttemptStatus::Queued | ExecutionAttemptStatus::Running
        ) || attempt.status==ExecutionAttemptStatus::WaitingInput
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
        // The same read projection used by the admission writer distinguishes
        // model yield from retained workspace responsibility. Missing facts
        // remain held; a WaitingInput display alone grants no release.
        let reservation=reservations.iter().find(|reservation|reservation.attempt_id==attempt.attempt_id);
        let workspace_held=reservation.is_none_or(|reservation|reservation.workspace_held);
        if workspace_held {workspace_reservation_count += 1;}
        let model_held=reservation.is_none_or(|reservation|reservation.capacity_held);
        if model_held { active_count += 1; }
        workspace_mutator_active |= workspace_held && current_steps
            .get(attempt.step_id.as_str())
            .is_some_and(|step| step.tool_policy != AgentToolPolicy::ReadOnly);
        if model_held { *selected_per_participant.entry(participant_id).or_default() += 1; }
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
        workspace_reservation_count += 1;
        workspace_mutator_active |= current_steps
            .get(step_id.as_str())
            .is_some_and(|step| step.tool_policy != AgentToolPolicy::ReadOnly);
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
    // All Steps in one AgentExecution share one canonical workspace. Full
    // tools can write directly and ReadShell is explicitly not an OS
    // read-only sandbox, so overlapping either class with a sibling can race
    // patches, commands, tests, Git state, or source observations. Until a
    // real per-Step worktree/merge owner exists, only ReadOnly siblings may
    // overlap. This preserves useful parallel exploration while making coding
    // execution deterministic instead of merely hoping writers touch
    // different files.
    if workspace_mutator_active {
        return selected;
    }
    for step in ready
        .into_iter()
        .filter(|step| step.kind == ExecutionStepKind::Agent)
        .filter(|step| !in_flight_step_ids.contains(&step.step_id))
    {
        let workspace_mutator = step.tool_policy != AgentToolPolicy::ReadOnly;
        if workspace_reservation_count > 0 && workspace_mutator {
            continue;
        }
        if workspace_mutator && !selected.is_empty() {
            continue;
        }
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
        if workspace_mutator || selected.len() == available {
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
            let upstream = detail
                .steps
                .iter()
                .find(|candidate| candidate.step_id == blocker);
            let title = upstream
                .map(|candidate| candidate.title.as_str())
                .unwrap_or("unknown step");
            let step_status = upstream
                .map(|candidate| candidate.status.to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            let latest = detail
                .attempts
                .iter()
                .filter(|attempt| attempt.step_id == blocker)
                .max_by_key(|attempt| attempt.attempt_no);
            let output = latest
                .and_then(|attempt| attempt.output_summary.as_deref())
                .unwrap_or("(no output summary)");
            brief.push_str(&format!(
                "- step_id={blocker}; title={title}; status={step_status}\n  Latest output summary: {output}\n"
            ));
            if let Some(attempt) = latest {
                brief.push_str(&format!(
                    "  Latest Attempt: attempt_no={}; status={}\n",
                    attempt.attempt_no, attempt.status
                ));
            }
            if let Some(reason) = latest.and_then(|attempt| {
                attempt
                    .error
                    .as_deref()
                    .or(attempt.question.as_deref())
                    .map(str::trim)
                    .filter(|reason| !reason.is_empty())
            }) {
                brief.push_str(&format!("  Waiting/blocked/error reason: {reason}\n"));
            }
            let verified_files = detail
                .attempts
                .iter()
                .filter(|attempt| {
                    attempt.step_id == blocker && attempt.status.is_terminal()
                })
                .max_by_key(|attempt| attempt.attempt_no)
                .map(|attempt| attempt.output_files.as_slice())
                .unwrap_or_default();
            if !verified_files.is_empty() {
                brief.push_str(&format!(
                    "  Verified output files from the exact settled Attempt (historical delivery references; re-read under current permissions): {}\n",
                    serde_json::to_string(verified_files).unwrap_or_else(|_| "[]".to_owned())
                ));
            }
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
        SqliteAgentExecutionRepository, SqlitePool,
    };
    use nomifun_realtime::UserEventSink;
    use tempfile::{TempDir, tempdir};
    use tokio::sync::Barrier;

    #[test]
    fn autowork_attempt_targets_only_the_original_root_at_the_bound_main_session() {
        let lead = "0190f5fe-7c00-7a00-8000-000000000071";
        let root = "0190f5fe-7c00-7a00-8000-000000000073";
        let child = "0190f5fe-7c00-7a00-8000-000000000074";
        let automation = serde_json::json!({
            "mode": "automation",
            "source": {
                "requirement_id": "0190f5fe-7c00-7a00-8000-000000000072",
                "claim_generation": 1,
                "operation_id": "autowork:test"
            },
            "plan": { "steps": [] }
        })
        .to_string();
        let mut control = nomifun_api_types::AutomationExecutionControl {
            root_step_id: Some(root.to_owned()),
            lead_conversation_id: lead.to_owned(),
            requirement_id: "0190f5fe-7c00-7a00-8000-000000000072".into(),
            claim_generation: 1,
        };
        assert_eq!(
            attempt_session_target(&automation, Some(&control), root).unwrap(),
            (
                AttemptSessionTarget::AutomationLead {
                    conversation_id: lead.to_owned()
                },
                AgentExecutionAttemptSessionKind::AutomationLead,
            )
        );
        assert_eq!(
            attempt_session_target(&automation, Some(&control), child).unwrap(),
            (AttemptSessionTarget::ChildAttempt, AgentExecutionAttemptSessionKind::ChildAttempt),
        );
        assert!(!should_project_lead_report(&automation).unwrap());
        assert!(attempt_session_target(&automation, None, root).is_err());
        control.root_step_id = None;
        assert!(attempt_session_target(&automation, Some(&control), root).is_err(),
            "planning-only identity cannot authorize an actual root or child dispatch");
        assert_eq!(
            attempt_session_target(r#"{"mode":"explicit","plan":{"steps":[]}}"#, None, child)
                .unwrap(),
            (
                AttemptSessionTarget::ChildAttempt,
                AgentExecutionAttemptSessionKind::ChildAttempt,
            )
        );
        assert!(
            should_project_lead_report(r#"{"mode":"explicit","plan":{"steps":[]}}"#)
                .unwrap()
        );
        let source = crate::AgentExecutionSource {
            agent_session_id: lead.into(), turn_operation_id:"source-turn".into(),
            tool_operation_id:"source-tool".into(), effect_id:"source-effect".into(),
        };
        for input in [
            serde_json::json!({"mode":"automatic","source":source}),
            serde_json::json!({"mode":"explicit","source":source,"plan":{"steps":[]}}),
        ] {
            let encoded = input.to_string();
            assert_eq!(attempt_session_target(&encoded, None, child).unwrap(),
                (AttemptSessionTarget::SourceChildAttempt { source: source.clone() }, AgentExecutionAttemptSessionKind::ChildAttempt));
            assert!(should_project_lead_report(&encoded).unwrap());
        }
        let retired = serde_json::json!({"mode":"directed","source":source,"plan":{"steps":[]}}).to_string();
        assert!(attempt_session_target(&retired, None, child).is_err());
    }

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
    async fn cleanup_batch_never_acknowledges_another_attempt_in_the_same_session() {
        let first=nomifun_db::PendingConversationCleanup {link_id:1,user_id:"owner".into(),conversation_id:"session".into(),
            execution_id:"execution".into(),step_id:"step-a".into(),attempt_id:"attempt-a".into(),
            target:nomifun_db::ConversationCleanupTurnTarget::Exact {operation_id:"original-a".into()}};
        let mut duplicate=first.clone();duplicate.link_id=2;
        let mut other=first.clone();other.link_id=3;other.step_id="step-b".into();other.attempt_id="attempt-b".into();
        other.target=nomifun_db::ConversationCleanupTurnTarget::Exact {operation_id:"original-b".into()};
        let cancelled=Arc::new(Mutex::new(Vec::new()));let acknowledged=Arc::new(Mutex::new(Vec::new()));
        let completed=reconcile_cleanup_batch(vec![first,duplicate,other],cleanup_unit_key,
            |_|Box::pin(async {CleanupValidation::Current}),
            {let cancelled=cancelled.clone();move |cleanup| {let cancelled=cancelled.clone();let attempt=cleanup.attempt_id.clone();
                Box::pin(async move {cancelled.lock().unwrap().push(attempt.clone());attempt=="attempt-a"})}},
            {let acknowledged=acknowledged.clone();move |cleanup| {let acknowledged=acknowledged.clone();let attempt=cleanup.attempt_id.clone();
                Box::pin(async move {acknowledged.lock().unwrap().push(attempt);true})}},
        ).await;
        assert!(!completed);let mut calls=cancelled.lock().unwrap().clone();calls.sort();
        assert_eq!(calls,["attempt-a","attempt-b"]);assert_eq!(*acknowledged.lock().unwrap(),["attempt-a"]);
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
    fn downstream_brief_carries_only_latest_settled_verified_output_files() {
        let execution_id = generate_id();
        let participant_id = generate_id();
        let upstream_id = generate_id();
        let downstream_id = generate_id();
        let upstream_new = harness_step(upstream_id.clone(), &participant_id, "Produce asset");
        let mut downstream_new = harness_step(
            downstream_id.clone(),
            &participant_id,
            "Synthesize delivery",
        );
        downstream_new.agent_mode = Some(AgentStepMode::Synthesis);
        let materialize_step = |step: NewAgentExecutionStep, status| ExecutionStep {
            step_id: step.step_id,
            execution_id: execution_id.clone(),
            title: step.title,
            spec: step.spec,
            profile: None,
            kind: step.kind,
            agent_mode: step.agent_mode,
            status,
            tool_policy: step.tool_policy,
            role: step.role,
            fanout_group: step.fanout_group,
            control_policy: None,
            failure_policy: step.failure_policy,
            assigned_participant_id: step.assigned_participant_id,
            assignment_source: step.assignment_source,
            assignment_score: step.assignment_score,
            assignment_rationale: step.assignment_rationale,
            assignment_locked: step.assignment_locked,
            preset_prompt: step.preset_prompt,
            graph_x: step.graph_x,
            graph_y: step.graph_y,
            dispatch_after: None,
            introduced_in_revision: 1,
            superseded_in_revision: None,
            version: 1,
            created_at: 1,
            updated_at: 1,
        };
        let upstream = materialize_step(upstream_new, ExecutionStepStatus::Completed);
        let downstream = materialize_step(downstream_new, ExecutionStepStatus::Pending);
        let attempt = |attempt_no: i64, summary: &str, files: Vec<&str>| {
            nomifun_api_types::ExecutionAttempt {
                attempt_id: generate_id(),
                execution_id: execution_id.clone(),
                step_id: upstream_id.clone(),
                attempt_no,
                participant_id: Some(participant_id.clone()),
                conversation_id: Some(generate_id()),
                status: ExecutionAttemptStatus::Completed,
                trigger_reason: "test".to_owned(),
                effective_config: json!({}),
                question: None,
                error: None,
                output_summary: Some(summary.to_owned()),
                output_files: files.into_iter().map(str::to_owned).collect(),
                tokens: None,
                retry_after: None,
                runtime_state: None,
                started_at: Some(1),
                finished_at: Some(2),
                version: 1,
                created_at: 1,
                updated_at: 2,
            }
        };
        let detail = AgentExecutionDetail {
            automation_control:None,
            execution: AgentExecution {
                execution_id: execution_id.clone(),
                goal: "Deliver a verified file".to_owned(),
                lead_conversation_id: None,
                work_dir: Some("/workspace".to_owned()),
                delegation_policy: DelegationPolicy::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                max_parallel: 1,
                status: AgentExecutionStatus::Running,
                summary: None,
                version: 1,
                plan_revision: 1,
                event_sequence: 1,
                created_at: 1,
                updated_at: 1,
            },
            participants: Vec::new(),
            steps: vec![upstream, downstream.clone()],
            dependencies: vec![nomifun_api_types::ExecutionStepDependency {
                execution_id: execution_id.clone(),
                blocker_step_id: upstream_id.clone(),
                blocked_step_id: downstream_id,
                introduced_in_revision: 1,
                superseded_in_revision: None,
            }],
            attempts: vec![
                attempt(1, "old", vec!["/workspace/stale.txt"]),
                attempt(2, "Done; prose mentions /workspace/fake.txt", vec!["/workspace/verified.txt"]),
            ],
        };
        let brief = compose_brief(&detail, &downstream);
        assert!(brief.contains("step_id="));
        assert!(brief.contains("status=completed"));
        assert!(brief.contains("/workspace/verified.txt"));
        assert!(!brief.contains("/workspace/stale.txt"));
        assert!(brief.contains("prose mentions /workspace/fake.txt"));
        assert_eq!(brief.matches("Verified output files").count(), 1);
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
    fn only_explicit_transient_provider_outcomes_are_retryable() {
        let transient = |error_code: &str| AttemptOutcome {
            user_input_request: None, canonical_terminal: None,
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000202".to_owned(),
            text: None,
            output_files: Vec::new(),
            disposition: AttemptDisposition::Failed,
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
            AttemptRetryClass::Deterministic
        );
        let mut unclassified = transient("UNKNOWN_OWNER_ERROR");
        unclassified.error = Some("A network timeout was mentioned in the tool output".to_owned());
        assert_eq!(attempt_outcome_retry_class(&unclassified, true, true), AttemptRetryClass::Deterministic,
            "error prose cannot authorize a replay without a structured transient owner code");
    }

    #[test]
    fn missing_canonical_receipt_is_persisted_as_outcome_unknown() {
        let outcome = AttemptOutcome {
            user_input_request: None, canonical_terminal: None,
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000203".to_owned(),
            text: None,
            output_files: Vec::new(),
            disposition: AttemptDisposition::Failed,
            tokens: None,
            error: Some("canonical receipt was not observed".to_owned()),
            error_code: Some(MISSING_DELIVERY_RECEIPT_CODE.to_owned()),
            error_retryable: Some(false),
        };
        assert_eq!(
            durable_attempt_failure_reason(&outcome, "canonical receipt was not observed".into()),
            "agent_delivery_receipt_missing: canonical receipt was not observed"
        );
        assert_eq!(
            attempt_outcome_retry_class(&outcome, true, false),
            AttemptRetryClass::Deterministic,
            "outcome-unknown must never enter an automatic retry lane"
        );
    }

    #[test]
    fn canonical_failure_and_empty_delivery_cannot_complete() {
        let mut outcome = AttemptOutcome {
            user_input_request: None, canonical_terminal: None,
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".to_owned(),
            text: Some("authoritative public answer".to_owned()),
            output_files: Vec::new(),
            disposition: AttemptDisposition::Failed,
            tokens: None,
            error: None,
            error_code: None,
            error_retryable: None,
        };
        assert!(!agent_outcome_can_complete(&outcome));
        outcome.disposition = AttemptDisposition::Delivered;
        assert!(agent_outcome_can_complete(&outcome));
        outcome.text = Some("  \n".to_owned());
        assert!(!agent_outcome_can_complete(&outcome));
        outcome.text = None;
        assert!(!agent_outcome_can_complete(&outcome));
        outcome.output_files = vec!["/workspace/index.html".to_owned()];
        assert!(agent_outcome_can_complete(&outcome));
        outcome.disposition = AttemptDisposition::Failed;
        assert!(!agent_outcome_can_complete(&outcome));
    }

    #[test]
    fn normal_input_handoff_cannot_complete_or_be_adopted_as_success() {
        let mut outcome = AttemptOutcome {
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".into(),
            user_input_request: Some(nomifun_agent_contracts::AgentUserInputRequest {
                request_id:"ask-name".into(), question:"姓名？".into(), remaining_work:"生成 PPT。".into(),
            }),
            canonical_terminal: Some(("operation".into(),"terminal".into())),
            text: Some("请补充姓名后继续。".into()), output_files: vec!["/workspace/draft.pptx".into()],
            disposition:AttemptDisposition::InputRequired, tokens:None, error:None, error_code:None, error_retryable:Some(false),
        };
        assert!(!agent_outcome_can_complete(&outcome));
        let recovered = recovered_delivery_output("attempt",RecoveredAttemptOutcome {
            outcome:outcome.clone(),canonical_operation_id:"operation".into(),terminal_event_id:"terminal".into(),
        });
        assert_eq!(recovered.disposition, AttemptDisposition::InputRequired, "recovery preserves the human handoff independently of automatic decisions");
        assert!(recovered.error.is_none());
        assert_eq!(recovered.user_input_request,outcome.user_input_request);
        outcome.user_input_request = None;
        outcome.disposition = AttemptDisposition::Delivered;
        assert!(agent_outcome_can_complete(&outcome), "ordinary completed receipts retain their current semantics");
    }

    #[test]
    fn canonical_delivery_is_not_reinterpreted_from_task_prose() {
        // The reported game task referred to snake-game-design.md and
        // snake-game-ui.md as inputs while delivering index.html. Settlement
        // must not manufacture a competing Markdown-output requirement.
        let outcome = AttemptOutcome {
            user_input_request: None, canonical_terminal: None,
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000201".to_owned(),
            text: Some("已完成 index.html，参考设计文档 snake-game-design.md 和 snake-game-ui.md".to_owned()),
            output_files: vec!["/workspace/index.html".to_owned()],
            disposition: AttemptDisposition::Delivered,
            tokens: None,
            error: None,
            error_code: None,
            error_retryable: Some(false),
        };
        assert!(agent_outcome_can_complete(&outcome));
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

    #[derive(Default)]
    struct NoopConversationEffects {
        fail_report_once: AtomicBool,
        report_operations: Mutex<Vec<String>>,
        fail_stop_once: AtomicBool,
        block_stop: AtomicBool,
        stop_started: AtomicBool,
        stop_release: Notify,
        block_steer: AtomicBool,
        steer_started: AtomicBool,
        steer_release: Notify,
        block_cleanup: AtomicBool,
        cleanup_started: AtomicBool,
        cleanup_release: Notify,
        cancelled_conversations: Mutex<Vec<String>>,
        cancelled_targets: Mutex<Vec<(String,String)>>,
        failed_cleanup_conversations: Mutex<HashSet<String>>,
        conversation_operations: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ConversationEffects for NoopConversationEffects {
        async fn cancel_attempt(
            &self,
            cleanup: &nomifun_db::PendingConversationCleanup,
        ) -> Result<(), AppError> {
            let conversation_id = cleanup.conversation_id.as_str();
            self.cancelled_conversations.lock().unwrap().push(conversation_id.to_owned());
            if let nomifun_db::ConversationCleanupTurnTarget::Exact {operation_id}=&cleanup.target {
                self.cancelled_targets.lock().unwrap().push((cleanup.attempt_id.clone(),operation_id.clone()));
            }
            self.cleanup_started.store(true, Ordering::SeqCst);
            if self.failed_cleanup_conversations.lock().unwrap().contains(conversation_id) {
                return Err(AppError::Conflict("fixture cleanup owner still busy".into()));
            }
            if self.block_cleanup.load(Ordering::SeqCst) {
                self.cleanup_release.notified().await;
            }
            Ok(())
        }

        async fn steer_attempt(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            operation_id: &str,
            _target_operation_id: &str,
            _text: &str,
        ) -> Result<(), AppError> {
            self.conversation_operations.lock().unwrap().push(operation_id.to_owned());
            self.steer_started.store(true, Ordering::SeqCst);
            if self.block_steer.load(Ordering::SeqCst) {
                self.steer_release.notified().await;
            }
            Ok(())
        }

        async fn stop_attempt_turn(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            operation_id: &str,
            _target_operation_id: &str,
        ) -> Result<(), AppError> {
            self.conversation_operations.lock().unwrap().push(operation_id.to_owned());
            self.stop_started.store(true, Ordering::SeqCst);
            if self.block_stop.load(Ordering::SeqCst) {
                self.stop_release.notified().await;
            }
            if self.fail_stop_once.swap(false, Ordering::SeqCst) {
                return Err(AppError::Timeout("fixture stop failed".into()));
            }
            Ok(())
        }

        async fn report_lead(
            &self,
            _owner_id: &str,
            _detail: &AgentExecutionDetail,
            operation_id: &str,
        ) -> Result<(), AppError> {
            self.report_operations.lock().unwrap().push(operation_id.to_owned());
            if self.fail_report_once.swap(false, Ordering::SeqCst) {
                return Err(AppError::Internal("fixture projection temporarily unavailable".into()));
            }
            Ok(())
        }
    }

    #[derive(Clone)]
    enum HarnessRunnerMode {
        ParallelRoots {
            barrier: Arc<Barrier>,
            downstream_started_too_early: Arc<AtomicBool>,
        },
        OverlapProbe,
        DeterministicFailure {
            failed_step: String,
        },
        RetryOnce {
            remaining_failures: Arc<AtomicUsize>,
        },
        DecisionContinuation {
            started: Arc<AtomicBool>,
            calls: Arc<AtomicUsize>,
            release: Arc<Notify>,
        },
        SteeredTurn { started: Arc<AtomicBool>, release: Arc<Notify> },
        UserInputHandoff,
        CanonicalCancellation,
        ReceiptQueryFailure,
    }

    #[derive(Clone)]
    struct HarnessAttemptRunner {
        mode: HarnessRunnerMode,
        calls: Arc<Mutex<Vec<String>>>,
        briefs: Arc<Mutex<Vec<(String, String)>>>,
        workspace_dirs: Arc<Mutex<Vec<Option<String>>>>,
        completed_successes: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        pool: Arc<Mutex<Option<SqlitePool>>>,
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
                briefs: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                pool: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn deterministic_failure(failed_step: &str) -> Self {
            Self {
                mode: HarnessRunnerMode::DeterministicFailure {
                    failed_step: failed_step.to_owned(),
                },
                calls: Arc::new(Mutex::new(Vec::new())),
                briefs: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                pool: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn overlap_probe() -> Self {
            Self {
                mode: HarnessRunnerMode::OverlapProbe,
                calls: Arc::new(Mutex::new(Vec::new())),
                briefs: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                pool: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn retry_once() -> Self {
            Self {
                mode: HarnessRunnerMode::RetryOnce {
                    remaining_failures: Arc::new(AtomicUsize::new(1)),
                },
                calls: Arc::new(Mutex::new(Vec::new())),
                briefs: Arc::new(Mutex::new(Vec::new())),
                workspace_dirs: Arc::new(Mutex::new(Vec::new())),
                completed_successes: Arc::new(AtomicUsize::new(0)),
                active: Arc::new(AtomicUsize::new(0)),
                max_active: Arc::new(AtomicUsize::new(0)),
                pool: Arc::new(Mutex::new(None)),
                owner_id: Arc::new(Mutex::new(None)),
            }
        }

        fn decision_continuation() -> Self {
            let mut runner = Self::overlap_probe();
            runner.mode = HarnessRunnerMode::DecisionContinuation {
                started: Arc::new(AtomicBool::new(false)),
                calls: Arc::new(AtomicUsize::new(0)),
                release: Arc::new(Notify::new()),
            };
            runner
        }

        fn steered_turn() -> Self {
            let mut runner = Self::overlap_probe();
            runner.mode = HarnessRunnerMode::SteeredTurn {
                started: Arc::new(AtomicBool::new(false)),
                release: Arc::new(Notify::new()),
            };
            runner
        }

        fn bind_pool(&self, pool: SqlitePool) {
            *self
                .pool
                .lock()
                .expect("harness database pool is not poisoned") = Some(pool);
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

        fn brief_for(&self, title: &str) -> Option<String> {
            self.briefs
                .lock()
                .expect("harness brief log is not poisoned")
                .iter()
                .find(|(step, _)| step == title)
                .map(|(_, brief)| brief.clone())
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
            session_target: AttemptSessionTarget,
            _participant: &ExecutionParticipant,
            _execution_model_pool: &[ExecutionModelRef],
            workspace_dir: Option<&str>,
            step_title: &str,
            _tool_policy: AgentToolPolicy,
            _managed_process_only: bool,
            _delegation_policy: DelegationPolicy,
            _delegation_depth: i64,
            _decision_policy: DecisionPolicy,
            attempt_creation_key: &str,
            brief: &str,
            _step_spec: &str,
            _timeout: Duration,
            on_started: crate::attempt_runner::AttemptStarted,
        ) -> Result<AttemptOutcome, AppError> {
            self.workspace_dirs
                .lock()
                .expect("harness workspace log is not poisoned")
                .push(workspace_dir.map(str::to_owned));
            self.briefs
                .lock()
                .expect("harness brief log is not poisoned")
                .push((step_title.to_owned(), brief.to_owned()));
            let creates_child = !matches!(&session_target, AttemptSessionTarget::AutomationLead { .. });
            let conversation_id = match session_target {
                AttemptSessionTarget::ChildAttempt | AttemptSessionTarget::SourceChildAttempt { .. } => {
                    nomifun_common::ConversationId::new().into_string()
                }
                AttemptSessionTarget::AutomationLead { conversation_id } => conversation_id,
            };
            let owner_id = self
                .owner_id
                .lock()
                .expect("harness owner is not poisoned")
                .clone()
                .ok_or_else(|| AppError::Internal("scheduler harness owner is missing".into()))?;
            let pool = self
                .pool
                .lock()
                .expect("harness database pool is not poisoned")
                .clone()
                .ok_or_else(|| {
                    AppError::Internal("scheduler harness database pool is missing".into())
                })?;
            if creates_child {
                sqlx::query(
                "INSERT INTO agent_sessions (\
                    agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, next_seq, created_at\
                 ) VALUES (?, ?, 'live', ?, 0, 0, '{}', 1, ?)",
            )
            .bind(&conversation_id)
            .bind(serde_json::json!({
                "principal_kind": "user",
                "principal_id": owner_id,
            }).to_string())
            .bind(format!("Scheduler harness · {step_title}"))
            .bind(now_ms())
            .execute(&pool)
            .await
            .map_err(|error| AppError::Internal(format!("create harness AgentSession: {error}")))?;
            } else {
                let existing: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE agent_session_id=? AND state='live' \
                     AND json_extract(owner_ref_json,'$.principal_kind')='user' \
                     AND json_extract(owner_ref_json,'$.principal_id')=?)",
                ).bind(&conversation_id).bind(&owner_id).fetch_one(&pool).await
                    .map_err(|error| AppError::Internal(format!("get harness AutoWork lead: {error}")))?;
                if !existing {
                    return Err(AppError::NotFound("original harness AutoWork lead Session".into()));
                }
            }
            on_started(conversation_id.clone()).await?;

            self.calls
                .lock()
                .expect("harness call log is not poisoned")
                .push(step_title.to_owned());
            if matches!(&self.mode, HarnessRunnerMode::ReceiptQueryFailure) {
                return Err(AppError::Timeout("the original receipt query failed after admission".into()));
            }
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
                HarnessRunnerMode::ParallelRoots { .. } | HarnessRunnerMode::OverlapProbe
                    | HarnessRunnerMode::DecisionContinuation { .. } | HarnessRunnerMode::SteeredTurn { .. }
                    | HarnessRunnerMode::UserInputHandoff | HarnessRunnerMode::CanonicalCancellation
                    | HarnessRunnerMode::ReceiptQueryFailure => false,
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
                    user_input_request: None, canonical_terminal: None,
                    conversation_id: format!("harness-{step_title}"),
                    text: None,
                    output_files: Vec::new(),
                    disposition: AttemptDisposition::Failed,
                    tokens: None,
                    error: Some(error.to_owned()),
                    error_code: Some(error_code.to_owned()),
                    error_retryable: Some(retryable),
                });
            }

            if matches!(&self.mode, HarnessRunnerMode::UserInputHandoff | HarnessRunnerMode::CanonicalCancellation) {
                let cancelled = matches!(&self.mode, HarnessRunnerMode::CanonicalCancellation);
                let request = (!cancelled).then(|| nomifun_agent_contracts::AgentUserInputRequest {
                    request_id:"ask-name".into(), question:"请提供姓名。".into(), remaining_work:"生成 PPT。".into(),
                });
                let operation = format!("turn:user:{owner_id}:{conversation_id}:{attempt_creation_key}:initial-turn");
                let started = generate_id();
                let terminal = generate_id();
                let terminal_kind = if cancelled {"turn/cancelled"} else {"turn/completed"};
                let payload = if cancelled {json!({"operation_id":operation})} else {json!({"user_input_request":request})};
                for (seq, id, kind, value) in [(1,&started,"turn/started",json!({})), (2,&terminal,terminal_kind,payload)] {
                    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'terminal-fixture',?,?,1,?,?)")
                        .bind(&conversation_id).bind(seq).bind(id).bind(id).bind(kind).bind(&operation).bind(value.to_string())
                        .execute(&pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                }
                sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,?,?,?,1,1,2)")
                    .bind(&conversation_id).bind(&operation).bind(&operation).bind(&operation).bind(if cancelled {"cancelled"} else {"completed"}).bind(started).bind(&terminal)
                    .execute(&pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_set_generation,last_seq,unread_count) VALUES(?,'ready',0,2,0)")
                    .bind(&conversation_id).execute(&pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                finish(self);
                return Ok(AttemptOutcome {
                    conversation_id, user_input_request:request, canonical_terminal:Some((operation,terminal)),
                    text:(!cancelled).then(||"请提供姓名后继续。".into()),output_files:Vec::new(),
                    disposition:if cancelled {AttemptDisposition::Cancelled} else {AttemptDisposition::InputRequired}, tokens:Some(1),
                    error:None,error_code:None,error_retryable:Some(false),
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
            if matches!(&self.mode, HarnessRunnerMode::OverlapProbe) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if let HarnessRunnerMode::SteeredTurn { started, release } = &self.mode {
                started.store(true, Ordering::SeqCst);
                release.notified().await;
            }

            self.completed_successes.fetch_add(1, Ordering::SeqCst);
            finish(self);
            Ok(AttemptOutcome {
                user_input_request: None, canonical_terminal: None,
                conversation_id: format!("harness-{step_title}"),
                text: Some(format!("completed {step_title}")),
                output_files: Vec::new(),
                disposition: AttemptDisposition::Delivered,
                tokens: Some(1),
                error: None,
                error_code: None,
                error_retryable: None,
            })
        }

        async fn observe_existing_turn(
            &self, owner_id: &str, conversation_id: &str, operation_id: &str,
        ) -> Result<AttemptOutcome, AppError> {
            let HarnessRunnerMode::SteeredTurn { started, release } = &self.mode else {
                return Err(AppError::Conflict("not an observation fixture".into()));
            };
            self.calls.lock().unwrap().push(format!("observe:{operation_id}"));
            started.store(true, Ordering::SeqCst);
            release.notified().await;
            let pool = self.pool.lock().unwrap().clone().unwrap();
            let operation = format!("turn:user:{owner_id}:{conversation_id}:{operation_id}");
            let terminal = generate_id();
            sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,2,?,'observer-fixture',?,'turn/completed',1,?,'{}')")
                .bind(conversation_id).bind(&terminal).bind(&terminal).bind(&operation).execute(&pool).await.unwrap();
            sqlx::query("UPDATE agent_turns SET state='completed',terminal_event_id=?,finished_at=2 WHERE session_id=? AND operation_id=?")
                .bind(&terminal).bind(conversation_id).bind(&operation).execute(&pool).await.unwrap();
            sqlx::query("UPDATE agent_session_heads SET status='ready',active_turn_id=NULL,last_seq=2 WHERE session_id=?")
                .bind(conversation_id).execute(&pool).await.unwrap();
            Ok(AttemptOutcome {
                conversation_id: conversation_id.into(), user_input_request: None,
                canonical_terminal: Some((operation, terminal)), text: Some("original recovered result".into()),
                output_files: Vec::new(), disposition: AttemptDisposition::Delivered,
                tokens: Some(3), error: None, error_code: None, error_retryable: None,
            })
        }

        async fn continue_with_input(
            &self,
            _owner_id: &str,
            conversation_id: &str,
            operation_id: &str,
            _authority: AgentExecutionTurnAuthority,
            _input: &str,
            _timeout: Duration,
        ) -> Result<AttemptOutcome, AppError> {
            if matches!(&self.mode,HarnessRunnerMode::UserInputHandoff) {
                let answer_no = {
                    let mut calls=self.calls.lock().unwrap();
                    calls.push(format!("answer:{operation_id}"));
                    calls.iter().filter(|call|call.starts_with("answer:")).count()
                };
                let request=(answer_no==1).then(||nomifun_agent_contracts::AgentUserInputRequest {
                    request_id:"ask-class".into(),question:"请提供班级。".into(),remaining_work:"完成介绍。".into(),
                });
                let pool=self.pool.lock().unwrap().clone().unwrap();
                let owner=self.owner_id.lock().unwrap().clone().unwrap();
                let operation=format!("turn:user:{owner}:{conversation_id}:{operation_id}");
                let base:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=?")
                    .bind(conversation_id).fetch_one(&pool).await.unwrap();
                let started=generate_id();let terminal=generate_id();
                for (seq,id,kind,value) in [(base+1,&started,"turn/started",json!({})),
                    (base+2,&terminal,"turn/completed",json!({"user_input_request":request}))] {
                    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'answer-fixture',?,?,1,?,?)")
                        .bind(conversation_id).bind(seq).bind(id).bind(id).bind(kind).bind(&operation).bind(value.to_string())
                        .execute(&pool).await.unwrap();
                }
                sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,?, ?,?)")
                    .bind(conversation_id).bind(&operation).bind(&operation).bind(&operation).bind(started).bind(&terminal)
                    .bind(base+1).bind(base+1).bind(base+2).execute(&pool).await.unwrap();
                sqlx::query("UPDATE agent_session_heads SET last_seq=? WHERE session_id=?")
                    .bind(base+2).bind(conversation_id).execute(&pool).await.unwrap();
                return Ok(AttemptOutcome {
                    conversation_id:conversation_id.into(),canonical_terminal:request.as_ref().map(|_|(operation,terminal)),
                    user_input_request:request.clone(),text:Some("本轮已结束。".into()),output_files:Vec::new(),disposition:if request.is_some() {AttemptDisposition::InputRequired} else {AttemptDisposition::Delivered},tokens:Some(1),
                    error:None,error_code:None,error_retryable:Some(false),
                });
            }
            let HarnessRunnerMode::DecisionContinuation { started, calls, release } = &self.mode else {
                return Err(AppError::BadRequest("not a continuation fixture".into()));
            };
            calls.fetch_add(1, Ordering::SeqCst);
            started.store(true, Ordering::SeqCst);
            release.notified().await;
            Ok(AttemptOutcome {
                user_input_request: None, canonical_terminal: None,
                conversation_id: conversation_id.to_owned(),
                text: Some("completed decision continuation".to_owned()),
                output_files: Vec::new(),
                disposition: AttemptDisposition::Delivered,
                tokens: Some(1),
                error: None,
                error_code: None,
                error_retryable: Some(false),
            })
        }
    }

    fn harness_participant(participant_id: String) -> NewAgentExecutionParticipant {
        NewAgentExecutionParticipant {
            participant_id,
            source_agent_id: HARNESS_SOURCE_AGENT_ID.to_owned(),
            preset_id: None,
            preset_revision: None,
            agent_snapshot: None,
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

    async fn make_scheduler_harness_with_policies(
        runner: Arc<HarnessAttemptRunner>,
        titles: &[&str],
        dependencies: &[(&str, &str)],
        max_parallel: i64,
        adaptation_policy: AdaptationPolicy,
        work_dir: Option<&str>,
        tool_policies: &[(&str, AgentToolPolicy)],
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
        runner.bind_pool(database.pool().clone());
        let participant_id = generate_id();
        let created = repository
            .create_execution_with_participants(
                &owner,
                &CreateAgentExecutionParams {
                    goal: "deterministic scheduler integration".to_owned(),
                    status: AgentExecutionStatus::Planning,
                    adaptation_policy,
                    decision_policy: nomifun_common::DecisionPolicy::Automatic,
                    delegation_policy: DelegationPolicy::Automatic,
                    max_parallel,
                    work_dir: work_dir.map(str::to_owned),
                    lead_conversation_id: None,
                    initial_plan_input: r#"{"mode":"explicit","plan":{"steps":[]}}"#.to_owned(),
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
            .map(|(step_id, title)| {
                let mut step = harness_step(step_id.clone(), &participant_id, title);
                if let Some((_, policy)) = tool_policies
                    .iter()
                    .find(|(policy_title, _)| policy_title == title)
                {
                    step.tool_policy = *policy;
                }
                step
            })
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
            Arc::new(NoopConversationEffects::default()),
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
        make_scheduler_harness_with_policies(
            runner,
            titles,
            dependencies,
            max_parallel,
            adaptation_policy,
            work_dir,
            &[],
        )
        .await
    }

        #[tokio::test]
        async fn failed_lead_projection_retries_and_marks_the_same_operation_delivered() {
            let runner = Arc::new(HarnessAttemptRunner::parallel());
            let (mut scheduler, repository, _, data_dir, owner) = make_scheduler_harness(
                runner, &["fixture-step"], &[], 1, AdaptationPolicy::Fixed, None,
            ).await;
            let effects = Arc::new(NoopConversationEffects::default());
            effects.fail_report_once.store(true, Ordering::SeqCst);
            Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
            let pool = sqlx::SqlitePool::connect_with(
                sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
            ).await.unwrap();
            let lead = generate_id();
            sqlx::query(
                "INSERT INTO agent_sessions (\
                    agent_session_id, owner_ref_json, state, title, archived, pinned, \
                    agent_binding_json, next_seq, created_at\
                 ) VALUES (?, ?, 'live', 'lead', 0, 0, '{}', 1, 1)",
            )
                .bind(&lead)
                .bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string())
                .execute(&pool).await.unwrap();
            let execution = repository.create_execution_with_participants(
                &owner,
                &CreateAgentExecutionParams {
                    goal: "report retry".into(), status: AgentExecutionStatus::Completed,
                    adaptation_policy: AdaptationPolicy::Fixed, decision_policy: DecisionPolicy::Automatic,
                    delegation_policy: DelegationPolicy::Automatic, max_parallel: 1,
                    work_dir: None, lead_conversation_id: Some(lead), initial_plan_input: r#"{"mode":"explicit","plan":{"steps":[]}}"#.into(),
                },
                &[harness_participant(generate_id())],
                &system_event(AgentExecutionEventKind::Created, None, None, json!({"lead_report_operation_id":"report-retry"})),
            ).await.unwrap();
            let detail = scheduler.detail(&owner, &execution.execution_id).await.unwrap();
            assert!(scheduler.reconcile_lead_report(&owner, &detail).await.is_err());
            assert!(scheduler.inner.pending_lead_reports.contains_key(&execution.execution_id), "a failed projection must own a retry task");
            tokio::time::timeout(Duration::from_secs(4), async {
                while scheduler.inner.pending_lead_reports.contains_key(&execution.execution_id) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("retry settles");
            assert_eq!(*effects.report_operations.lock().unwrap(), ["report-retry", "report-retry"]);
            let events = repository.list_events(&owner, &execution.execution_id, 0, 100).await.unwrap();
            assert!(events.iter().any(|event| {
                let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
                payload["change"] == "lead_report_delivered" && payload["operation_id"] == "report-retry"
            }));
            scheduler.reconcile_lead_report(&owner, &detail).await.unwrap();
            assert_eq!(effects.report_operations.lock().unwrap().len(), 2, "delivered operation is not reprojected");
            scheduler.shutdown().await.unwrap();
            pool.close().await;
        }

    #[tokio::test]
    async fn durable_stop_and_steer_acknowledge_only_successful_delivery() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let (mut scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness(
            runner, &["fixture-step"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        effects.fail_stop_once.store(true, Ordering::SeqCst);
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let step = &detail.steps[0];
        let pending = AttemptConversationEffects {
            pending_conversation_effects: vec![
                PendingConversationEffect::StopTurn { operation_id: "stop-id".into(), target_operation_id: "target-turn".into() },
                PendingConversationEffect::Steer { operation_id: "steer-id".into(), target_operation_id: "target-turn".into(), content: "next".into() },
            ],
            ..Default::default()
        };
        let created = repository.create_attempt(
            &owner, &execution_id, &step.step_id, step.version, None,
            &CreateAgentExecutionAttemptParams {
                participant_id: step.assigned_participant_id.clone(), start_immediately: false,
                trigger_reason: "test".into(), effective_config: "{}".into(), retry_after: None,
                runtime_state: Some(pending.encode().unwrap()),
            },
            &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), None, json!({})),
        ).await.unwrap();
        let attempt_id = &created.current_attempt.as_ref().unwrap().attempt.attempt_id;
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
        ).await.unwrap();
        let conversation_id = generate_id();
        sqlx::query(
            "INSERT INTO agent_sessions (\
                agent_session_id, owner_ref_json, state, title, archived, pinned, \
                agent_binding_json, next_seq, created_at\
             ) VALUES (?, ?, 'live', 'effect', 0, 0, '{}', 1, 1)",
        )
            .bind(&conversation_id)
            .bind(serde_json::json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(&pool).await.unwrap();
        repository.start_attempt(
            &owner, &execution_id, &step.step_id, created.step.version,
            attempt_id, created.current_attempt.as_ref().unwrap().attempt.version,
            &conversation_id, AgentExecutionAttemptSessionKind::ChildAttempt, None,
            &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), Some(attempt_id), json!({})),
        ).await.unwrap();
        let lease = AgentExecutionLeaseToken::new("effects-test".into());
        let mut jobs = ScheduledAgentJobs::new();
        let mut in_flight = HashSet::new();
        assert!(scheduler.process_one_pending_conversation_effect(&owner, &execution_id, &lease, &mut jobs, &mut in_flight).await.unwrap());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !effects.stop_started.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(scheduler.detail(&owner, &execution_id).await.unwrap().attempts[0].runtime_state.is_some(),
            "a failed native delivery must retain the exact durable head");
        assert!(!scheduler.process_one_pending_conversation_effect(&owner, &execution_id, &lease, &mut jobs, &mut in_flight).await.unwrap(),
            "one Attempt must not have two control deliveries in flight");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if scheduler.detail(&owner, &execution_id).await.unwrap().attempts[0].runtime_state.is_none() { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("the failed queue head retries before the later steer");
        let events = repository.list_events(&owner, &execution_id, 0, 100).await.unwrap();
        let delivered = events.iter().filter_map(|event| {
            let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
            (payload["change"] == "conversation_effect_delivered").then(|| payload["effect"].as_str().unwrap().to_owned())
        }).collect::<Vec<_>>();
        assert_eq!(delivered, ["stop_turn", "steer"]);
        assert!(!scheduler.process_one_pending_conversation_effect(&owner, &execution_id, &lease, &mut jobs, &mut in_flight).await.unwrap());
        assert_eq!(*effects.conversation_operations.lock().unwrap(), ["stop-id", "stop-id", "steer-id"]);
        assert!(scheduler.detail(&owner, &execution_id).await.unwrap().attempts[0].runtime_state.is_none());
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cleanup_worker_reaches_item_101_while_the_first_page_keeps_failing() {
        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let titles=(0..101).map(|index|format!("cleanup-{index}")).collect::<Vec<_>>();
        let (mut scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness(
            runner, &titles.iter().map(String::as_str).collect::<Vec<_>>(), &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
        ).await.unwrap();
        let mut conversations = Vec::new();
        for index in 0..101 {
            let (step_id,attempt_id,conversation_id,fixture_pool)=start_harness_attempt(&scheduler,&repository,
                &owner,&execution_id,&data_dir,&titles[index],None).await;
            let running=repository.get_step_detail(&owner,&execution_id,&step_id).await.unwrap().unwrap();
            let attempt=&running.current_attempt.as_ref().unwrap().attempt;
            repository.settle_attempt(&owner,&execution_id,&step_id,running.step.version,&attempt_id,attempt.version,None,
                &SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:None,
                    attempt_status:ExecutionAttemptStatus::Completed,step_status:ExecutionStepStatus::Completed,execution_status:None,
                    question:None,error:None,output_summary:Some(Some("finished".into())),output_files:None,tokens:None,retry_after:None,
                    runtime_state:None,started_at:None,finished_at:None,loop_repeat_reset:None},
                &system_event(AgentExecutionEventKind::AttemptChanged,Some(&step_id),Some(&attempt_id),json!({}))).await.unwrap();
            fixture_pool.close().await;
            if index < 100 {
                effects.failed_cleanup_conversations.lock().unwrap().insert(conversation_id.clone());
            }
            conversations.push(conversation_id);
        }
        scheduler.schedule_cleanup_reconciliation();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let acknowledged: Option<i64> = sqlx::query_scalar(
                    "SELECT cleanup_completed_at FROM conversation_execution_links WHERE conversation_id=?",
                ).bind(&conversations[100]).fetch_one(&pool).await.unwrap();
                if acknowledged.is_some() { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("the worker must advance its cursor past 100 failing cleanup owners");
        let remaining = repository.list_pending_conversation_cleanups(None, None, 100).await.unwrap();
        assert_eq!(remaining.cleanups.len(), 100, "failures stay pending for a later sweep");
        assert!(!remaining.cleanups.iter().any(|cleanup| cleanup.conversation_id == conversations[100]));
        assert_eq!(effects.cancelled_conversations.lock().unwrap().iter()
            .filter(|conversation| **conversation == conversations[100]).count(), 1);
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn autowork_scheduler_keeps_delegated_attempt_sessions_distinct_from_the_original_root() {
        use nomifun_db::IRequirementRepository;

        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let (scheduler, repository, _, data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["independent"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
        ).await.unwrap();
        let lead = generate_id();
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
            VALUES(?,json_object('principal_kind','user','principal_id',?),'live','permanent AutoWork Session',0,0,'{}',1,1)")
            .bind(&lead).bind(&owner).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) \
            VALUES(?,'ready',NULL,0,0,0)")
            .bind(&lead).execute(&pool).await.unwrap();
        let requirements = nomifun_db::SqliteRequirementRepository::new(pool.clone());
        requirements.insert(&nomifun_db::models::NewRequirementRow {
            title: "original automatic task".into(), content: "retain the original root".into(),
            tag: "automation-dispatch".into(), order_key: String::new(), sort_seq: String::new(),
            status: "pending".into(), priority: 0, completion_note: None, owner_conversation_id: None,
            owner_terminal_id: None, active_turn_started_at: None, lease_expires_at: None,
            started_at: None, completed_at: None, attempt_count: 0, created_by: "user".into(),
            extra: "{}".into(), created_at: 1, updated_at: 1,
        }).await.unwrap();
        let claim = requirements.claim_next_for_runner("automation-dispatch", Some(&lead), None, 30_000, now_ms())
            .await.unwrap().unwrap().row;
        let participant_id = generate_id();
        let created = repository.create_execution_with_participants(&owner, &CreateAgentExecutionParams {
            goal: "original automatic task".into(), status: AgentExecutionStatus::Planning,
            adaptation_policy: AdaptationPolicy::Fixed, decision_policy: DecisionPolicy::Automatic,
            delegation_policy: DelegationPolicy::Automatic, max_parallel: 1, work_dir: None,
            lead_conversation_id: Some(lead.clone()),
            initial_plan_input: json!({"mode":"automation","source":{
                "requirement_id":claim.requirement_id,"claim_generation":claim.claim_generation,
                "operation_id":"automation-dispatch-source"},"plan":{"steps":[]}}).to_string(),
        }, &[harness_participant(participant_id.clone())],
            &system_event(AgentExecutionEventKind::Created, None, None, json!({}))).await.unwrap();
        let root_id = generate_id();
        repository.reconcile_plan(&owner, &created.execution_id, created.version, &ReconcileAgentExecutionPlanParams {
            goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
            keep_step_ids: Vec::new(), new_participants: Vec::new(), retire_participant_ids: Vec::new(),
            new_steps: vec![harness_step(root_id.clone(), &participant_id, "original automatic task")],
            new_dependencies: Vec::new(), execution_status: AgentExecutionStatus::Running,
        }, &system_event(AgentExecutionEventKind::PlanChanged, None, None, json!({}))).await.unwrap();

        let original = scheduler.detail(&owner, &created.execution_id).await.unwrap();
        let lease = AgentExecutionLeaseToken::new("automation-dispatch-owner".into());
        repository.try_acquire_lease(&created.execution_id, original.execution.version, lease.owner(), now_ms() + 30_000)
            .await.unwrap().unwrap();
        scheduler.execute_agent_step(&owner, &created.execution_id, original.steps[0].clone(), &lease).await.unwrap();
        let root_result = scheduler.detail(&owner, &created.execution_id).await.unwrap();
        assert_eq!(root_result.steps[0].status, ExecutionStepStatus::Completed);
        assert_eq!(root_result.attempts[0].conversation_id.as_deref(), Some(lead.as_str()));

        // These are appended in a later materialized revision, as delegation
        // does. The first child even has the same role and title as the root;
        // a second child changes only role. Presentation must not select lead.
        let child_ids = [generate_id(), generate_id(), generate_id()];
        let mut children = child_ids.iter().map(|id|
            harness_step(id.clone(), &participant_id, "original automatic task")).collect::<Vec<_>>();
        children[1].role = Some("reviewer".into());
        repository.reconcile_plan(&owner, &created.execution_id, root_result.execution.version, &ReconcileAgentExecutionPlanParams {
            goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
            keep_step_ids: vec![root_id.clone()], new_participants: Vec::new(), retire_participant_ids: Vec::new(),
            new_steps: children, new_dependencies: Vec::new(), execution_status: AgentExecutionStatus::Running,
        }, &system_event(AgentExecutionEventKind::PlanChanged, Some(&root_id), Some(&root_result.attempts[0].attempt_id),
            json!({"change":"delegate","caller_step_id":root_id,"added_step_ids":child_ids}))).await.unwrap();
        let delegated = scheduler.detail(&owner, &created.execution_id).await.unwrap();
        assert_eq!(delegated.automation_control.as_ref().unwrap().root_step_id.as_deref(), Some(root_id.as_str()),
            "scheduler consumes the original shared identity after later revisions");
        for id in &child_ids[..2] {
            scheduler.execute_agent_step(&owner, &created.execution_id,
                delegated.steps.iter().find(|step| step.step_id == *id).unwrap().clone(), &lease).await.unwrap();
        }
        let completed = scheduler.detail(&owner, &created.execution_id).await.unwrap();
        assert_eq!(completed.attempts.len(), 3);
        let links = repository.list_conversation_links(&owner, &created.execution_id).await.unwrap();
        let root_links = links.iter().filter(|link| link.relation == "automation").collect::<Vec<_>>();
        assert_eq!(root_links.len(), 1);
        assert_eq!(root_links[0].step_id.as_deref(), Some(root_id.as_str()));
        assert_eq!(root_links[0].conversation_id, lead);
        let mut child_sessions = HashSet::new();
        for id in &child_ids[..2] {
            let attempt = completed.attempts.iter().find(|attempt| attempt.step_id == *id).unwrap();
            assert_eq!(attempt.status, ExecutionAttemptStatus::Completed);
            let session = attempt.conversation_id.as_ref().unwrap();
            assert_ne!(session, &lead);
            assert!(child_sessions.insert(session));
            assert_eq!(attempt.effective_config["session_kind"], "attempt");
            assert!(links.iter().any(|link| link.relation == "attempt" && link.step_id.as_deref() == Some(id.as_str())
                && link.attempt_id.as_deref() == Some(attempt.attempt_id.as_str()) && link.conversation_id == *session));
        }
        assert_eq!(runner.call_count("original automatic task"), 3);

        // Independent child Sessions still consume the same original claim
        // admission guard. Changing the Session kind must not bypass Stop.
        requirements.resolve_claim_exact(&claim.requirement_id, claim.claim_generation, claim.claim_token.as_deref().unwrap(),
            Some(&lead), None, &nomifun_db::RequirementClaimResolution::Cancelled { completion_note: None }, now_ms())
            .await.unwrap().unwrap();
        let pending = completed.steps.iter().find(|step| step.step_id == child_ids[2]).unwrap().clone();
        assert!(matches!(scheduler.execute_agent_step(&owner, &created.execution_id, pending, &lease).await,
            Err(AppError::Conflict(_))));
        assert_eq!(scheduler.detail(&owner, &created.execution_id).await.unwrap().attempts.len(), 3);
        assert_eq!(runner.call_count("original automatic task"), 3, "cancelled source never creates another child Session");
        let state: String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(&lead).fetch_one(&pool).await.unwrap();
        assert_eq!(state, "live");
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn soft_deleted_automation_root_sends_exact_cleanup_without_disposing_the_user_session() {
        use nomifun_db::IRequirementRepository;
        let runner=Arc::new(HarnessAttemptRunner::parallel());
        let (mut scheduler,repository,_,data_dir,owner)=make_scheduler_harness(runner,&["independent"],&[],1,AdaptationPolicy::Fixed,None).await;
        let effects=Arc::new(NoopConversationEffects::default());
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects=effects.clone();
        let pool=sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite"))).await.unwrap();
        let session=generate_id();
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
            VALUES(?,json_object('principal_kind','user','principal_id',?),'live','permanent AutoWork Session',0,0,'{}',1,1)")
            .bind(&session).bind(&owner).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'ready',NULL,0,0,0)")
            .bind(&session).execute(&pool).await.unwrap();
        let requirements=nomifun_db::SqliteRequirementRepository::new(pool.clone());
        requirements.insert(&nomifun_db::models::NewRequirementRow {title:"current automatic work".into(),content:"original task".into(),tag:"cleanup-root".into(),
            order_key:String::new(),sort_seq:String::new(),status:"pending".into(),priority:0,completion_note:None,owner_conversation_id:None,owner_terminal_id:None,
            active_turn_started_at:None,lease_expires_at:None,started_at:None,completed_at:None,attempt_count:0,created_by:"user".into(),extra:"{}".into(),created_at:1,updated_at:1}).await.unwrap();
        let claim=requirements.claim_next_for_runner("cleanup-root",Some(&session),None,30_000,2).await.unwrap().unwrap().row;
        let participant_id=generate_id();
        let created=repository.create_execution_with_participants(&owner,&CreateAgentExecutionParams {goal:"original automatic task".into(),
            status:AgentExecutionStatus::Planning,adaptation_policy:AdaptationPolicy::Fixed,decision_policy:DecisionPolicy::Automatic,
            delegation_policy:DelegationPolicy::Automatic,max_parallel:1,work_dir:None,lead_conversation_id:Some(session.clone()),
            initial_plan_input:json!({"mode":"automation","source":{"requirement_id":claim.requirement_id,"claim_generation":claim.claim_generation,
                "operation_id":"automatic-root-source"},"plan":{"steps":[]}}).to_string()},
            &[harness_participant(participant_id.clone())],&system_event(AgentExecutionEventKind::Created,None,None,json!({}))).await.unwrap();
        let mut root=harness_step(generate_id(),&participant_id,"automatic root");root.tool_policy=AgentToolPolicy::ReadOnly;
        let planned=repository.reconcile_plan(&owner,&created.execution_id,created.version,&ReconcileAgentExecutionPlanParams {
            goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:Vec::new(),new_participants:Vec::new(),retire_participant_ids:Vec::new(),
            new_steps:vec![root],new_dependencies:Vec::new(),execution_status:AgentExecutionStatus::Running},&system_event(AgentExecutionEventKind::PlanChanged,None,None,json!({}))).await.unwrap();
        let root=&planned.steps[0];
        let queued=repository.create_attempt(&owner,&created.execution_id,&root.step_id,root.version,None,&CreateAgentExecutionAttemptParams {
            participant_id:Some(participant_id),start_immediately:false,trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},
            &system_event(AgentExecutionEventKind::AttemptChanged,None,None,json!({}))).await.unwrap();
        let attempt=&queued.current_attempt.as_ref().unwrap().attempt;
        repository.start_attempt(&owner,&created.execution_id,&root.step_id,queued.step.version,&attempt.attempt_id,attempt.version,&session,
            AgentExecutionAttemptSessionKind::AutomationLead,None,&system_event(AgentExecutionEventKind::AttemptChanged,None,None,json!({}))).await.unwrap();
        let current=repository.get_execution(&owner,&created.execution_id).await.unwrap().unwrap();
        repository.delete_execution(&owner,&created.execution_id,current.version,&system_event(AgentExecutionEventKind::Deleted,None,None,json!({}))).await.unwrap();
        assert!(repository.get_attempt(&owner,&created.execution_id,&root.step_id,&attempt.attempt_id).await.unwrap().is_none());
        assert_eq!(repository.automation_source_cleanup_execution(&owner,&claim.requirement_id,claim.claim_generation,"automatic-root-source").await.unwrap(),
            Some(created.execution_id.clone()),"soft deletion cannot fabricate absence of its original cleanup source");
        scheduler.reconcile_conversation_cleanup(Some(&created.execution_id)).await;
        assert_eq!(*effects.cancelled_targets.lock().unwrap(),[(attempt.attempt_id.clone(),format!("turn:user:{owner}:{session}:{}:initial-turn",attempt.attempt_id))],
            "the automation relation must dispatch its exact root Stop through the original owner");
        assert!(repository.list_pending_conversation_cleanups(Some(&created.execution_id),None,100).await.unwrap().cleanups.is_empty());
        let state:String=sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?").bind(&session).fetch_one(&pool).await.unwrap();
        assert_eq!(state,"live","automation cleanup never disposes of its permanent user Session");
        scheduler.shutdown().await.unwrap();pool.close().await;
    }

    async fn start_harness_attempt(
        scheduler: &ExecutionScheduler,
        repository: &SqliteAgentExecutionRepository,
        owner: &str,
        execution_id: &str,
        data_dir: &TempDir,
        title: &str,
        runtime_state: Option<String>,
    ) -> (String, String, String, SqlitePool) {
        let detail = scheduler.detail(owner, execution_id).await.unwrap();
        let step = detail.steps.iter().find(|step| step.title == title).unwrap();
        let created = repository.create_attempt(
            owner, execution_id, &step.step_id, step.version, None,
            &CreateAgentExecutionAttemptParams {
                participant_id: step.assigned_participant_id.clone(), start_immediately: false,
                trigger_reason: "interruption-fixture".into(), effective_config: "{}".into(),
                retry_after: None, runtime_state,
            }, &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), None, json!({})),
        ).await.unwrap();
        let attempt = &created.current_attempt.as_ref().unwrap().attempt;
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
        ).await.unwrap();
        let conversation_id = generate_id();
        sqlx::query(
            "INSERT INTO agent_sessions (agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
             VALUES (?,?,'live','interrupted attempt',0,0,'{}',1,1)"
        ).bind(&conversation_id).bind(json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(&pool).await.unwrap();
        repository.start_attempt(
            owner, execution_id, &step.step_id, created.step.version,
            &attempt.attempt_id, attempt.version, &conversation_id,
            AgentExecutionAttemptSessionKind::ChildAttempt, None,
            &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), Some(&attempt.attempt_id), json!({})),
        ).await.unwrap();
        (step.step_id.clone(), attempt.attempt_id.clone(), conversation_id, pool)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocked_turn_stop_releases_scheduler_on_cancel_without_acknowledging_the_intent() {
        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let (mut scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["blocked-stop"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        effects.block_stop.store(true, Ordering::SeqCst);
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let pending = AttemptConversationEffects {
            pending_conversation_effects: vec![PendingConversationEffect::StopTurn {
                operation_id: "blocked-stop-id".into(), target_operation_id: "original-turn".into(),
            }],
            ..Default::default()
        };
        let (_, attempt_id, _, pool) = start_harness_attempt(
            &scheduler, &repository, &owner, &execution_id, &data_dir, "blocked-stop", Some(pending.encode().unwrap()),
        ).await;
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !effects.stop_started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("scheduler reached the blocked canonical Turn stop");
        scheduler.stop(&execution_id);
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.inner.active.contains_key(&execution_id) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("a blocked control delivery must not hold the stopped scheduler or its lease");
        let current = scheduler.detail(&owner, &execution_id).await.unwrap();
        let retained = current.attempts.iter().find(|entry| entry.attempt_id == attempt_id).unwrap();
        assert_eq!(retained.status, ExecutionAttemptStatus::Running);
        assert_eq!(serde_json::from_value::<AttemptConversationEffects>(retained.runtime_state.clone().unwrap()).unwrap(), pending);
        assert_eq!(runner.call_count("blocked-stop"), 0, "cancelling transport cannot replay the interrupted Turn");
        let row = repository.get_execution(&owner, &execution_id).await.unwrap().unwrap();
        assert!(row.lease_owner.is_none());
        effects.stop_release.notify_one();
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn linked_receipt_query_failure_preserves_the_attempt_instead_of_retry_or_cleanup() {
        let mut mode = HarnessAttemptRunner::overlap_probe();
        mode.mode = HarnessRunnerMode::ReceiptQueryFailure;
        let runner = Arc::new(mode);
        let (scheduler, repository, execution_id, _data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["original"], &[], 1, AdaptationPolicy::Adaptive, None,
        ).await;
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let lease = AgentExecutionLeaseToken::new("query-failure-owner".into());
        repository.try_acquire_lease(&execution_id, detail.execution.version, lease.owner(), now_ms() + 30_000)
            .await.unwrap().unwrap();
        assert!(matches!(scheduler.execute_agent_step(&owner, &execution_id, detail.steps[0].clone(), &lease).await,
            Err(AppError::Timeout(_))));
        let after = scheduler.detail(&owner, &execution_id).await.unwrap();
        assert_eq!(after.execution.status, AgentExecutionStatus::Running);
        assert_eq!(after.steps[0].status, ExecutionStepStatus::Running);
        assert_eq!(after.attempts.len(), 1);
        assert_eq!(after.attempts[0].status, ExecutionAttemptStatus::Running);
        assert!(after.attempts[0].retry_after.is_none() && after.attempts[0].finished_at.is_none());
        assert_eq!(runner.call_count("original"), 1);
        let links = repository.list_conversation_links(&owner, &execution_id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert!(links[0].active && links[0].cleanup_completed_at.is_none());
        scheduler.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn recovery_observer_reattaches_the_same_attempt_and_settles_without_reexecution() {
        let runner = Arc::new(HarnessAttemptRunner::steered_turn());
        let HarnessRunnerMode::SteeredTurn { started, release } = &runner.mode else { unreachable!(); };
        let (scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["original"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let (step_id, attempt_id, session, pool) = start_harness_attempt(
            &scheduler, &repository, &owner, &execution_id, &data_dir, "original", None,
        ).await;
        let operation = format!("turn:user:{owner}:{session}:{attempt_id}:initial-turn");
        let start = generate_id();
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'observer-fixture',?,'turn/started',1,?,'{}')")
            .bind(&session).bind(&start).bind(&start).bind(&operation).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,accepted_at,started_at,execution_lease_until) VALUES(?,?,?,?,'running',?,1,1,0)")
            .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(start).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,1,0)")
            .bind(&session).bind(&operation).execute(&pool).await.unwrap();
        let execution = repository.get_execution(&owner, &execution_id).await.unwrap().unwrap();
        let lease = AgentExecutionLeaseToken::new("reattached-observer".into());
        repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), now_ms() + 30_000)
            .await.unwrap().unwrap();
        let mut jobs = ScheduledAgentJobs::new();
        let mut in_flight = HashSet::new();
        scheduler.recover_interrupted(&owner, &execution_id, &lease, &mut jobs, &mut in_flight).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert!(in_flight.contains(&step_id));
        let links = repository.resolve_conversation_link(&owner, &session).await.unwrap();
        assert!(links[0].active && links[0].cleanup_completed_at.is_none());
        let next = jobs.next();
        tokio::pin!(next);
        tokio::select! {
            result = &mut next => panic!("the original native owner has not settled: {result:?}"),
            _ = async { while !started.load(Ordering::SeqCst) { tokio::task::yield_now().await; } } => {}
        }
        assert_eq!(runner.call_count("original"), 0, "recovery cannot execute the Step again");
        release.notify_one();
        let (finished_step, outcome) = next.await.unwrap();
        outcome.unwrap();
        assert_eq!(finished_step, step_id);
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(detail.attempts[0].attempt_id, attempt_id);
        assert_eq!(detail.attempts[0].status, ExecutionAttemptStatus::Completed);
        assert_eq!(detail.attempts[0].output_summary.as_deref(), Some("original recovered result"));
        assert_eq!(runner.call_count("original"), 0);
        assert_eq!(runner.calls.lock().unwrap().as_slice(), [format!("observe:{attempt_id}:initial-turn")]);
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn interrupted_turn_review_can_be_cancelled_after_its_child_link_was_retired() {
        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let (mut scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["interrupted", "not-started"], &[("interrupted", "not-started")],
            1, AdaptationPolicy::Fixed, None,
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let (_, attempt_id, conversation_id, pool) = start_harness_attempt(
            &scheduler, &repository, &owner, &execution_id, &data_dir, "interrupted", None,
        ).await;
        let operation = format!("turn:user:{owner}:{conversation_id}:{attempt_id}:initial-turn");
        let started_event = generate_id();
        sqlx::query(
            "INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) \
             VALUES(?,1,?,'interrupted-fixture',?,'turn/started',1,?,'{}')"
        ).bind(&conversation_id).bind(&started_event).bind(&started_event).bind(&operation)
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,accepted_at,started_at) \
             VALUES(?,?,?,?,'running',?,1,1)"
        ).bind(&conversation_id).bind(&operation).bind(&operation).bind(&operation).bind(&started_event)
            .execute(&pool).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        let waiting = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let current = scheduler.detail(&owner, &execution_id).await.unwrap();
                if current.execution.status == AgentExecutionStatus::WaitingInput
                    && !scheduler.inner.active.contains_key(&execution_id) {
                    break current;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("interrupted Turn is parked for review without model replay");
        let interrupted = waiting.attempts.iter().find(|entry| entry.attempt_id == attempt_id).unwrap();
        let review = interrupted.runtime_state.clone().unwrap();
        assert_eq!(review["review_blocked"]["receipt_state"], "running");
        assert_eq!(review["review_blocked"]["operation_id"], operation);
        let links = repository.resolve_conversation_link(&owner, &conversation_id).await.unwrap();
        assert!(links.iter().all(|link| !link.active));
        assert_eq!(*effects.cancelled_conversations.lock().unwrap(), [conversation_id.clone()],
            "retiring the recovery link must still dispatch its child cleanup");
        repository.cancel_execution(
            &owner, &execution_id, waiting.execution.version,
            &system_event(AgentExecutionEventKind::StatusChanged, None, None,
                json!({"status":"cancelled","reason":"cancelled_by_caller"})),
        ).await.unwrap();
        scheduler.stop(&execution_id);
        scheduler.reconcile_conversation_cleanup_strict(&execution_id).await.unwrap();
        let cancelled = scheduler.detail(&owner, &execution_id).await.unwrap();
        assert_eq!(cancelled.execution.status, AgentExecutionStatus::Cancelled);
        assert!(cancelled.steps.iter().all(|step| step.status == ExecutionStepStatus::Cancelled));
        let interrupted = cancelled.attempts.iter().find(|entry| entry.attempt_id == attempt_id).unwrap();
        assert_eq!(interrupted.status, ExecutionAttemptStatus::Cancelled);
        assert!(interrupted.question.is_none());
        assert_eq!(interrupted.runtime_state.as_ref(), Some(&review), "cancellation preserves the unknown prior effect evidence");
        assert!(repository.list_pending_conversation_cleanups(Some(&execution_id), None, 100).await.unwrap().cleanups.is_empty());
        assert_eq!(runner.call_count("interrupted"), 0);
        assert_eq!(runner.call_count("not-started"), 0);
        let state: String = sqlx::query_scalar("SELECT state FROM agent_turns WHERE operation_id=?")
            .bind(&operation).fetch_one(&pool).await.unwrap();
        assert_eq!(state, "running", "an external cleanup adapter cannot fabricate a canonical terminal receipt");
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn decision_continuation_keeps_sibling_dag_and_cancellation_responsive() {
        let runner = Arc::new(HarnessAttemptRunner::decision_continuation());
        let HarnessRunnerMode::DecisionContinuation { started, calls, release: _ } = &runner.mode else {
            unreachable!();
        };
        let (scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness_with_policies(
            runner.clone(), &["decision-step", "sibling-root", "sibling-downstream"],
            &[("sibling-root", "sibling-downstream")],
            2, AdaptationPolicy::Fixed, None,
            &[("decision-step", AgentToolPolicy::ReadOnly), ("sibling-root", AgentToolPolicy::ReadOnly), ("sibling-downstream", AgentToolPolicy::ReadOnly)],
        ).await;
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let step = detail.steps.iter().find(|step| step.title == "decision-step").unwrap();
        let pending = AttemptConversationEffects {
            pending_conversation_effects: vec![PendingConversationEffect::DecisionInput {
                operation_id: "decision-answer".into(), content: "continue".into(),
            }],
            ..Default::default()
        };
        let created = repository.create_attempt(
            &owner, &execution_id, &step.step_id, step.version, None,
            &CreateAgentExecutionAttemptParams {
                participant_id: step.assigned_participant_id.clone(), start_immediately: false,
                trigger_reason: "decision-fixture".into(), effective_config: "{}".into(),
                retry_after: None, runtime_state: Some(pending.encode().unwrap()),
            }, &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), None, json!({})),
        ).await.unwrap();
        let attempt = &created.current_attempt.as_ref().unwrap().attempt;
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(data_dir.path().join("harness.sqlite")),
        ).await.unwrap();
        let conversation_id = generate_id();
        sqlx::query(
            "INSERT INTO agent_sessions (agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) \
             VALUES (?,?,'live','decision',0,0,'{}',1,1)"
        ).bind(&conversation_id).bind(json!({"principal_kind":"user","principal_id":owner}).to_string())
            .execute(&pool).await.unwrap();
        repository.start_attempt(
            &owner, &execution_id, &step.step_id, created.step.version,
            &attempt.attempt_id, attempt.version, &conversation_id,
            AgentExecutionAttemptSessionKind::ChildAttempt, None,
            &system_event(AgentExecutionEventKind::AttemptChanged, Some(&step.step_id), Some(&attempt.attempt_id), json!({})),
        ).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let current = scheduler.detail(&owner, &execution_id).await.unwrap();
                if started.load(Ordering::SeqCst) && current.steps.iter().any(|step| {
                    step.title == "sibling-downstream" && step.status == ExecutionStepStatus::Completed
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("a blocked decision must not starve independent sibling dispatch or settlement");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A nudge must not enqueue the same write-ahead continuation twice.
        scheduler.start(owner.clone(), execution_id.clone());
        scheduler.stop(&execution_id);
        tokio::time::timeout(Duration::from_secs(1), scheduler.shutdown())
            .await.expect("cancellation must not await the blocked continuation").unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let current = scheduler.detail(&owner, &execution_id).await.unwrap();
        let decision = current.attempts.iter().find(|attempt| attempt.step_id == step.step_id).unwrap();
        assert_eq!(decision.status, ExecutionAttemptStatus::Running);
        assert!(decision.runtime_state.is_some(), "cancellation must retain the durable continuation identity");
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduling_pause_settles_the_original_job_and_resume_dispatches_only_remaining_work() {
        let runner = Arc::new(HarnessAttemptRunner::steered_turn());
        let HarnessRunnerMode::SteeredTurn { started, release } = &runner.mode else { unreachable!(); };
        let (scheduler, repository, execution_id, _data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["active-before-pause", "remaining-after-resume"],
            &[("active-before-pause", "remaining-after-resume")], 1, AdaptationPolicy::Fixed, None,
        ).await;
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !started.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        let before = scheduler.detail(&owner, &execution_id).await.unwrap();
        let original_attempt = before.attempts[0].attempt_id.clone();
        repository.pause_execution(&owner, &execution_id, before.execution.version,
            &system_event(AgentExecutionEventKind::StatusChanged, None, None, json!({"status":"paused"})),
        ).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        release.notify_one();
        let paused = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
                if detail.steps.iter().any(|step| step.title == "active-before-pause" && step.status == ExecutionStepStatus::Completed)
                    && !scheduler.inner.active.contains_key(&execution_id) { break detail; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("dispatch pause must keep polling and settle the original admitted job");
        assert_eq!(paused.execution.status, AgentExecutionStatus::Paused);
        assert_eq!(paused.execution.max_parallel, 1);
        assert_eq!(paused.attempts.len(), 1);
        assert_eq!(paused.attempts[0].attempt_id, original_attempt);
        assert_eq!(runner.call_count("active-before-pause"), 1);
        assert_eq!(runner.call_count("remaining-after-resume"), 0, "pause blocks new dispatch");
        assert_eq!(paused.steps.iter().find(|step| step.title == "remaining-after-resume").unwrap().status, ExecutionStepStatus::Pending);
        repository.resume_execution(&owner, &execution_id, paused.execution.version,
            &system_event(AgentExecutionEventKind::StatusChanged, None, None, json!({"status":"running"})),
        ).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            while runner.call_count("remaining-after-resume") == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        release.notify_one();
        let terminal = wait_for_terminal(&repository, &owner, &execution_id).await;
        assert_eq!(terminal.execution.status, "completed");
        assert_eq!(terminal.attempts.len(), 2);
        assert_eq!(runner.call_count("active-before-pause"), 1, "resume cannot replay an already settled Turn");
        assert_eq!(runner.call_count("remaining-after-resume"), 1);
        scheduler.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocked_steer_preserves_fifo_while_siblings_settle_and_user_stop_preempts() {
        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let (mut scheduler, repository, execution_id, data_dir, owner) = make_scheduler_harness_with_policies(
            runner, &["control-target", "sibling-root", "sibling-child"],
            &[("sibling-root", "sibling-child")], 2, AdaptationPolicy::Fixed, None,
            &[("control-target", AgentToolPolicy::ReadOnly), ("sibling-root", AgentToolPolicy::ReadOnly), ("sibling-child", AgentToolPolicy::ReadOnly)],
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        effects.block_steer.store(true, Ordering::SeqCst);
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let (step_id, attempt_id, conversation_id, pool) = start_harness_attempt(
            &scheduler, &repository, &owner, &execution_id, &data_dir, "control-target", None,
        ).await;
        let target_turn=format!("turn:user:{owner}:{conversation_id}:{attempt_id}:initial-turn");
        let mut queue = AttemptConversationEffects::default();
        queue.push_steer("steer-first".into(), target_turn.clone(), "first".into()).unwrap();
        let initial=scheduler.detail(&owner,&execution_id).await.unwrap();
        let initial_step=initial.steps.iter().find(|step|step.step_id==step_id).unwrap();
        let initial_attempt=initial.attempts.iter().find(|attempt|attempt.attempt_id==attempt_id).unwrap();
        repository.enqueue_attempt_conversation_effect(&owner,&execution_id,initial.execution.version,&step_id,initial_step.version,
            &attempt_id,initial_attempt.version,&AttemptConversationEffectParams {runtime_state:Some(queue.encode().unwrap())},
            &system_event(AgentExecutionEventKind::StepChanged,Some(&step_id),Some(&attempt_id),json!({"effect":"steer"}))).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
                if effects.steer_started.load(Ordering::SeqCst)
                    && detail.steps.iter().any(|step| step.title == "sibling-child" && step.status == ExecutionStepStatus::Completed) { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("a stalled native Steer must not block independent sibling dispatch/settlement");
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let step = detail.steps.iter().find(|step| step.step_id == step_id).unwrap();
        let attempt = detail.attempts.iter().find(|attempt| attempt.attempt_id == attempt_id).unwrap();
        queue.push_steer("steer-second".into(), target_turn.clone(), "second".into()).unwrap();
        repository.enqueue_attempt_conversation_effect(
            &owner, &execution_id, detail.execution.version, &step_id, step.version, &attempt_id, attempt.version,
            &AttemptConversationEffectParams { runtime_state: Some(queue.encode().unwrap()) },
            &system_event(AgentExecutionEventKind::StepChanged, Some(&step_id), Some(&attempt_id), json!({"effect":"steer"})),
        ).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        assert_eq!(*effects.conversation_operations.lock().unwrap(), ["steer-first"], "the next Steer cannot overtake its original owner");

        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let step = detail.steps.iter().find(|step| step.step_id == step_id).unwrap();
        let mut stop_event = system_event(AgentExecutionEventKind::StepChanged, Some(&step_id), Some(&attempt_id), json!({}));
        stop_event.actor = nomifun_common::AgentExecutionActor::User { user_id: owner.clone() };
        repository.cancel_step(&owner, &execution_id, detail.execution.version, &step_id, step.version, "task".parse().unwrap(),
            &stop_event,
        ).await.unwrap();
        scheduler.reconcile_conversation_cleanup(Some(&execution_id)).await;
        assert!(effects.cancelled_conversations.lock().unwrap().contains(&conversation_id), "user Stop reaches the exact cleanup owner while the Steer is still in flight");
        assert!(scheduler.inner.conversation_controls.contains_key(&attempt_id), "Stop does not drop the old native operation");
        let stopped = scheduler.detail(&owner, &execution_id).await.unwrap();
        let stopped_attempt = stopped.attempts.iter().find(|attempt| attempt.attempt_id == attempt_id).unwrap();
        assert_eq!(stopped_attempt.status, ExecutionAttemptStatus::Cancelled);
        let stopped_state = stopped_attempt.runtime_state.clone();

        effects.block_steer.store(false, Ordering::SeqCst);
        effects.steer_release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            while scheduler.inner.conversation_controls.contains_key(&attempt_id) { tokio::task::yield_now().await; }
        }).await.expect("the original control owner settles its late result");
        let after = scheduler.detail(&owner, &execution_id).await.unwrap();
        let after_attempt = after.attempts.iter().find(|attempt| attempt.attempt_id == attempt_id).unwrap();
        assert_eq!(after_attempt.status, ExecutionAttemptStatus::Cancelled);
        assert_eq!(after_attempt.runtime_state, stopped_state, "late acknowledgement cannot clear the durable stopped scope");
        assert_eq!(*effects.conversation_operations.lock().unwrap(), ["steer-first"], "queued controls do not revive a cancelled task");
        scheduler.shutdown().await.unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn successful_steer_metadata_does_not_discard_the_running_turn_completion() {
        let runner = Arc::new(HarnessAttemptRunner::steered_turn());
        let HarnessRunnerMode::SteeredTurn { started, release } = &runner.mode else {
            unreachable!();
        };
        let (scheduler, repository, execution_id, _data_dir, owner) = make_scheduler_harness(
            runner.clone(), &["steered-step"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("the original Turn must start");
        let detail = scheduler.detail(&owner, &execution_id).await.unwrap();
        let step = &detail.steps[0];
        let attempt = &detail.attempts[0];
        let original_attempt_version = attempt.version;
        let mut effects = AttemptConversationEffects::default();
        effects.push_steer("steer-one".into(), "target-turn".into(), "incorporate this correction".into()).unwrap();
        let enqueued = repository.enqueue_attempt_conversation_effect(
            &owner, &execution_id, detail.execution.version, &step.step_id, step.version,
            &attempt.attempt_id, attempt.version,
            &AttemptConversationEffectParams { runtime_state: Some(effects.encode().unwrap()) },
            &system_event(AgentExecutionEventKind::StepChanged, Some(&step.step_id), Some(&attempt.attempt_id), json!({"effect":"steer"})),
        ).await.unwrap();
        let acknowledged = repository.acknowledge_attempt_conversation_effect(
            &owner, &execution_id, &step.step_id, &attempt.attempt_id,
            enqueued.detail.current_attempt.as_ref().unwrap().attempt.version,
            &AttemptConversationEffectParams { runtime_state: None },
            &system_event(AgentExecutionEventKind::StepChanged, Some(&step.step_id), Some(&attempt.attempt_id), json!({"effect":"steer_delivered"})),
        ).await.unwrap();
        assert_eq!(acknowledged.step.version, step.version, "a steer does not replace the invocation generation");
        assert!(acknowledged.current_attempt.as_ref().unwrap().attempt.version > original_attempt_version,
            "metadata commands still need their own optimistic CAS");
        release.notify_one();
        let terminal = wait_for_terminal(&repository, &owner, &execution_id).await;
        assert_eq!(terminal.execution.status, "completed");
        assert_eq!(terminal.attempts.len(), 1, "steering must not duplicate the model Turn");
        assert_eq!(terminal.attempts[0].attempt.status, "completed");
        assert_eq!(terminal.attempts[0].attempt.output_summary.as_deref(), Some("completed steered-step"));
        assert_eq!(runner.call_count("steered-step"), 1);
        scheduler.shutdown().await.unwrap();
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
    async fn scheduler_human_handoff_waits_under_both_decision_policies_without_replaying_the_attempt() {
        for ask_user in [true,false] {
            let mut mode = HarnessAttemptRunner::overlap_probe();
            mode.mode = HarnessRunnerMode::UserInputHandoff;
            let runner = Arc::new(mode);
            let (mut scheduler,repository,execution_id,_dir,owner) = make_scheduler_harness(
                runner.clone(),&["introduction"],&[],1,AdaptationPolicy::Adaptive,None,
            ).await;
            let effects=Arc::new(NoopConversationEffects::default());
            Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects=effects.clone();
            let pool=runner.pool.lock().unwrap().clone().unwrap();
            if ask_user {
                sqlx::query("UPDATE agent_executions SET decision_policy='ask_user' WHERE execution_id=?")
                    .bind(&execution_id).execute(&pool).await.unwrap();
            }
            scheduler.start(owner.clone(),execution_id.clone());
            let detail=tokio::time::timeout(Duration::from_secs(5),async {
                loop {
                    let detail=repository.get_execution_detail(&owner,&execution_id).await.unwrap().unwrap();
                    if detail.execution.status=="waiting_input" { break detail; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert_eq!(detail.steps[0].status,"waiting_input");
            assert_eq!(detail.attempts.len(),1,"a normal handoff never authorizes an automatic task replay");
            let attempt=&detail.attempts[0].attempt;
            assert_eq!(attempt.status,"waiting_input");
            {
                assert_eq!(attempt.question.as_deref(),Some("请提供姓名。"));
                assert!(attempt.finished_at.is_none());
                let mut waiting=detail;
                for (index,answer) in ["小明","一年级"].into_iter().enumerate() {
                    let step=waiting.steps[0].clone();let attempt=waiting.attempts[0].attempt.clone();
                    let conversation_id=waiting.attempts[0].conversation_id.clone();
                    assert!(attempt.runtime_state.is_none(),"a proved closed handoff must retire the delivered DecisionInput");
                    let operation=generate_id();
                    let mut pending=AttemptConversationEffects::default();
                    pending.push_decision(operation.clone(),answer.into()).unwrap();
                    assert_eq!(pending.current_turn_operation_key(&attempt.attempt_id).unwrap(),operation);
                    assert!(matches!(pending.pending_conversation_effects.as_slice(),[PendingConversationEffect::DecisionInput{..}]));
                    // This is the existing answer_decision write-ahead CAS;
                    // the scheduler dispatches continue_with_input on the same Session.
                    repository.resume_waiting_attempt(&owner,&execution_id,waiting.execution.version,
                        &step.step_id,step.version,&attempt.attempt_id,attempt.version,
                        &AttemptConversationEffectParams {runtime_state:Some(pending.encode().unwrap())},
                        &system_event(AgentExecutionEventKind::DecisionAnswered,Some(&step.step_id),Some(&attempt.attempt_id),json!({})),
                    ).await.unwrap();
                    scheduler.start(owner.clone(),execution_id.clone());
                    // This asserts identity and no replay, not a five-second
                    // provider/SQLite latency contract. Leave the same bounded
                    // allowance as App handoff fixtures under parallel native
                    // builds; a stalled owner still prints its exact facts.
                    let next_waiting=tokio::time::timeout(Duration::from_secs(15),async {
                        loop {
                            let current=repository.get_execution_detail(&owner,&execution_id).await.unwrap().unwrap();
                            let answered=if index==0 { current.execution.status=="waiting_input"
                                && current.attempts[0].attempt.question.as_deref()==Some("请提供班级。") }
                                else { current.execution.status=="completed" };
                            if answered { break current; }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }).await;
                    waiting=match next_waiting {
                        Ok(detail)=>detail,
                        Err(error)=> {
                            let current=repository.get_execution_detail(&owner,&execution_id).await.unwrap().unwrap();
                            let active=scheduler.inner.active.get(&execution_id).map(|handle|
                                (handle.generation.clone(),handle.restart_requested,*handle.cancel.borrow(),handle.lease.as_ref().map(|lease|lease.owner().to_owned())));
                            panic!("handoff failed to settle ask_user={ask_user} answer={index}: {error}; execution={:?}; steps={:?}; attempts={:?}; active={active:?}; calls={:?}",
                                current.execution,current.steps,current.attempts,runner.calls.lock().unwrap());
                        }
                    };
                    assert_eq!(waiting.attempts[0].attempt.attempt_id,attempt.attempt_id);
                    assert_eq!(waiting.attempts[0].conversation_id,conversation_id);
                }
                assert_eq!(runner.calls.lock().unwrap().iter().filter(|call|call.starts_with("answer:")).count(),2);
                assert!(effects.conversation_operations.lock().unwrap().is_empty(),"a closed source Turn is never stopped or steered");
            }
            assert_eq!(runner.call_count("introduction"),1);
            scheduler.shutdown().await.unwrap();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn canonical_session_cancellation_settles_only_its_step_without_retry() {
        let mut mode = HarnessAttemptRunner::overlap_probe();
        mode.mode = HarnessRunnerMode::CanonicalCancellation;
        let runner = Arc::new(mode);
        let (scheduler, repository, execution_id, _dir, owner) = make_scheduler_harness(
            runner.clone(), &["cancel-source", "not-started"], &[("cancel-source", "not-started")], 1,
            AdaptationPolicy::Adaptive, None,
        ).await;
        scheduler.start(owner.clone(), execution_id.clone());
        let detail = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let detail = repository.get_execution_detail(&owner, &execution_id).await.unwrap().unwrap();
                if detail.execution.status == "completed_with_failures" { break detail; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert_eq!(detail.steps.iter().find(|step| step.title == "cancel-source").unwrap().status, "cancelled");
        assert_eq!(detail.steps.iter().find(|step| step.title == "not-started").unwrap().status, "skipped",
            "a missing dependency is not cancelled by task ownership");
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(detail.attempts[0].attempt.status, "cancelled");
        assert!(detail.attempts[0].attempt.error.is_none());
        assert!(detail.attempts[0].attempt.retry_after.is_none());
        assert!(detail.execution.lease_owner.is_none());
        assert_eq!(runner.call_count("cancel-source"), 1);
        assert_eq!(runner.call_count("not-started"), 0);
        let pool = runner.pool.lock().unwrap().clone().unwrap();
        let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_execution_links WHERE execution_id=? AND active=1")
            .bind(&execution_id).fetch_one(&pool).await.unwrap();
        assert_eq!(active, 0, "domain cancellation retires original links through its existing cleanup boundary");
        scheduler.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn captured_stop_does_not_stop_a_successor_scheduler_generation() {
        let runner=Arc::new(HarnessAttemptRunner::parallel());
        let (scheduler,_,execution_id,_directory,_owner)=make_scheduler_harness(
            runner,&["generation-fence"],&[],1,AdaptationPolicy::Fixed,None,
        ).await;
        let (old_cancel,_old_receiver)=watch::channel(false);
        scheduler.inner.active.insert(execution_id.clone(),ActiveHandle {
            generation:"captured-generation".into(),cancel:old_cancel,wake:Arc::new(Notify::new()),
            restart_requested:false,lease:None,
        });
        let captured=scheduler.generation(&execution_id).unwrap();
        let (successor_cancel,successor_receiver)=watch::channel(false);
        scheduler.inner.active.insert(execution_id.clone(),ActiveHandle {
            generation:"successor-generation".into(),cancel:successor_cancel,wake:Arc::new(Notify::new()),
            restart_requested:false,lease:None,
        });
        scheduler.stop_generation(&execution_id,&captured);
        assert!(!*successor_receiver.borrow(),"an earlier cancellation cannot signal the replacement scheduler");
        scheduler.stop_generation(&execution_id,"successor-generation");
        assert!(*successor_receiver.borrow());
        scheduler.inner.active.remove(&execution_id);
        scheduler.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn terminal_report_is_delivered_while_native_cleanup_remains_pending() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let (mut scheduler, repository, execution_id, _directory, owner) = make_scheduler_harness(
            runner.clone(), &["ready-result"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let effects = Arc::new(NoopConversationEffects::default());
        effects.block_cleanup.store(true, Ordering::SeqCst);
        Arc::get_mut(&mut scheduler.inner).unwrap().deps.conversation_effects = effects.clone();
        let pool = runner.pool.lock().unwrap().clone().unwrap();
        let lead = generate_id();
        sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,next_seq,created_at) VALUES(?,?,'live','interactive lead',0,0,'{}',1,1)")
            .bind(&lead).bind(json!({"principal_kind":"user","principal_id":owner}).to_string()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,created_at,updated_at) VALUES(?,?,'lead',1,1)")
            .bind(&lead).bind(&execution_id).execute(&pool).await.unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !effects.cleanup_started.load(Ordering::SeqCst) {tokio::task::yield_now().await;}
        }).await.expect("the old native owner reaches its independent cleanup");
        tokio::time::timeout(Duration::from_secs(1), async {
            while effects.report_operations.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("verified output must not wait for the two-second cleanup timeout");
        let detail = scheduler.detail(&owner,&execution_id).await.unwrap();
        assert_eq!(detail.execution.status,AgentExecutionStatus::Completed);
        let pending: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_execution_links WHERE execution_id=? AND relation='attempt' AND active=0 AND cleanup_completed_at IS NULL")
            .bind(&execution_id).fetch_one(&pool).await.unwrap();
        assert_eq!(pending,1,"result delivery does not manufacture native exit or retire its cleanup receipt");
        effects.block_cleanup.store(false,Ordering::SeqCst);
        effects.cleanup_release.notify_waiters();
        scheduler.shutdown().await.unwrap();
        assert_eq!(runner.call_count("ready-result"),1,"cleanup cannot replay completed work");
        assert_eq!(effects.report_operations.lock().unwrap().len(),1);
        assert_eq!(repository.get_execution(&owner,&execution_id).await.unwrap().unwrap().status,"completed");
    }

    #[tokio::test]
    async fn retired_directed_work_is_preserved_without_starting_an_attempt() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let (scheduler, repository, execution_id, _directory, owner) = make_scheduler_harness(
            runner.clone(), &["old-role-task"], &[], 1, AdaptationPolicy::Fixed, None,
        ).await;
        let pool = runner.pool.lock().unwrap().clone().unwrap();
        let historical = json!({"mode":"directed","source":{
            "agent_session_id":generate_id(), "turn_operation_id":"old-turn",
            "tool_operation_id":"old-tool", "effect_id":"old-effect"
        }, "plan":{"steps":[{"title":"old-role-task","spec":"retired routing"}]}}).to_string();
        sqlx::query("UPDATE agent_executions SET initial_plan_input=? WHERE execution_id=?")
            .bind(&historical).bind(&execution_id).execute(&pool).await.unwrap();
        let before = repository.get_execution(&owner, &execution_id).await.unwrap().unwrap();
        scheduler.start(owner.clone(), execution_id.clone());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.inner.active.contains_key(&execution_id) {
                tokio::task::yield_now().await;
            }
        }).await.expect("retired work stops before lease or dispatch");
        let after = repository.get_execution(&owner, &execution_id).await.unwrap().unwrap();
        assert_eq!(after.initial_plan_input, historical, "immutable source history is preserved");
        assert_eq!(after.status, before.status, "retirement does not manufacture a terminal result");
        assert_eq!(after.version, before.version, "read-only recognition does not rewrite the aggregate");
        assert_eq!(runner.call_count("old-role-task"), 0);
        assert!(scheduler.detail(&owner, &execution_id).await.unwrap().attempts.is_empty());
        scheduler.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_harness_runs_ready_roots_in_parallel_then_unblocks_downstream() {
        let runner = Arc::new(HarnessAttemptRunner::parallel());
        let downstream_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) = make_scheduler_harness_with_policies(
            runner,
            &["upstream-a", "upstream-b", "downstream"],
            &[("upstream-a", "downstream"), ("upstream-b", "downstream")],
            2,
            AdaptationPolicy::Fixed,
            None,
            &[
                ("upstream-a", AgentToolPolicy::ReadOnly),
                ("upstream-b", AgentToolPolicy::ReadOnly),
                ("downstream", AgentToolPolicy::ReadOnly),
            ],
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
        let downstream_brief = downstream_guard
            .brief_for("downstream")
            .expect("downstream brief was recorded");
        assert!(downstream_brief.contains("title=upstream-a; status=completed"));
        assert!(downstream_brief.contains("Latest output summary: completed upstream-a"));
        assert!(downstream_brief.contains("title=upstream-b; status=completed"));
        assert!(downstream_brief.contains("Latest output summary: completed upstream-b"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scheduler_serializes_workspace_mutators_but_keeps_read_only_parallelism() {
        let runner = Arc::new(HarnessAttemptRunner::overlap_probe());
        let runner_guard = runner.clone();
        let (scheduler, repository, execution_id, _data_dir, owner_id) =
            make_scheduler_harness_with_policies(
                runner,
                &["writer", "shell-verifier"],
                &[],
                2,
                AdaptationPolicy::Fixed,
                None,
                &[
                    ("writer", AgentToolPolicy::Full),
                    ("shell-verifier", AgentToolPolicy::ReadShell),
                ],
            )
            .await;

        scheduler.start(owner_id.clone(), execution_id.clone());
        let detail = wait_for_terminal(&repository, &owner_id, &execution_id).await;
        assert_eq!(detail.execution.status, "completed");
        assert_eq!(runner_guard.call_count("writer"), 1);
        assert_eq!(runner_guard.call_count("shell-verifier"), 1);
        assert_eq!(
            runner_guard.max_active(),
            1,
            "Full and ReadShell Steps share one workspace and must never overlap",
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
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let execution_version = repository
                    .get_execution(&owner_id, &execution_id)
                    .await
                    .expect("reload retry execution")
                    .expect("retry execution exists")
                    .version;
                match repository
                    .reset_steps_for_retry(
                        &owner_id,
                        &execution_id,
                        execution_version,
                        &[RetryAgentExecutionStep {
                            step_id: retry_step.0.clone(),
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
                {
                    Ok(_) => break,
                    Err(nomifun_db::DbError::Query(error))
                        if error.to_string().contains("database is locked") =>
                    {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => panic!("clear retry backoff: {error:?}"),
                }
            }
        })
        .await
        .expect("clear retry backoff before the scheduler deadline");

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
