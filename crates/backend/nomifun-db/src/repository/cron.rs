use nomifun_common::TimestampMs;

use crate::error::DbError;
use crate::models::{CronJobRow, CronRunHistoryRow, CronRunReservationRow};

pub const CRON_RUN_HISTORY_LIMIT: i64 = 7;

/// Parameters for updating a cron job.
///
/// All fields are optional; `None` means "keep the current value".
#[derive(Debug, Clone, Default)]
pub struct UpdateCronJobParams {
    /// Optional compare-and-swap precondition for schedule-bearing user
    /// mutations. This is deliberately separate from `schedule_revision`,
    /// which is the replacement value.
    pub expected_schedule_revision: Option<i64>,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub schedule_revision: Option<i64>,
    pub schedule_kind: Option<String>,
    pub schedule_value: Option<String>,
    pub schedule_tz: Option<Option<String>>,
    pub schedule_description: Option<Option<String>>,
    pub payload_message: Option<String>,
    pub execution_mode: Option<String>,
    pub agent_selection: Option<Option<String>>,
    pub model_selection: Option<Option<String>>,
    pub agent_binding: Option<Option<String>>,
    pub clear_context_each_run: Option<bool>,
    pub configuration_error: Option<Option<String>>,
    /// Target conversation. `Some(Some(id))` binds a conversation, `Some(None)`
    /// clears it to NULL, `None` leaves it unchanged.
    pub conversation_id: Option<Option<String>>,
    pub conversation_title: Option<Option<String>>,
    pub skill_content: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub next_run_at: Option<Option<TimestampMs>>,
    pub last_run_at: Option<Option<TimestampMs>>,
    pub last_status: Option<Option<String>>,
    pub last_error: Option<Option<String>>,
    pub run_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveCronRunParams {
    pub cron_job_run_id: String,
    pub cron_job_id: String,
    pub trigger_kind: String,
    pub operation_key: String,
    pub request_fingerprint: String,
    pub schedule_revision: Option<i64>,
    pub planned_at_ms: Option<TimestampMs>,
    /// Successor committed with the exact scheduled reservation. Run-now never
    /// mutates the schedule; an At occurrence has no successor.
    pub next_run_at: Option<TimestampMs>,
    pub disable: bool,
    pub now: TimestampMs,
}

/// One exact Cron run settlement plus its aggregate summary projection.
///
/// SQLite applies every field below in the same transaction that changes the
/// reservation from `reserved` to a terminal status that supplies presentation
/// history. The reservation's `pending` projection state is the exact run-id
/// CAS; replays cannot increment `run_count` twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeCronRunParams {
    pub cron_job_run_id: String,
    pub status: String,
    pub conversation_id: Option<String>,
    pub result_error: Option<String>,
    /// Host-authored preparation failure only; canonical Turn failures remain in Session facts.
    pub domain_failure_json: Option<String>,
    pub now: TimestampMs,
    /// Set `cron_jobs.last_run_at`; `None` preserves it.
    pub last_run_at: Option<TimestampMs>,
    /// Set `cron_jobs.last_status`; `None` preserves it.
    pub last_status: Option<String>,
    /// Outer `None` preserves the field; `Some(None)` clears it.
    pub last_error: Option<Option<String>>,
    pub increment_run_count: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizeCronRunOutcome {
    Applied,
    AlreadyApplied,
    /// This reservation predates the exact projection protocol. Its aggregate
    /// may already have been updated, so replay must quarantine rather than
    /// guess or double-apply.
    LegacyProjectionUnknown,
}

/// Data access abstraction for the `cron_jobs` table.
#[async_trait::async_trait]
pub trait ICronRepository: Send + Sync {
    /// Inserts a new cron job row.
    async fn insert(&self, row: &CronJobRow) -> Result<(), DbError>;

    /// Atomically inserts a Cron job and binds both sides of its canonical
    /// Session relation.
    ///
    /// Implementations must authenticate and lock either the canonical
    /// live `agent_sessions` row, then
    /// commit `cron_jobs.conversation_id` in the same transaction. Legacy
    /// targets additionally commit `conversations.cron_job_id`; canonical
    /// targets intentionally have no mutable back-reference. The bind is a
    /// CAS: an unbound Session may be claimed and a different existing Cron
    /// relation is a conflict.
    async fn insert_with_session_relation(&self, row: &CronJobRow) -> Result<(), DbError> {
        let _ = row;
        Err(DbError::Init(
            "Cron repository does not implement atomic Session relation insertion".to_owned(),
        ))
    }

    /// Updates a cron job by its stable business ID with the provided fields.
    /// Returns `DbError::NotFound` if absent.
    async fn update(
        &self,
        user_id: &str,
        cron_job_id: &str,
        params: &UpdateCronJobParams,
    ) -> Result<(), DbError>;

    /// Deletes a cron job by business ID. Returns `DbError::NotFound` if absent.
    ///
    /// A job with a non-terminal durable run reservation must return
    /// `DbError::Conflict`. Deletion must never erase the recovery authority
    /// for a turn that may already have been accepted by the Session owner.
    async fn delete(&self, user_id: &str, cron_job_id: &str) -> Result<(), DbError>;

    /// Returns a single cron job by business ID, or `None` if not found.
    async fn get_by_cron_job_id(
        &self,
        user_id: &str,
        cron_job_id: &str,
    ) -> Result<Option<CronJobRow>, DbError>;

    /// Returns all cron jobs ordered by creation time ascending.
    async fn list_all(&self, user_id: &str) -> Result<Vec<CronJobRow>, DbError>;

    /// Process-internal scheduler lookup. The returned row carries the
    /// authoritative non-empty owner; callers must preserve it through the
    /// execution path rather than supplying or deriving a fallback owner.
    async fn get_by_cron_job_id_for_scheduler(
        &self,
        cron_job_id: &str,
    ) -> Result<Option<CronJobRow>, DbError>;

    /// Process-internal scheduler scan across owners.
    async fn list_enabled_for_scheduler(&self) -> Result<Vec<CronJobRow>, DbError>;

    /// Returns all cron jobs for a given conversation.
    async fn list_by_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<CronJobRow>, DbError>;

    /// Deletes all cron jobs associated with a Conversation or canonical
    /// AgentSession and returns their exact stable job IDs for process-local
    /// timer/artifact cleanup.
    ///
    /// The operation is all-or-nothing and must return `DbError::Conflict`
    /// when any selected job owns a non-terminal durable run reservation.
    async fn delete_by_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<String>, DbError>;

    /// Returns recent execution records for one job, newest first.
    async fn list_runs_by_job(
        &self,
        user_id: &str,
        cron_job_id: &str,
        limit: i64,
    ) -> Result<Vec<CronRunHistoryRow>, DbError>;

    /// Reserve one immutable scheduler occurrence or run-now invocation before
    /// any Conversation creation, runtime build, message delivery, or event.
    /// A fresh scheduled reservation must atomically prove that the job is
    /// enabled and that both its schedule revision and exact persisted
    /// `next_run_at` still match the callback. An already-existing exact
    /// reservation remains replay-absorbing even after the job advances or is
    /// disabled. Returns the canonical reservation plus whether this call
    /// inserted it.
    async fn reserve_run(
        &self,
        user_id: &str,
        params: &ReserveCronRunParams,
    ) -> Result<(CronRunReservationRow, bool), DbError> {
        let _ = (user_id, params);
        Err(DbError::Init(
            "Cron repository does not implement durable run reservation".to_owned(),
        ))
    }

    /// Fix a lazy continuing Session and this original reservation in one
    /// transaction before accepting its first Turn. Cancellation still fences
    /// late attachment; a different existing Session is a conflict.
    async fn attach_continuing_session(
        &self,
        user_id: &str,
        cron_job_run_id: &str,
        conversation_id: &str,
        now: TimestampMs,
    ) -> Result<CronRunReservationRow, DbError>;

    /// Attach the canonical target Conversation. Replays may only present the
    /// same identity; a different target is a durable-key conflict.
    async fn attach_run_conversation(
        &self,
        user_id: &str,
        cron_job_run_id: &str,
        conversation_id: &str,
        now: TimestampMs,
    ) -> Result<CronRunReservationRow, DbError> {
        let _ = (user_id, cron_job_run_id, conversation_id, now);
        Err(DbError::Init(
            "Cron repository does not implement run-conversation attachment".to_owned(),
        ))
    }

    /// Owner-scoped read of the immutable original occurrence. Never reserve,
    /// attach, execute, or select a newer run from the job's mutable summary.
    async fn get_run_reservation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        cron_job_run_id: &str,
    ) -> Result<Option<CronRunReservationRow>, DbError> {
        let _ = (user_id, cron_job_id, cron_job_run_id);
        Err(DbError::Init(
            "Cron repository does not implement exact run lookup".into(),
        ))
    }

    /// Persist exact-run cancellation before any Session/native signal. The
    /// returned original reservation fixes its immutable source coordinates;
    /// terminal runs absorb replay without changing the job or their outcome.
    async fn request_run_cancellation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        cron_job_run_id: &str,
        now: TimestampMs,
    ) -> Result<CronRunReservationRow, DbError> {
        let _ = (user_id, cron_job_id, cron_job_run_id, now);
        Err(DbError::Init(
            "Cron repository does not implement exact run cancellation".into(),
        ))
    }

    /// Startup-only owner metadata for an already attached immutable source.
    /// Missing job configuration never grants attachment or execution.
    async fn attached_run_owner_for_scheduler(
        &self,
        cron_job_run_id: &str,
    ) -> Result<Option<String>, DbError> {
        let _ = cron_job_run_id;
        Err(DbError::Init(
            "Cron repository does not implement attached run owner lookup".into(),
        ))
    }

    /// Atomically settle one current-protocol reservation and project its job
    /// summary exactly once.
    async fn finalize_run_with_job_projection(
        &self,
        user_id: &str,
        params: &FinalizeCronRunParams,
    ) -> Result<FinalizeCronRunOutcome, DbError> {
        let _ = (user_id, params);
        Err(DbError::Init(
            "Cron repository does not implement exact run/job finalization".to_owned(),
        ))
    }

    /// Scheduler-internal boot recovery across every owner. Each returned row
    /// is crash-ambiguous and must be settled conservatively, never executed
    /// again. User-facing callers must not expose this unscoped method.
    async fn list_reserved_runs_for_scheduler(
        &self,
    ) -> Result<Vec<CronRunReservationRow>, DbError> {
        Err(DbError::Init(
            "Cron repository does not implement reserved-run recovery".to_owned(),
        ))
    }

    /// Read a scheduled occurrence without creating it. System-resume uses this
    /// to distinguish an already-reserved interrupted run (recover it) from an
    /// occurrence that was never admitted (record it as missed).
    async fn get_scheduled_run_reservation(
        &self,
        user_id: &str,
        cron_job_id: &str,
        schedule_revision: i64,
        planned_at_ms: TimestampMs,
    ) -> Result<Option<CronRunReservationRow>, DbError> {
        let _ = (user_id, cron_job_id, schedule_revision, planned_at_ms);
        Err(DbError::Init(
            "Cron repository does not implement scheduled reservation lookup".to_owned(),
        ))
    }
}
