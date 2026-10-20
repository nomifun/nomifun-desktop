use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CronJobRow {
    pub id: i64,
    pub cron_job_id: String,
    /// Immutable aggregate owner. Runtime code must never infer or default it
    /// from an optional Conversation.
    pub user_id: String,
    pub name: String,
    pub enabled: bool,
    /// Monotonic identity of the installed schedule. Timer callbacks carry
    /// this value so a late callback from an older schedule can never reserve
    /// or execute an occurrence belonging to the replacement schedule.
    pub schedule_revision: i64,
    pub schedule_kind: String,
    pub schedule_value: String,
    pub schedule_tz: Option<String>,
    pub schedule_description: Option<String>,
    pub payload_message: String,
    pub execution_mode: String,
    /// Authoring intent only. Scheduled execution consumes `agent_binding`.
    pub agent_selection: Option<String>,
    pub model_selection: Option<String>,
    /// One standard canonical AgentBindingValue; immutable artifacts live in
    /// the existing Agent Revision/Snapshot stores.
    pub agent_binding: Option<String>,
    pub clear_context_each_run: bool,
    pub configuration_error: Option<String>,
    /// The existing-mode continuing Session, fixed after its first opening.
    /// New-conversation jobs keep only occurrence-level Session references.
    /// This is a business-ID logical reference.
    pub conversation_id: Option<String>,
    pub conversation_title: Option<String>,
    pub created_by: String,
    pub skill_content: Option<String>,
    pub description: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub next_run_at: Option<TimestampMs>,
    pub last_run_at: Option<TimestampMs>,
    /// Exact source of the mutable summary, never inferred from history order.
    pub last_run_id: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub run_count: i64,
}
