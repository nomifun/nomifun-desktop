use nomifun_common::TimestampMs;

/// Owner-scoped Remote binding with the exact frozen AgentBindingValue JSON.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RemoteBindingRow {
    pub remote_binding_id: String,
    pub owner_user_id: String,
    pub name: String,
    pub agent_binding_json: String,
    pub nomi_snapshot_json: String,
    pub provenance_json: String,
    pub agent_binding_digest: String,
    pub binding_version: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Durable Remote projection. `agent_session_id` is also the Conversation ID.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NomiRemoteSessionRow {
    pub agent_session_id: String,
    pub owner_user_id: String,
    pub remote_binding_id: String,
    pub open_idempotency_key: String,
    pub binding_version: i64,
    pub agent_binding_digest: String,
    pub initial_input_digest: Option<String>,
    pub agent_binding_json: String,
    pub nomi_snapshot_json: String,
    pub provenance_json: String,
    pub state: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NomiRemoteEventRow {
    pub event_id: String,
    pub agent_session_id: String,
    pub seq: i64,
    pub event_type: String,
    pub payload_json: String,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NomiRemoteEventPage {
    pub events: Vec<NomiRemoteEventRow>,
    pub next_cursor: i64,
}
