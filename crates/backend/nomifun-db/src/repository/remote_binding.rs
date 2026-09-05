use crate::error::DbError;
use crate::models::{
    NomiRemoteEventPage, NomiRemoteEventRow, NomiRemoteSessionRow, RemoteBindingRow,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRemoteBindingParams {
    pub remote_binding_id: String,
    pub owner_user_id: String,
    pub name: String,
    pub agent_binding_json: String,
    pub nomi_snapshot_json: String,
    pub provenance_json: String,
    pub agent_binding_digest: String,
    pub binding_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRemoteBindingParams {
    pub owner_user_id: String,
    pub remote_binding_id: String,
    pub expected_binding_version: i64,
    pub expected_agent_binding_digest: String,
    pub name: String,
    pub agent_binding_json: String,
    pub nomi_snapshot_json: String,
    pub provenance_json: String,
    pub agent_binding_digest: String,
    pub binding_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetOrCreateRemoteSessionParams {
    pub owner_user_id: String,
    pub remote_binding_id: String,
    pub expected_binding_version: i64,
    pub expected_agent_binding_digest: String,
    pub open_idempotency_key: String,
    pub agent_session_id: String,
    pub initial_input_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOpenResult {
    Created(NomiRemoteSessionRow),
    Existing(NomiRemoteSessionRow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendNomiRemoteEventParams {
    pub owner_user_id: String,
    pub agent_session_id: String,
    pub event_type: String,
    pub payload_json: String,
}

/// Atomically update the durable Remote state and append the corresponding
/// projection event.
///
/// Remote state and its event cursor are two views of one fact.  Keeping this
/// input at the repository boundary prevents the application adapter from
/// committing the state in one transaction and the event in a later
/// transaction, which would leave a crash window where observers could see a
/// terminal state without the terminal event (or vice versa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionNomiRemoteSessionParams {
    pub owner_user_id: String,
    pub agent_session_id: String,
    pub expected_state: String,
    pub next_state: String,
    pub event_type: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NomiRemoteStateTransitionResult {
    pub session: NomiRemoteSessionRow,
    pub event: NomiRemoteEventRow,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendNomiRemoteEventResult {
    pub event: NomiRemoteEventRow,
    pub inserted: bool,
}

#[async_trait::async_trait]
pub trait IRemoteBindingRepository: Send + Sync {
    async fn create_binding(
        &self,
        input: CreateRemoteBindingParams,
    ) -> Result<RemoteBindingRow, DbError>;
    async fn get_binding(
        &self,
        owner_user_id: &str,
        remote_binding_id: &str,
    ) -> Result<Option<RemoteBindingRow>, DbError>;
    async fn list_bindings(&self, owner_user_id: &str) -> Result<Vec<RemoteBindingRow>, DbError>;
    async fn update_binding_cas(
        &self,
        input: UpdateRemoteBindingParams,
    ) -> Result<RemoteBindingRow, DbError>;
    async fn delete_binding_cas(
        &self,
        owner_user_id: &str,
        remote_binding_id: &str,
        expected_binding_version: i64,
        expected_agent_binding_digest: &str,
    ) -> Result<(), DbError>;
    async fn get_or_create_session(
        &self,
        input: GetOrCreateRemoteSessionParams,
    ) -> Result<RemoteOpenResult, DbError>;
    async fn get_session(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<Option<NomiRemoteSessionRow>, DbError>;
    async fn get_session_by_open_key(
        &self,
        owner_user_id: &str,
        open_idempotency_key: &str,
    ) -> Result<Option<NomiRemoteSessionRow>, DbError>;
    async fn set_session_state(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        state: &str,
    ) -> Result<NomiRemoteSessionRow, DbError>;
    async fn transition_session_state_and_append_event(
        &self,
        input: TransitionNomiRemoteSessionParams,
    ) -> Result<NomiRemoteStateTransitionResult, DbError>;
    async fn append_event(
        &self,
        input: AppendNomiRemoteEventParams,
    ) -> Result<NomiRemoteEventRow, DbError>;
    async fn append_event_once(
        &self,
        input: AppendNomiRemoteEventParams,
    ) -> Result<AppendNomiRemoteEventResult, DbError>;
    /// Find the first event for one exact operation digest and event type.
    ///
    /// The query is owner-scoped and is intentionally exposed separately from
    /// `read_events`: callers that recover from a compare-and-swap conflict
    /// must prove that the corresponding event was committed instead of
    /// treating a matching Session state as sufficient evidence.
    async fn find_event_by_operation_key(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        event_type: &str,
        operation_key_digest: &str,
    ) -> Result<Option<NomiRemoteEventRow>, DbError> {
        let page = self
            .read_events(owner_user_id, agent_session_id, 0, 1000)
            .await?;
        for event in page.events {
            if event.event_type != event_type {
                continue;
            }
            let payload: serde_json::Value = serde_json::from_str(&event.payload_json)
                .map_err(|error| DbError::Conflict(format!(
                    "persisted Remote event payload is invalid: {error}"
                )))?;
            if payload
                .get("operation_key_digest")
                .and_then(serde_json::Value::as_str)
                == Some(operation_key_digest)
            {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }
    /// Return the authoritative Remote event cursor for one Session.
    async fn current_event_cursor(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<i64, DbError> {
        Ok(self
            .read_events(owner_user_id, agent_session_id, 0, 1000)
            .await?
            .next_cursor)
    }
    async fn read_events(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<NomiRemoteEventPage, DbError>;
}
