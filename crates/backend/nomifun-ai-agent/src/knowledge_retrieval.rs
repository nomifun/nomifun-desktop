//! Production `KnowledgeRetrievalSink` over `KnowledgeService`. Mirrors
//! `LiveKnowledgeCompleter`: the trait lives in `nomi-agent`, this is the
//! backend impl wired in at app startup.

use std::sync::Arc;

use async_trait::async_trait;
use nomi_agent::knowledge_tools::{KnowledgeHit, KnowledgeRetrievalSink};
use nomifun_knowledge::KnowledgeService;

/// Bridges the agent-facing retrieval trait to the backend KnowledgeService.
pub struct LiveKnowledgeRetrievalSink {
    pub service: Arc<KnowledgeService>,
}

#[async_trait]
impl KnowledgeRetrievalSink for LiveKnowledgeRetrievalSink {
    async fn search(&self, kb_ids: &[String], query: &str, limit: usize) -> Result<Vec<KnowledgeHit>, String> {
        let hits = self
            .service
            .search_bases(kb_ids, query, limit)
            .await
            .map_err(|e| e.to_string())?;
        Ok(hits
            .into_iter()
            .map(|h| KnowledgeHit {
                handle: nomifun_knowledge::encode_doc_handle(&h.kb_id, &h.rel_path),
                kb_id: h.kb_id,
                kb_name: h.kb_name,
                rel_path: h.rel_path,
                heading: h.heading,
                snippet: h.snippet,
            })
            .collect())
    }

    async fn read_document(&self, kb_ids: &[String], handle: &str) -> Result<String, String> {
        let (kb_id, rel_path) =
            nomifun_knowledge::decode_doc_handle(handle).ok_or_else(|| format!("invalid handle: {handle}"))?;
        if !kb_ids.iter().any(|b| b == &kb_id) {
            return Err("handle points to a base not mounted in this session".to_owned());
        }
        let file = self.service.read_file(&kb_id, &rel_path).await.map_err(|e| e.to_string())?;
        Ok(file.content)
    }
}
