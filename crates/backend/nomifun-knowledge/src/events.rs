//! Installation-owner-scoped WS events for the knowledge domain.

use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_common::{KnowledgeBaseId, KnowledgeEntryId};
use nomifun_realtime::UserEventSink;

/// One atomic knowledge-tree relocation. Paths are base-relative and use
/// forward slashes on every platform. A directory relocation describes all
/// descendants through a single prefix mapping instead of flooding clients
/// with one event per child.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeTreeChangedEvent {
    pub knowledge_base_id: KnowledgeBaseId,
    pub operation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<KnowledgeEntryId>,
    pub old_prefix: String,
    pub new_prefix: String,
    pub kind: String,
    pub moved_descendant_count: u64,
    pub tree_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeEntryContentUpdatedEvent {
    pub knowledge_base_id: KnowledgeBaseId,
    pub entry_id: KnowledgeEntryId,
    pub rel_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
}

#[derive(Clone)]
pub struct KnowledgeEventEmitter {
    sink: Arc<dyn UserEventSink>,
    authoritative_user_id: Arc<str>,
}

impl KnowledgeEventEmitter {
    pub fn new(
        sink: Arc<dyn UserEventSink>,
        authoritative_user_id: Arc<str>,
    ) -> Self {
        Self {
            sink,
            authoritative_user_id,
        }
    }

    fn try_broadcast<T: serde::Serialize>(
        &self,
        event_name: &str,
        payload: &T,
    ) -> Result<(), serde_json::Error> {
        let value = serde_json::to_value(payload)?;
        self.sink.send_to_user(
            &self.authoritative_user_id,
            WebSocketMessage::new(event_name, value),
        );
        Ok(())
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        if let Err(error) = self.try_broadcast(event_name, payload) {
            tracing::warn!(%error, event_name, "failed to serialize knowledge event");
        }
    }

    pub fn emit_base_created<T: serde::Serialize>(&self, base: &T) {
        self.broadcast("knowledge.base-created", base);
    }

    pub fn emit_base_updated<T: serde::Serialize>(&self, base: &T) {
        self.broadcast("knowledge.base-updated", base);
    }

    pub fn emit_base_deleted(&self, id: &KnowledgeBaseId) {
        self.broadcast(
            "knowledge.base-deleted",
            &serde_json::json!({ "knowledge_base_id": id }),
        );
    }

    pub fn emit_tree_changed(&self, change: &KnowledgeTreeChangedEvent) {
        self.broadcast("knowledge.tree-changed", change);
    }

    pub fn emit_entry_content_updated(&self, change: &KnowledgeEntryContentUpdatedEvent) {
        self.broadcast("knowledge.entry-content-updated", change);
    }

    /// Publish one durable tree-outbox payload. Returning serialization
    /// failure lets the coordinator leave the outbox row pending instead of
    /// acknowledging an event that was never handed to the realtime sink.
    pub(crate) fn try_emit_tree_changed(
        &self,
        change: &KnowledgeTreeChangedEvent,
    ) -> Result<(), serde_json::Error> {
        self.try_broadcast("knowledge.tree-changed", change)
    }

    pub fn emit_binding_changed<T: serde::Serialize>(&self, binding: &T) {
        self.broadcast("knowledge.binding-changed", binding);
    }

    /// A tag was created / renamed / recolored / reordered / deleted. Consumers
    /// (the filter bar, tag→label maps, the management modal) just re-list, so
    /// the payload is a bare signal rather than a per-entity diff.
    pub fn emit_tag_changed(&self) {
        self.broadcast("knowledge.tag-changed", &serde_json::json!({}));
    }
}
