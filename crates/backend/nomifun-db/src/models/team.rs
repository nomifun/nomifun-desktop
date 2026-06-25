use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `teams` table.
///
/// The former `agents` JSON array is columnized into the `team_agents` table
/// (see [`TeamAgentRow`]). `lead_agent_id` is an agent-address (a slot_id, or
/// the `lead`/`user` sentinel), NOT a foreign key.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TeamRow {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub workspace: String,
    pub workspace_mode: String,
    pub lead_agent_id: Option<String>,
    pub session_mode: Option<String>,
    pub agents_version: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `team_agents` table (was `teams.agents` JSON array).
///
/// `slot_id` stays a string PK because it is transmitted in the MCP env
/// (`TEAM_AGENT_SLOT_ID`) and the remote protocol. `conversation_id` is a real
/// FK (CASCADE); `custom_agent_id` is a soft reference (no FK).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TeamAgentRow {
    pub slot_id: String,
    pub team_id: String,
    pub name: String,
    pub role: String,
    pub conversation_id: Option<i64>,
    pub backend: String,
    pub model: String,
    pub custom_agent_id: Option<String>,
    pub status: Option<String>,
    pub conversation_type: Option<String>,
    pub cli_path: Option<String>,
    pub sort_order: i64,
}

/// Row mapping for the `mailbox` table.
///
/// Represents an inter-agent message within a team. `to_agent_id` /
/// `from_agent_id` are agent-addresses (slot_id or `user`/`lead` sentinel),
/// NOT foreign keys.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MailboxMessageRow {
    pub id: i64,
    pub team_id: String,
    pub to_agent_id: String,
    pub from_agent_id: String,
    /// Message type: 'message', 'idle_notification', or 'shutdown_request'.
    #[sqlx(rename = "type")]
    pub msg_type: String,
    pub content: String,
    pub summary: Option<String>,
    /// JSON-serialized file paths attached to the message.
    pub files: Option<String>,
    pub read: bool,
    pub created_at: TimestampMs,
}

/// Row mapping for the `team_tasks` table.
///
/// The former bidirectional `blocked_by` / `blocks` JSON arrays are columnized
/// into the single-directed [`TeamTaskDepRow`] edge table. `owner` is an
/// agent-address (slot_id or sentinel), NOT a foreign key.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TeamTaskRow {
    pub id: String,
    pub team_id: String,
    pub subject: String,
    pub description: Option<String>,
    /// Task status: 'pending', 'in_progress', 'completed', or 'deleted'.
    pub status: String,
    pub owner: Option<String>,
    /// JSON object: arbitrary extension metadata.
    pub metadata: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row in the `team_task_deps` edge table (was `team_tasks.blocked_by` /
/// `blocks` JSON arrays). A row means `blocker_task_id` blocks
/// `blocked_task_id`. "who blocks X" = WHERE blocked_task_id=X; "what X blocks"
/// = WHERE blocker_task_id=X.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TeamTaskDepRow {
    pub blocker_task_id: String,
    pub blocked_task_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_agent_row_roundtrip() {
        let row = TeamAgentRow {
            slot_id: "slot_1".into(),
            team_id: "team_1".into(),
            name: "Builder".into(),
            role: "teammate".into(),
            conversation_id: Some(1),
            backend: "claude".into(),
            model: String::new(),
            custom_agent_id: None,
            status: Some("idle".into()),
            conversation_type: Some("acp".into()),
            cli_path: None,
            sort_order: 0,
        };
        let back: TeamAgentRow = serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
        assert_eq!(back.slot_id, "slot_1");
        assert_eq!(back.conversation_id, Some(1));
    }

    #[test]
    fn mailbox_row_msg_type_field_maps_correctly() {
        let row = MailboxMessageRow {
            id: 1,
            team_id: "team_1".into(),
            to_agent_id: "slot_1".into(),
            from_agent_id: "user".into(),
            msg_type: "message".into(),
            content: "hello".into(),
            summary: None,
            files: None,
            read: false,
            created_at: 0,
        };
        assert_eq!(row.msg_type, "message");
        assert_eq!(row.from_agent_id, "user");
    }

    #[test]
    fn team_task_dep_row_roundtrip() {
        let dep = TeamTaskDepRow {
            blocker_task_id: "task_a".into(),
            blocked_task_id: "task_b".into(),
        };
        let back: TeamTaskDepRow = serde_json::from_str(&serde_json::to_string(&dep).unwrap()).unwrap();
        assert_eq!(back.blocker_task_id, "task_a");
        assert_eq!(back.blocked_task_id, "task_b");
    }

    #[test]
    fn team_task_row_serialization_roundtrip() {
        let row = TeamTaskRow {
            id: "task_1".into(),
            team_id: "team_1".into(),
            subject: "Implement feature".into(),
            description: Some("Details".into()),
            status: "in_progress".into(),
            owner: Some("slot_1".into()),
            metadata: Some(r#"{"priority":"high"}"#.into()),
            created_at: 1000,
            updated_at: 2000,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let restored: TeamTaskRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.id, row.id);
        assert_eq!(restored.status, row.status);
        assert_eq!(restored.owner.as_deref(), Some("slot_1"));
    }
}
