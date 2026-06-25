use std::fmt;

use nomifun_api_types::{TeamAgentResponse, TeamResponse};
use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TeammateRole
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeammateRole {
    #[serde(alias = "leader")]
    Lead,
    Teammate,
}

impl fmt::Display for TeammateRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lead => write!(f, "lead"),
            Self::Teammate => write!(f, "teammate"),
        }
    }
}

impl TeammateRole {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "lead" | "leader" => Some(Self::Lead),
            "teammate" => Some(Self::Teammate),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TeammateStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeammateStatus {
    #[serde(alias = "pending")]
    Idle,
    #[serde(alias = "active")]
    Working,
    Thinking,
    ToolUse,
    #[serde(alias = "completed")]
    Completed,
    #[serde(alias = "failed")]
    Error,
}

impl fmt::Display for TeammateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Working => write!(f, "working"),
            Self::Thinking => write!(f, "thinking"),
            Self::ToolUse => write!(f, "tool_use"),
            Self::Completed => write!(f, "completed"),
            Self::Error => write!(f, "error"),
        }
    }
}

impl TeammateStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" | "pending" => Some(Self::Idle),
            "working" | "active" => Some(Self::Working),
            "thinking" => Some(Self::Thinking),
            "tool_use" => Some(Self::ToolUse),
            "completed" => Some(Self::Completed),
            "error" | "failed" => Some(Self::Error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TeamAgent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamAgent {
    #[serde(default, alias = "slotId")]
    pub slot_id: String,
    #[serde(alias = "agentName")]
    pub name: String,
    pub role: TeammateRole,
    #[serde(alias = "conversationId")]
    pub conversation_id: String,
    #[serde(alias = "agentType")]
    pub backend: String,
    #[serde(default)]
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none", alias = "customAgentId")]
    pub custom_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TeammateStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "conversationType")]
    pub conversation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "cliPath")]
    pub cli_path: Option<String>,
}

impl TeamAgent {
    pub fn to_response(&self) -> TeamAgentResponse {
        self.to_response_with_icon(None)
    }

    pub fn to_response_with_icon(&self, icon: Option<String>) -> TeamAgentResponse {
        TeamAgentResponse {
            slot_id: self.slot_id.clone(),
            name: self.name.clone(),
            role: self.role.to_string(),
            // DTO `conversation_id` is i64 (Option A keeps the domain field a
            // String); parse, defaulting to 0 when absent/unset.
            conversation_id: self.conversation_id.parse::<i64>().unwrap_or(0),
            backend: self.backend.clone(),
            icon,
            model: self.model.clone(),
            custom_agent_id: self.custom_agent_id.clone(),
            status: self.status.map(|s| s.to_string()),
            pending_confirmations: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Team
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub agents: Vec<TeamAgent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_agent_id: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

// ---------------------------------------------------------------------------
// MailboxMessageType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxMessageType {
    Message,
    IdleNotification,
    ShutdownRequest,
}

impl fmt::Display for MailboxMessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message => write!(f, "message"),
            Self::IdleNotification => write!(f, "idle_notification"),
            Self::ShutdownRequest => write!(f, "shutdown_request"),
        }
    }
}

impl MailboxMessageType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "message" => Some(Self::Message),
            "idle_notification" => Some(Self::IdleNotification),
            "shutdown_request" => Some(Self::ShutdownRequest),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// MailboxMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailboxMessage {
    pub id: i64,
    pub team_id: String,
    pub to_agent_id: String,
    pub from_agent_id: String,
    #[serde(rename = "type")]
    pub msg_type: MailboxMessageType,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    pub read: bool,
    pub created_at: TimestampMs,
}

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

impl TaskStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TeamTask
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamTask {
    pub id: String,
    pub team_id: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    pub blocks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

// ---------------------------------------------------------------------------
// Conversion helpers: DB rows ↔ domain types
// ---------------------------------------------------------------------------

use nomifun_db::models::{MailboxMessageRow, TeamAgentRow, TeamRow, TeamTaskRow};

impl TeamAgent {
    /// Maps a persisted [`TeamAgentRow`] (was an element of the `teams.agents`
    /// JSON array) into the in-memory domain type.
    ///
    /// `role`/`status`/`conversation_type` fall back to sensible defaults when
    /// the stored string is unrecognized (matching the lenient JSON parsing
    /// behaviour the array form used to have). `conversation_id` is stored as a
    /// nullable FK column but the domain type keeps it as a plain `String`
    /// (empty when absent — every active slot has a conversation).
    pub fn from_row(row: &TeamAgentRow) -> Self {
        Self {
            slot_id: row.slot_id.clone(),
            name: row.name.clone(),
            role: TeammateRole::parse(&row.role).unwrap_or(TeammateRole::Teammate),
            conversation_id: row.conversation_id.map(|id| id.to_string()).unwrap_or_default(),
            backend: row.backend.clone(),
            model: row.model.clone(),
            custom_agent_id: row.custom_agent_id.clone(),
            status: row.status.as_deref().and_then(TeammateStatus::parse),
            conversation_type: row.conversation_type.clone(),
            cli_path: row.cli_path.clone(),
        }
    }

    /// Builds a [`TeamAgentRow`] for persistence. `team_id` and `sort_order`
    /// are supplied by the caller (the agent's position in the roster array).
    pub fn to_row(&self, team_id: &str, sort_order: i64) -> TeamAgentRow {
        TeamAgentRow {
            slot_id: self.slot_id.clone(),
            team_id: team_id.to_owned(),
            name: self.name.clone(),
            role: self.role.to_string(),
            // Domain keeps conversation_id as String (Option A); the FK column
            // is now Option<i64>. Empty or unparseable → NULL (no conversation).
            conversation_id: self.conversation_id.parse::<i64>().ok(),
            backend: self.backend.clone(),
            model: self.model.clone(),
            custom_agent_id: self.custom_agent_id.clone(),
            status: self.status.map(|s| s.to_string()),
            conversation_type: self.conversation_type.clone(),
            cli_path: self.cli_path.clone(),
            sort_order,
        }
    }
}

impl Team {
    /// Assembles the `Team` aggregate from its `teams` row plus the agent
    /// slots loaded separately from `team_agents` (the former `agents` JSON
    /// array). Callers fetch `row` via `get_team`/`list_teams` and `agent_rows`
    /// via `list_team_agents(team_id)`.
    pub fn from_parts(row: &TeamRow, agent_rows: &[TeamAgentRow]) -> Self {
        Self {
            id: row.id.clone(),
            name: row.name.clone(),
            agents: agent_rows.iter().map(TeamAgent::from_row).collect(),
            lead_agent_id: row.lead_agent_id.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    pub fn to_response(&self) -> TeamResponse {
        TeamResponse {
            id: self.id.clone(),
            name: self.name.clone(),
            agents: self.agents.iter().map(|a| a.to_response()).collect(),
            lead_agent_id: self.lead_agent_id.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl MailboxMessage {
    pub fn from_row(row: &MailboxMessageRow) -> Option<Self> {
        let msg_type = MailboxMessageType::parse(&row.msg_type)?;
        let files = row
            .files
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .filter(|v| !v.is_empty());
        Some(Self {
            id: row.id,
            team_id: row.team_id.clone(),
            to_agent_id: row.to_agent_id.clone(),
            from_agent_id: row.from_agent_id.clone(),
            msg_type,
            content: row.content.clone(),
            summary: row.summary.clone(),
            files,
            read: row.read,
            created_at: row.created_at,
        })
    }
}

impl TeamTask {
    /// Assembles the `TeamTask` aggregate from its `team_tasks` row plus the
    /// dependency lists loaded separately from `team_task_deps` (the former
    /// `blocked_by` / `blocks` JSON arrays). `blocked_by` = tasks that block
    /// this one (`list_blockers`); `blocks` = tasks this one blocks
    /// (`list_blocking`).
    pub fn from_parts(
        row: &TeamTaskRow,
        blocked_by: Vec<String>,
        blocks: Vec<String>,
    ) -> Result<Self, serde_json::Error> {
        let status = TaskStatus::parse(&row.status).unwrap_or(TaskStatus::Pending);
        let metadata: Option<serde_json::Value> = row.metadata.as_deref().map(serde_json::from_str).transpose()?;
        Ok(Self {
            id: row.id.clone(),
            team_id: row.team_id.clone(),
            subject: row.subject.clone(),
            description: row.description.clone(),
            status,
            owner: row.owner.clone(),
            blocked_by,
            blocks,
            metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- TeammateRole ---------------------------------------------------------

    #[test]
    fn teammate_role_display() {
        assert_eq!(TeammateRole::Lead.to_string(), "lead");
        assert_eq!(TeammateRole::Teammate.to_string(), "teammate");
    }

    #[test]
    fn teammate_role_parse() {
        assert_eq!(TeammateRole::parse("lead"), Some(TeammateRole::Lead));
        assert_eq!(TeammateRole::parse("teammate"), Some(TeammateRole::Teammate));
        assert_eq!(TeammateRole::parse("unknown"), None);
    }

    #[test]
    fn teammate_role_serde_roundtrip() {
        let role = TeammateRole::Lead;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, r#""lead""#);
        let parsed: TeammateRole = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, role);
    }

    // -- TeammateStatus -------------------------------------------------------

    #[test]
    fn teammate_status_display() {
        assert_eq!(TeammateStatus::Idle.to_string(), "idle");
        assert_eq!(TeammateStatus::Working.to_string(), "working");
        assert_eq!(TeammateStatus::Thinking.to_string(), "thinking");
        assert_eq!(TeammateStatus::ToolUse.to_string(), "tool_use");
        assert_eq!(TeammateStatus::Completed.to_string(), "completed");
        assert_eq!(TeammateStatus::Error.to_string(), "error");
    }

    #[test]
    fn teammate_status_parse_all_variants() {
        assert_eq!(TeammateStatus::parse("idle"), Some(TeammateStatus::Idle));
        assert_eq!(TeammateStatus::parse("working"), Some(TeammateStatus::Working));
        assert_eq!(TeammateStatus::parse("thinking"), Some(TeammateStatus::Thinking));
        assert_eq!(TeammateStatus::parse("tool_use"), Some(TeammateStatus::ToolUse));
        assert_eq!(TeammateStatus::parse("completed"), Some(TeammateStatus::Completed));
        assert_eq!(TeammateStatus::parse("error"), Some(TeammateStatus::Error));
        assert_eq!(TeammateStatus::parse("bad"), None);
    }

    #[test]
    fn teammate_status_parse_nomifun_aliases() {
        assert_eq!(TeammateStatus::parse("pending"), Some(TeammateStatus::Idle));
        assert_eq!(TeammateStatus::parse("active"), Some(TeammateStatus::Working));
        assert_eq!(TeammateStatus::parse("failed"), Some(TeammateStatus::Error));
    }

    #[test]
    fn teammate_status_serde_roundtrip() {
        for status in [
            TeammateStatus::Idle,
            TeammateStatus::Working,
            TeammateStatus::Thinking,
            TeammateStatus::ToolUse,
            TeammateStatus::Completed,
            TeammateStatus::Error,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: TeammateStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn teammate_status_serde_nomifun_aliases() {
        let pending: TeammateStatus = serde_json::from_str(r#""pending""#).unwrap();
        assert_eq!(pending, TeammateStatus::Idle);
        let active: TeammateStatus = serde_json::from_str(r#""active""#).unwrap();
        assert_eq!(active, TeammateStatus::Working);
        let completed: TeammateStatus = serde_json::from_str(r#""completed""#).unwrap();
        assert_eq!(completed, TeammateStatus::Completed);
        let failed: TeammateStatus = serde_json::from_str(r#""failed""#).unwrap();
        assert_eq!(failed, TeammateStatus::Error);
    }

    #[test]
    fn teammate_role_serde_leader_alias() {
        let leader: TeammateRole = serde_json::from_str(r#""leader""#).unwrap();
        assert_eq!(leader, TeammateRole::Lead);
    }

    // -- MailboxMessageType ---------------------------------------------------

    #[test]
    fn mailbox_message_type_display() {
        assert_eq!(MailboxMessageType::Message.to_string(), "message");
        assert_eq!(MailboxMessageType::IdleNotification.to_string(), "idle_notification");
        assert_eq!(MailboxMessageType::ShutdownRequest.to_string(), "shutdown_request");
    }

    #[test]
    fn mailbox_message_type_parse() {
        assert_eq!(MailboxMessageType::parse("message"), Some(MailboxMessageType::Message));
        assert_eq!(
            MailboxMessageType::parse("idle_notification"),
            Some(MailboxMessageType::IdleNotification)
        );
        assert_eq!(
            MailboxMessageType::parse("shutdown_request"),
            Some(MailboxMessageType::ShutdownRequest)
        );
        assert_eq!(MailboxMessageType::parse("unknown"), None);
    }

    #[test]
    fn mailbox_message_type_serde_roundtrip() {
        for mt in [
            MailboxMessageType::Message,
            MailboxMessageType::IdleNotification,
            MailboxMessageType::ShutdownRequest,
        ] {
            let json = serde_json::to_string(&mt).unwrap();
            let parsed: MailboxMessageType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, mt);
        }
    }

    // -- TaskStatus -----------------------------------------------------------

    #[test]
    fn task_status_display() {
        assert_eq!(TaskStatus::Pending.to_string(), "pending");
        assert_eq!(TaskStatus::InProgress.to_string(), "in_progress");
        assert_eq!(TaskStatus::Completed.to_string(), "completed");
        assert_eq!(TaskStatus::Deleted.to_string(), "deleted");
    }

    #[test]
    fn task_status_parse_all_variants() {
        assert_eq!(TaskStatus::parse("pending"), Some(TaskStatus::Pending));
        assert_eq!(TaskStatus::parse("in_progress"), Some(TaskStatus::InProgress));
        assert_eq!(TaskStatus::parse("completed"), Some(TaskStatus::Completed));
        assert_eq!(TaskStatus::parse("deleted"), Some(TaskStatus::Deleted));
        assert_eq!(TaskStatus::parse("bad"), None);
    }

    #[test]
    fn task_status_serde_roundtrip() {
        for status in [
            TaskStatus::Pending,
            TaskStatus::InProgress,
            TaskStatus::Completed,
            TaskStatus::Deleted,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    // -- TeamAgent conversion -------------------------------------------------

    #[test]
    fn team_agent_to_response() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "Lead".into(),
            role: TeammateRole::Lead,
            conversation_id: "c1".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: Some("custom-1".into()),
            status: Some(TeammateStatus::Working),
            conversation_type: None,
            cli_path: None,
        };
        let resp = agent.to_response();
        assert_eq!(resp.slot_id, "s1");
        assert_eq!(resp.role, "lead");
        assert!(resp.icon.is_none());
        assert_eq!(resp.status.as_deref(), Some("working"));
        assert_eq!(resp.custom_agent_id.as_deref(), Some("custom-1"));
    }

    #[test]
    fn team_agent_to_response_with_icon() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "Lead".into(),
            role: TeammateRole::Lead,
            conversation_id: "c1".into(),
            backend: "claude".into(),
            model: "opus".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        };

        let resp = agent.to_response_with_icon(Some("/api/assets/logos/ai-major/claude.svg".into()));
        assert_eq!(resp.icon.as_deref(), Some("/api/assets/logos/ai-major/claude.svg"));
        assert_eq!(resp.backend, "claude");
    }

    #[test]
    fn team_agent_serde_roundtrip() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "Worker".into(),
            role: TeammateRole::Teammate,
            conversation_id: "c1".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        };
        let json = serde_json::to_string(&agent).unwrap();
        let parsed: TeamAgent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, agent);
    }

    #[test]
    fn team_agent_snake_case_serialization() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "A".into(),
            role: TeammateRole::Lead,
            conversation_id: "c1".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: Some("x".into()),
            status: Some(TeammateStatus::Idle),
            conversation_type: None,
            cli_path: None,
        };
        let val = serde_json::to_value(&agent).unwrap();
        assert!(val.get("slot_id").is_some());
        assert!(val.get("conversation_id").is_some());
        assert!(val.get("custom_agent_id").is_some());
    }

    #[test]
    fn team_agent_deserialize_nomifun_format() {
        let raw = serde_json::json!({
            "slot_id": "slot-abc",
            "conversation_id": "conv-1",
            "role": "leader",
            "agentType": "claude",
            "agentName": "Leader",
            "conversation_type": "acp",
            "status": "active",
            "custom_agent_id": "custom-1"
        });
        let agent: TeamAgent = serde_json::from_value(raw).unwrap();
        assert_eq!(agent.name, "Leader");
        assert_eq!(agent.backend, "claude");
        assert_eq!(agent.role, TeammateRole::Lead);
        assert_eq!(agent.status, Some(TeammateStatus::Working));
        assert_eq!(agent.conversation_type.as_deref(), Some("acp"));
    }

    // -- Team from_parts ------------------------------------------------------

    #[test]
    fn team_from_parts_success() {
        use nomifun_db::models::TeamAgentRow;
        let row = TeamRow {
            id: "t1".into(),
            user_id: "system_default_user".into(),
            name: "Alpha".into(),
            workspace: String::new(),
            workspace_mode: "shared".into(),
            lead_agent_id: Some("s1".into()),
            session_mode: None,
            agents_version: "1.0.1".into(),
            created_at: 1000,
            updated_at: 2000,
        };
        let agent_rows = vec![TeamAgentRow {
            slot_id: "s1".into(),
            team_id: "t1".into(),
            name: "Lead".into(),
            role: "lead".into(),
            conversation_id: Some(1),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
            sort_order: 0,
        }];
        let team = Team::from_parts(&row, &agent_rows);
        assert_eq!(team.id, "t1");
        assert_eq!(team.agents.len(), 1);
        assert_eq!(team.agents[0].slot_id, "s1");
        assert_eq!(team.agents[0].conversation_id, "1");
        assert_eq!(team.lead_agent_id.as_deref(), Some("s1"));
    }

    #[test]
    fn team_agent_row_roundtrip_via_domain() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "Lead".into(),
            role: TeammateRole::Lead,
            conversation_id: "1".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: Some("custom-1".into()),
            status: Some(TeammateStatus::Working),
            conversation_type: Some("acp".into()),
            cli_path: None,
        };
        let row = agent.to_row("t1", 3);
        assert_eq!(row.team_id, "t1");
        assert_eq!(row.sort_order, 3);
        assert_eq!(row.role, "lead");
        assert_eq!(row.status.as_deref(), Some("working"));
        let back = TeamAgent::from_row(&row);
        assert_eq!(back, agent);
    }

    #[test]
    fn team_agent_to_row_empty_conversation_id_is_null() {
        let agent = TeamAgent {
            slot_id: "s1".into(),
            name: "Lead".into(),
            role: TeammateRole::Lead,
            conversation_id: String::new(),
            backend: "acp".into(),
            model: String::new(),
            custom_agent_id: None,
            status: None,
            conversation_type: None,
            cli_path: None,
        };
        let row = agent.to_row("t1", 0);
        assert!(row.conversation_id.is_none());
        let back = TeamAgent::from_row(&row);
        assert_eq!(back.conversation_id, "");
    }

    #[test]
    fn team_to_response() {
        let team = Team {
            id: "t1".into(),
            name: "Alpha".into(),
            agents: vec![TeamAgent {
                slot_id: "s1".into(),
                name: "Lead".into(),
                role: TeammateRole::Lead,
                conversation_id: "c1".into(),
                backend: "acp".into(),
                model: "claude".into(),
                custom_agent_id: None,
                status: Some(TeammateStatus::Idle),
                conversation_type: None,
                cli_path: None,
            }],
            lead_agent_id: Some("s1".into()),
            created_at: 1000,
            updated_at: 2000,
        };
        let resp = team.to_response();
        assert_eq!(resp.id, "t1");
        assert_eq!(resp.name, "Alpha");
        assert_eq!(resp.agents.len(), 1);
        assert_eq!(resp.agents[0].slot_id, "s1");
        assert_eq!(resp.lead_agent_id.as_deref(), Some("s1"));
        assert_eq!(resp.created_at, 1000);
        assert_eq!(resp.updated_at, 2000);
    }

    #[test]
    fn team_agent_deserialize_old_camelcase_format() {
        let raw = serde_json::json!({
            "slotId": "slot-abc",
            "conversationId": "conv-123",
            "role": "leader",
            "status": "pending",
            "agentType": "claude",
            "agentName": "Leader",
            "conversationType": "acp",
            "cliPath": "claude"
        });
        let agent: TeamAgent = serde_json::from_value(raw).unwrap();
        assert_eq!(agent.slot_id, "slot-abc");
        assert_eq!(agent.conversation_id, "conv-123");
        assert_eq!(agent.name, "Leader");
        assert_eq!(agent.backend, "claude");
        assert_eq!(agent.conversation_type.as_deref(), Some("acp"));
        assert_eq!(agent.cli_path.as_deref(), Some("claude"));
    }

    // -- MailboxMessage from_row ----------------------------------------------

    #[test]
    fn mailbox_message_from_row_success() {
        let row = MailboxMessageRow {
            id: 1,
            team_id: "t1".into(),
            to_agent_id: "a1".into(),
            from_agent_id: "a2".into(),
            msg_type: "message".into(),
            content: "hello".into(),
            summary: None,
            files: None,
            read: false,
            created_at: 1000,
        };
        let msg = MailboxMessage::from_row(&row).unwrap();
        assert_eq!(msg.msg_type, MailboxMessageType::Message);
        assert!(!msg.read);
    }

    #[test]
    fn mailbox_message_from_row_idle_notification() {
        let row = MailboxMessageRow {
            id: 2,
            team_id: "t1".into(),
            to_agent_id: "lead".into(),
            from_agent_id: "a1".into(),
            msg_type: "idle_notification".into(),
            content: "done".into(),
            summary: Some("Finished task".into()),
            files: None,
            read: false,
            created_at: 2000,
        };
        let msg = MailboxMessage::from_row(&row).unwrap();
        assert_eq!(msg.msg_type, MailboxMessageType::IdleNotification);
        assert_eq!(msg.summary.as_deref(), Some("Finished task"));
    }

    #[test]
    fn mailbox_message_from_row_unknown_type() {
        let row = MailboxMessageRow {
            id: 3,
            team_id: "t1".into(),
            to_agent_id: "a1".into(),
            from_agent_id: "a2".into(),
            msg_type: "unknown_type".into(),
            content: "x".into(),
            summary: None,
            files: None,
            read: false,
            created_at: 0,
        };
        assert!(MailboxMessage::from_row(&row).is_none());
    }

    #[test]
    fn mailbox_message_serializes_type_field() {
        let msg = MailboxMessage {
            id: 1,
            team_id: "t1".into(),
            to_agent_id: "a1".into(),
            from_agent_id: "a2".into(),
            msg_type: MailboxMessageType::Message,
            content: "hello".into(),
            summary: None,
            files: None,
            read: false,
            created_at: 1000,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json.get("type").is_some(), "field must serialize as 'type'");
        assert!(json.get("msgType").is_none(), "must not serialize as 'msgType'");
        assert_eq!(json["type"], "message");
    }

    // -- TeamTask from_parts --------------------------------------------------

    #[test]
    fn team_task_from_parts_success() {
        let row = TeamTaskRow {
            id: "tk1".into(),
            team_id: "t1".into(),
            subject: "Implement".into(),
            description: Some("Details".into()),
            status: "in_progress".into(),
            owner: Some("a1".into()),
            metadata: Some(r#"{"priority":"high"}"#.into()),
            created_at: 1000,
            updated_at: 2000,
        };
        let task = TeamTask::from_parts(&row, vec!["tk0".into()], vec!["tk2".into()]).unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(task.blocked_by, vec!["tk0"]);
        assert_eq!(task.blocks, vec!["tk2"]);
        assert!(task.metadata.is_some());
    }

    #[test]
    fn team_task_from_parts_empty_deps() {
        let row = TeamTaskRow {
            id: "tk1".into(),
            team_id: "t1".into(),
            subject: "Simple".into(),
            description: None,
            status: "pending".into(),
            owner: None,
            metadata: None,
            created_at: 0,
            updated_at: 0,
        };
        let task = TeamTask::from_parts(&row, vec![], vec![]).unwrap();
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.blocked_by.is_empty());
        assert!(task.blocks.is_empty());
        assert!(task.metadata.is_none());
    }

    #[test]
    fn team_task_from_parts_unknown_status_defaults_to_pending() {
        let row = TeamTaskRow {
            id: "tk1".into(),
            team_id: "t1".into(),
            subject: "S".into(),
            description: None,
            status: "unknown".into(),
            owner: None,
            metadata: None,
            created_at: 0,
            updated_at: 0,
        };
        let task = TeamTask::from_parts(&row, vec![], vec![]).unwrap();
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[test]
    fn team_task_from_parts_invalid_metadata_json() {
        let row = TeamTaskRow {
            id: "tk1".into(),
            team_id: "t1".into(),
            subject: "S".into(),
            description: None,
            status: "pending".into(),
            owner: None,
            metadata: Some("not-json".into()),
            created_at: 0,
            updated_at: 0,
        };
        assert!(TeamTask::from_parts(&row, vec![], vec![]).is_err());
    }
}
