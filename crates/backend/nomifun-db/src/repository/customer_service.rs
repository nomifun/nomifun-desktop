use nomifun_common::TimestampMs;
use nomifun_common::text_search::NoteQueryTerms;

use crate::error::DbError;
use crate::models::{
    CsAgentRow, CsAuditEventRow, CsChannelBindingRow, CsDialogueRow, CsMessageRow, CsNoteRow,
    NewCsAgentRow,
};
use crate::repository::customer_service_search::CsNoteSearchHit;

/// Identity triple that pins a visitor dialogue lane (一人一线).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsDialogueKey {
    pub channel_plugin_id: String,
    pub channel_user_id: String,
    pub chat_id: String,
}

/// Mutable columns accepted when updating a `cs_agents` row. `None` keeps the
/// stored value.
#[derive(Debug, Clone, Default)]
pub struct UpdateCsAgentParams {
    pub name: Option<String>,
    pub greeting: Option<String>,
    pub persona: Option<String>,
    pub service_policy: Option<String>,
    /// `Some(None)` clears the provider binding; `None` keeps it.
    pub provider_id: Option<Option<String>>,
    pub model: Option<Option<String>>,
    /// JSON array string (`CsAgentRow::encode_knowledge_base_ids`).
    pub knowledge_base_ids: Option<String>,
    pub enabled: Option<bool>,
    pub max_concurrent: Option<i64>,
    pub audit_retention_days: Option<i64>,
}

/// Data access abstraction for the customer-service (`cs_`) tables.
///
/// Object-safe via `async_trait` to support `Arc<dyn ICustomerServiceRepository>`.
#[async_trait::async_trait]
pub trait ICustomerServiceRepository: Send + Sync {
    // ── cs_agents CRUD ───────────────────────────────────────────────

    /// Insert a new customer-service agent and return the persisted row.
    async fn create_agent(&self, row: &NewCsAgentRow) -> Result<CsAgentRow, DbError>;

    /// Return one agent by business ID, or `None`.
    async fn get_agent(&self, cs_agent_id: &str) -> Result<Option<CsAgentRow>, DbError>;

    /// Return all agents ordered by creation time descending.
    async fn list_agents(&self) -> Result<Vec<CsAgentRow>, DbError>;

    /// Patch the mutable columns of an agent. Returns the updated row.
    /// `DbError::NotFound` if absent.
    async fn update_agent(
        &self,
        cs_agent_id: &str,
        params: &UpdateCsAgentParams,
        now: TimestampMs,
    ) -> Result<CsAgentRow, DbError>;

    /// Delete an agent and cascade its bindings, dialogues (with messages) and
    /// private notes in one transaction. Shared notes (`cs_agent_id IS NULL`)
    /// and audit events are retained. `DbError::NotFound` if absent.
    async fn delete_agent(&self, cs_agent_id: &str) -> Result<(), DbError>;

    // ── cs_channel_bindings ──────────────────────────────────────────

    /// Replace the full binding set of one agent (PUT semantics): every listed
    /// plugin ends up bound to `cs_agent_id` (rebinding steals a plugin from
    /// any other agent), and bindings of this agent not listed are removed.
    async fn replace_agent_bindings(
        &self,
        cs_agent_id: &str,
        channel_plugin_ids: &[String],
        now: TimestampMs,
    ) -> Result<Vec<CsChannelBindingRow>, DbError>;

    /// Bindings of one agent, newest first.
    async fn list_agent_bindings(
        &self,
        cs_agent_id: &str,
    ) -> Result<Vec<CsChannelBindingRow>, DbError>;

    /// The binding owning `channel_plugin_id`, or `None` (a bot serves at most
    /// one agent).
    async fn binding_for_plugin(
        &self,
        channel_plugin_id: &str,
    ) -> Result<Option<CsChannelBindingRow>, DbError>;

    // ── cs_dialogues / cs_messages ───────────────────────────────────

    /// Fetch or create the dialogue lane for an identity triple. On reuse the
    /// row's `last_activity` (and `cs_agent_id`, if the bot was rebound) is
    /// refreshed.
    async fn get_or_create_dialogue(
        &self,
        cs_agent_id: &str,
        key: &CsDialogueKey,
        now: TimestampMs,
    ) -> Result<CsDialogueRow, DbError>;

    /// Return one dialogue by business ID, or `None`.
    async fn get_dialogue(&self, cs_dialogue_id: &str) -> Result<Option<CsDialogueRow>, DbError>;

    /// Dialogues of one agent ordered by last activity descending.
    async fn list_dialogues(&self, cs_agent_id: &str) -> Result<Vec<CsDialogueRow>, DbError>;

    /// Append one transcript message and bump the dialogue's `last_activity`.
    async fn append_message(
        &self,
        cs_dialogue_id: &str,
        role: &str,
        content: &str,
        now: TimestampMs,
    ) -> Result<CsMessageRow, DbError>;

    /// The most recent messages of a dialogue in CHRONOLOGICAL order, capped
    /// at `limit` rows and (approximately) `char_budget` total content chars.
    /// The newest messages win when the budget truncates.
    async fn recent_messages(
        &self,
        cs_dialogue_id: &str,
        limit: usize,
        char_budget: usize,
    ) -> Result<Vec<CsMessageRow>, DbError>;

    /// Full transcript of a dialogue in chronological order.
    async fn list_messages(&self, cs_dialogue_id: &str) -> Result<Vec<CsMessageRow>, DbError>;

    // ── cs_notes CRUD ────────────────────────────────────────────────

    /// Insert a note (private when `cs_agent_id` is set, shared when `None`).
    async fn create_note(&self, row: &CsNoteRow) -> Result<CsNoteRow, DbError>;

    /// Notes visible to one agent: its private notes plus every shared note.
    /// `None` lists ALL notes (management surface).
    async fn list_notes(&self, cs_agent_id: Option<&str>) -> Result<Vec<CsNoteRow>, DbError>;

    /// Ranked hybrid search over the enabled notes visible to one agent.
    ///
    /// `terms` come from [`nomifun_common::text_search::expand_query`], which
    /// normalizes and splits the caller's natural-language query. Passing
    /// pre-expanded terms rather than a raw string is deliberate: the previous
    /// signature took a `&str` and matched it as one contiguous `LIKE` pattern,
    /// so a model-generated query with an extra space missed notes that
    /// existed. The type now makes the expansion step unskippable.
    ///
    /// Empty `terms` yield no hits — a query with no signal must match nothing,
    /// never everything.
    async fn search_notes(
        &self,
        cs_agent_id: &str,
        terms: &NoteQueryTerms,
        limit: usize,
    ) -> Result<Vec<CsNoteSearchHit>, DbError>;

    /// One-line topic labels for the notes visible to one agent, newest first.
    ///
    /// Backs the "nothing matched, but here is what exists" reply, so a model
    /// that guessed the wrong keywords can see the available subjects and
    /// re-query instead of telling the visitor there is no answer.
    async fn note_topics(&self, cs_agent_id: &str, limit: usize) -> Result<Vec<String>, DbError>;

    /// Patch `kind`/`content`/`aliases`/`enabled` of a note. `DbError::NotFound`
    /// if absent. Keeps the full-text index in step with the row.
    async fn update_note(
        &self,
        cs_note_id: &str,
        kind: Option<&str>,
        content: Option<&str>,
        aliases: Option<&str>,
        enabled: Option<bool>,
        now: TimestampMs,
    ) -> Result<CsNoteRow, DbError>;

    /// Delete a note by business ID. `DbError::NotFound` if absent.
    async fn delete_note(&self, cs_note_id: &str) -> Result<(), DbError>;

    // ── cs_audit_events ──────────────────────────────────────────────

    /// Append one audit event.
    async fn insert_audit_event(&self, row: &CsAuditEventRow) -> Result<(), DbError>;

    /// Audit events of one agent, newest first, capped at `limit`.
    async fn list_audit_events(
        &self,
        cs_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<CsAuditEventRow>, DbError>;

    /// Prune audit events older than each agent's `audit_retention_days`.
    /// Returns the number of deleted rows.
    async fn cleanup_audit_events(&self, now: TimestampMs) -> Result<u64, DbError>;
}
