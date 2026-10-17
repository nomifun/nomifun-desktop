//! Memory output for Agent model context only. The Store, REST and export
//! contracts retain complete provenance. Source filters must run in the Store
//! before this projection removes the potentially unbounded lineage arrays.

use nomifun_common::TimestampMs;
use serde::Serialize;

use crate::store::CompanionMemory;

/// A memory fact and its primary provenance, without the accumulated lineage
/// that is retained for source validation and retraction in durable storage.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentMemoryProjection {
    pub memory_id: String,
    pub kind: String,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: f64,
    pub strength: f64,
    pub pinned: bool,
    pub source: String,
    pub status: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub last_reinforced_at: TimestampMs,
    pub companion_id: Option<String>,
    pub revision: i64,
    pub source_session_id: Option<String>,
    pub source_operation_id: Option<String>,
}

impl From<CompanionMemory> for AgentMemoryProjection {
    fn from(memory: CompanionMemory) -> Self {
        Self {
            memory_id: memory.memory_id,
            kind: memory.kind,
            content: memory.content,
            tags: memory.tags,
            importance: memory.importance,
            strength: memory.strength,
            pinned: memory.pinned,
            source: memory.source,
            status: memory.status,
            created_at: memory.created_at,
            updated_at: memory.updated_at,
            last_reinforced_at: memory.last_reinforced_at,
            companion_id: memory.companion_id,
            revision: memory.revision,
            source_session_id: memory.source_session_id,
            source_operation_id: memory.source_operation_id,
        }
    }
}

impl From<&CompanionMemory> for AgentMemoryProjection {
    fn from(memory: &CompanionMemory) -> Self {
        Self {
            memory_id: memory.memory_id.clone(),
            kind: memory.kind.clone(),
            content: memory.content.clone(),
            tags: memory.tags.clone(),
            importance: memory.importance,
            strength: memory.strength,
            pinned: memory.pinned,
            source: memory.source.clone(),
            status: memory.status.clone(),
            created_at: memory.created_at,
            updated_at: memory.updated_at,
            last_reinforced_at: memory.last_reinforced_at,
            companion_id: memory.companion_id.clone(),
            revision: memory.revision,
            source_session_id: memory.source_session_id.clone(),
            source_operation_id: memory.source_operation_id.clone(),
        }
    }
}

/// An already-filtered search hit prepared for bounded Agent output.
#[derive(Debug, Clone, Serialize)]
pub struct AgentMemorySearchHit {
    pub memory: AgentMemoryProjection,
    pub rank: f64,
    pub snippet: Option<String>,
}

impl From<crate::MemorySearchHit> for AgentMemorySearchHit {
    fn from(hit: crate::MemorySearchHit) -> Self {
        Self {
            memory: hit.memory.into(),
            rank: hit.rank,
            snippet: hit.snippet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CompanionMemoryMessageSource;

    fn memory_fixture() -> CompanionMemory {
        CompanionMemory {
            memory_id: nomifun_common::CompanionMemoryId::new().into_string(),
            kind: "preference".into(),
            content: "主人现在只喝浅烘焙咖啡".into(),
            tags: vec!["coffee".into()],
            importance: 0.9,
            strength: 0.8,
            pinned: true,
            source: "agent".into(),
            status: "active".into(),
            created_at: 10,
            updated_at: 30,
            last_reinforced_at: 20,
            companion_id: Some(nomifun_common::CompanionId::new().into_string()),
            revision: 23,
            source_session_id: Some(nomifun_common::ConversationId::new().into_string()),
            source_operation_id: Some("primary-operation".into()),
            source_event_ids: vec![],
            source_message_refs: vec![],
        }
    }

    #[test]
    fn agent_projection_size_is_independent_of_full_lineage() {
        let mut row = memory_fixture();
        let baseline = serde_json::to_vec(&AgentMemoryProjection::from(row.clone())).unwrap();
        for _ in 0..1024 {
            row.source_event_ids.push(nomifun_common::CompanionEventId::new().into_string());
            row.source_message_refs.push(CompanionMemoryMessageSource {
                session_id: nomifun_common::ConversationId::new().into_string(),
                message_id: nomifun_common::generate_id(),
            });
        }
        let full = serde_json::to_value(&row).unwrap();
        assert!(serde_json::to_vec(&full).unwrap().len() - baseline.len() > 64 * 1024);
        let unchanged = row.clone();
        let projected = AgentMemoryProjection::from(row.clone());
        assert_eq!(serde_json::to_vec(&projected).unwrap(), baseline);
        assert_eq!(row, unchanged, "model projection must not mutate the stored row");
        assert_eq!(AgentMemoryProjection::from(&row), projected);
        let mut expected = full;
        let fields = expected.as_object_mut().unwrap();
        fields.remove("source_event_ids");
        fields.remove("source_message_refs");
        let actual = serde_json::to_value(&projected).unwrap();
        assert_eq!(actual, expected, "all ordinary fields and primary provenance survive exactly");
        assert!(actual.get("source_event_ids").is_none());
        assert!(actual.get("source_message_refs").is_none());
        assert_eq!(projected.memory_id, row.memory_id);
        assert_eq!(projected.revision, row.revision);
        assert_eq!(projected.content, row.content);
        assert_eq!(projected.source_session_id, row.source_session_id);
        assert_eq!(projected.source_operation_id, row.source_operation_id);
    }

    #[test]
    fn agent_search_hit_preserves_rank_and_snippet() {
        let memory = memory_fixture();
        let expected = AgentMemoryProjection::from(&memory);
        let hit = AgentMemorySearchHit::from(crate::MemorySearchHit {
            memory,
            rank: -1.25,
            snippet: Some("只喝<b>浅烘焙</b>咖啡".into()),
        });
        assert_eq!(hit.memory, expected);
        assert_eq!(hit.rank, -1.25);
        assert_eq!(hit.snippet.as_deref(), Some("只喝<b>浅烘焙</b>咖啡"));
    }
}
