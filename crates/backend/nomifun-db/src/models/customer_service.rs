use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `cs_agents` table — one customer-service employee.
///
/// The SQLite-local technical key is intentionally omitted. `cs_agent_id` is
/// the stable UUIDv7 business identity used by every relationship and API
/// outside the repository. `knowledge_base_ids` is stored as a JSON array
/// string; use [`CsAgentRow::knowledge_base_ids_vec`] for the decoded form.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsAgentRow {
    pub cs_agent_id: String,
    pub name: String,
    pub greeting: String,
    pub persona: String,
    pub service_policy: String,
    /// Logical reference to `providers.provider_id`; the provider row may be
    /// deleted afterwards (KeepHistory) — resolve at call time.
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// JSON array of `knowledge_bases.knowledge_base_id` values.
    pub knowledge_base_ids: String,
    pub enabled: bool,
    /// Per-agent concurrent turn ceiling (1..=64, default 8).
    pub max_concurrent: i64,
    pub audit_retention_days: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl CsAgentRow {
    /// Decode the stored JSON array of knowledge-base IDs. Invalid JSON is
    /// treated as an empty list rather than failing a read path.
    pub fn knowledge_base_ids_vec(&self) -> Vec<String> {
        serde_json::from_str(&self.knowledge_base_ids).unwrap_or_default()
    }

    /// Encode a list of knowledge-base IDs into the stored JSON column form.
    pub fn encode_knowledge_base_ids(ids: &[String]) -> String {
        serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_owned())
    }
}

/// Values accepted when inserting a `cs_agents` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCsAgentRow {
    pub cs_agent_id: String,
    pub name: String,
    pub greeting: String,
    pub persona: String,
    pub service_policy: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// JSON array string (`CsAgentRow::encode_knowledge_base_ids`).
    pub knowledge_base_ids: String,
    pub enabled: bool,
    pub max_concurrent: i64,
    pub audit_retention_days: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `cs_channel_bindings` table — bot ↔ agent binding.
/// `channel_plugin_id` is UNIQUE: one bot serves at most one agent.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsChannelBindingRow {
    pub cs_agent_id: String,
    pub channel_plugin_id: String,
    pub created_at: TimestampMs,
}

/// Row mapping for the `cs_dialogues` table — one visitor lane
/// (`channel_plugin_id`, `channel_user_id`, `chat_id` triple is UNIQUE).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsDialogueRow {
    pub cs_dialogue_id: String,
    pub cs_agent_id: String,
    pub channel_plugin_id: String,
    pub channel_user_id: String,
    pub chat_id: String,
    /// `open` | `closed`.
    pub state: String,
    pub created_at: TimestampMs,
    pub last_activity: TimestampMs,
}

/// Row mapping for the `cs_messages` table — dialogue transcript entry.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsMessageRow {
    pub cs_message_id: String,
    pub cs_dialogue_id: String,
    /// `visitor` | `agent` | `system`.
    pub role: String,
    pub content: String,
    pub created_at: TimestampMs,
}

/// Row mapping for the `cs_notes` table — owner-maintained read-only FAQ /
/// script / business-fact notes. `cs_agent_id = NULL` means shared by all
/// customer-service agents.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsNoteRow {
    pub cs_note_id: String,
    pub cs_agent_id: Option<String>,
    pub kind: String,
    pub content: String,
    /// Owner-authored alternate phrasings of the same question, newline
    /// separated. Folded into the searchable text alongside `content`.
    ///
    /// This is the synonym channel for note recall. Purely lexical matching
    /// provably cannot bridge a paraphrase that shares no vocabulary with the
    /// note (「这个软件是干什么的」 against a note about NomiFun), and this
    /// repository has no offline embedding capability to fall back on, so the
    /// owner supplies the phrasings their visitors actually use. Auditable and
    /// correctable, with no provider on the reply path.
    #[serde(default)]
    pub aliases: String,
    pub enabled: bool,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `cs_audit_events` table — in-database audit trail
/// (replaces the retired public-agent JSONL side store). Rows are pruned by
/// `cs_agents.audit_retention_days`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CsAuditEventRow {
    pub cs_agent_id: String,
    pub kind: String,
    pub platform: String,
    pub detail: String,
    pub created_at: TimestampMs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cs_agent_row_roundtrips_and_decodes_kb_ids() {
        let row = CsAgentRow {
            cs_agent_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            name: "小客服".into(),
            greeting: "您好".into(),
            persona: "耐心".into(),
            service_policy: "只答业务问题".into(),
            provider_id: None,
            model: Some("gpt-x".into()),
            knowledge_base_ids: r#"["0190f5fe-7c00-7a00-8000-000000000002"]"#.into(),
            enabled: true,
            max_concurrent: 8,
            audit_retention_days: 30,
            created_at: 1,
            updated_at: 2,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: CsAgentRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cs_agent_id, row.cs_agent_id);
        assert_eq!(
            back.knowledge_base_ids_vec(),
            vec!["0190f5fe-7c00-7a00-8000-000000000002".to_owned()]
        );
    }

    #[test]
    fn kb_ids_decode_tolerates_garbage() {
        let mut row_json = serde_json::json!({
            "cs_agent_id": "0190f5fe-7c00-7a00-8000-000000000001",
            "name": "n", "greeting": "", "persona": "", "service_policy": "",
            "provider_id": null, "model": null,
            "knowledge_base_ids": "not-json",
            "enabled": true, "max_concurrent": 8, "audit_retention_days": 30,
            "created_at": 1, "updated_at": 1
        });
        let row: CsAgentRow = serde_json::from_value(row_json.take()).unwrap();
        assert!(row.knowledge_base_ids_vec().is_empty());
    }

    #[test]
    fn encode_kb_ids_produces_json_array() {
        assert_eq!(CsAgentRow::encode_knowledge_base_ids(&[]), "[]");
        assert_eq!(
            CsAgentRow::encode_knowledge_base_ids(&["a".into(), "b".into()]),
            r#"["a","b"]"#
        );
    }
}
