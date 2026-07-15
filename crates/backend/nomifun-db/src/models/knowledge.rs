use nomifun_common::{
    CompanionId, ConversationId, KnowledgeBaseId, KnowledgeBindingId, TerminalId, TimestampMs,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqliteRow};

/// Row in the `knowledge_bases` table — a registered directory of markdown
/// documents. The directory is the source of truth for content; the row only
/// stores registration metadata (the user may drop files in at any time).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KnowledgeBaseRow {
    #[sqlx(try_from = "String")]
    pub id: KnowledgeBaseId,
    pub name: String,
    pub description: String,
    /// Absolute root directory of the base.
    pub root_path: String,
    /// `true` when the directory lives under `{data_dir}/knowledge/{id}` and
    /// is owned by us (purge-on-delete allowed); `false` for user-referenced
    /// external directories which we never modify structurally.
    pub managed: bool,
    pub extra: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    /// JSON array of tag keys assigned to this base; NULL = no tags.
    /// Deserialized by the service layer, stored opaquely here.
    pub tags: Option<String>,
}

/// Row in the `knowledge_bindings` table — which bases a target mounts and
/// whether write-back is allowed. The former composite (target_kind,target_id)
/// PK + JSON `kb_ids` array is redesigned into a surrogate `binding_id` +
/// type-discriminated nullable target columns (exactly one non-null, enforced
/// by a CHECK) + the `knowledge_binding_bases` junction.
///   - `target_workpath`: normalized workspace path key (not an entity, no FK)
///   - `target_conv_id` / `target_term_id`: real TEXT FK (CASCADE)
///   - `target_companion_id`: filesystem companion entity (no FK)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBindingRow {
    pub binding_id: KnowledgeBindingId,
    pub target_kind: String,
    pub target_workpath: Option<String>,
    pub target_conv_id: Option<ConversationId>,
    pub target_term_id: Option<TerminalId>,
    pub target_companion_id: Option<CompanionId>,
    pub enabled: bool,
    pub writeback: bool,
    /// `staged` (agent writes confined to `_inbox/{conversation_id}/`,
    /// conflict-free across sessions) or `direct` (agent may edit the base
    /// body). Only meaningful while `writeback` is true.
    pub writeback_mode: String,
    /// Write-back disposition ("回写意识"), orthogonal to `writeback_mode`:
    /// `conservative` (restrained, the default — only clearly-worth-keeping
    /// knowledge) or `aggressive` (capture anything plausibly relevant). Only
    /// meaningful while `writeback` is true.
    pub writeback_eagerness: String,
    /// When `true`, an external IM Channel Agent binding may write back
    /// (forced to STAGED placement). Default `false` — channel writes are
    /// disabled unless the user explicitly re-enables them. Ignored for
    /// non-channel surfaces.
    pub channel_write_enabled: bool,
    pub updated_at: TimestampMs,
}

impl KnowledgeBindingRow {
    /// Resolve the target id for the row's kind (the value the service layer
    /// addresses bindings by), as an owned string. `workpath`/`companion` targets are
    /// TEXT, including the typed conversation and terminal entity IDs.
    pub fn target_id(&self) -> Option<String> {
        match self.target_kind.as_str() {
            "workpath" => self.target_workpath.clone(),
            "conversation" => self.target_conv_id.as_ref().map(ToString::to_string),
            "terminal" => self.target_term_id.as_ref().map(ToString::to_string),
            "companion" => self.target_companion_id.as_ref().map(ToString::to_string),
            _ => None,
        }
    }
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for KnowledgeBindingRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        fn parse_required<T>(value: String) -> Result<T, sqlx::Error>
        where
            T: TryFrom<String>,
            T::Error: std::error::Error + Send + Sync + 'static,
        {
            T::try_from(value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
        }

        fn parse_optional<T>(value: Option<String>) -> Result<Option<T>, sqlx::Error>
        where
            T: TryFrom<String>,
            T::Error: std::error::Error + Send + Sync + 'static,
        {
            value.map(parse_required).transpose()
        }

        Ok(Self {
            binding_id: parse_required(row.try_get("binding_id")?)?,
            target_kind: row.try_get("target_kind")?,
            target_workpath: row.try_get("target_workpath")?,
            target_conv_id: parse_optional(row.try_get("target_conv_id")?)?,
            target_term_id: parse_optional(row.try_get("target_term_id")?)?,
            target_companion_id: parse_optional(row.try_get("target_companion_id")?)?,
            enabled: row.try_get("enabled")?,
            writeback: row.try_get("writeback")?,
            writeback_mode: row.try_get("writeback_mode")?,
            writeback_eagerness: row.try_get("writeback_eagerness")?,
            channel_write_enabled: row.try_get("channel_write_enabled")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// Row in the `knowledge_tags` table — a user-defined tag definition.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KnowledgeTagRow {
    pub key: String,
    pub label: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
}

/// Parameters for creating a knowledge tag.
#[derive(Debug, Clone)]
pub struct CreateKnowledgeTagParams {
    pub key: String,
    pub label: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
}

/// Parameters for updating a knowledge tag (all fields optional — only non-None
/// fields are written).
#[derive(Debug, Clone, Default)]
pub struct UpdateKnowledgeTagParams {
    pub label: Option<String>,
    pub color: Option<Option<String>>,
    pub sort_order: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_rows_roundtrip() {
        let base_id = KnowledgeBaseId::new();
        let base = KnowledgeBaseRow {
            id: base_id.clone(),
            name: "领域知识".into(),
            description: "测试".into(),
            root_path: format!("C:/data/knowledge/{base_id}"),
            managed: true,
            extra: "{}".into(),
            created_at: 1,
            updated_at: 2,
            tags: None,
        };
        let back: KnowledgeBaseRow = serde_json::from_str(&serde_json::to_string(&base).unwrap()).unwrap();
        assert_eq!(back.id, base.id);
        assert!(back.managed);

        let conversation_id = ConversationId::new();
        let binding = KnowledgeBindingRow {
            binding_id: KnowledgeBindingId::new(),
            target_kind: "conversation".into(),
            target_workpath: None,
            target_conv_id: Some(conversation_id.clone()),
            target_term_id: None,
            target_companion_id: None,
            enabled: true,
            writeback: false,
            writeback_mode: "staged".into(),
            writeback_eagerness: "conservative".into(),
            channel_write_enabled: false,
            updated_at: 3,
        };
        let back: KnowledgeBindingRow = serde_json::from_str(&serde_json::to_string(&binding).unwrap()).unwrap();
        assert!(back.enabled);
        assert!(!back.writeback);
        assert_eq!(
            back.target_id(),
            Some(conversation_id.into_string())
        );
    }
}
