//! Composition for the instance-owned Feishu connector runtime.
use std::sync::Arc;
use nomifun_common::AppError;
use nomifun_knowledge::{FeishuDocContent, FeishuIndexHit, FeishuKnowledgePort};
use nomifun_feishu_connector::{FeishuConnectorService, runtime::CliRuntime, service::ConnectorSpawner, store::FeishuConnectorStore};
use crate::services::AppServices;

pub(crate) async fn build(services:&AppServices)->anyhow::Result<Arc<FeishuConnectorService>>{
    let service=services.feishu_connector.get_or_try_init(||async {
        let tasks=services.background_tasks.clone();
        let spawn:ConnectorSpawner=Arc::new(move|task|tasks.spawn(task));
        let store=Arc::new(FeishuConnectorStore::new(services.database.pool().clone()));
        let runtime=Arc::new(CliRuntime::new(services.data_dir.clone()));
        FeishuConnectorService::new(services.authoritative_user_id.to_string(),store,runtime,spawn,
            services.background_shutdown.clone(),services._boot_reconciliation_authority.is_some()).await
    }).await?;
    Ok(service.clone())
}

/// Knowledge-domain Feishu reads ride the supervised Feishu connection: the
/// service performs ready()/scope checks and bounded concurrency internally.
/// Knowledge never sees a CLI, profile, or credential.
pub(crate) struct FeishuConnectorKnowledgePort {
    hub: Arc<FeishuConnectorService>,
}

impl FeishuConnectorKnowledgePort {
    pub(crate) fn new(hub: Arc<FeishuConnectorService>) -> Self {
        Self { hub }
    }
}

#[async_trait::async_trait]
impl FeishuKnowledgePort for FeishuConnectorKnowledgePort {
    async fn search_documents(&self, query: &str) -> Result<Vec<FeishuIndexHit>, AppError> {
        // Page through Search v2 until exhaustion or the curated-corpus cap;
        // a capped page failure propagates rather than silently truncating.
        let mut hits = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .hub
                .knowledge_search_documents(query, cursor.clone())
                .await
                .map_err(AppError::from)?;
            hits.extend(page.items.iter().map(|item| FeishuIndexHit {
                token: item.id.clone(),
                kind: item.kind.clone(),
                title: item.title.clone(),
                url: item.url.clone(),
                modified_at: item.modified_at.clone(),
                summary: None,
            }));
            if !page.has_more || hits.len() >= nomifun_knowledge::feishu::MAX_FEISHU_SOURCE_ENTRIES {
                return Ok(hits);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(hits);
            }
        }
    }

    async fn inspect_link(&self, url: &str) -> Result<FeishuIndexHit, AppError> {
        let meta = self
            .hub
            .knowledge_resolve_link(url)
            .await
            .map_err(AppError::from)?;
        Ok(FeishuIndexHit {
            token: meta.token,
            kind: meta.kind,
            title: meta.title,
            url: meta.url,
            modified_at: meta.modified_at,
            summary: None,
        })
    }

    async fn fetch_document(&self, target: &str) -> Result<FeishuDocContent, AppError> {
        let body = self
            .hub
            .knowledge_fetch_document(target)
            .await
            .map_err(AppError::from)?;
        Ok(FeishuDocContent {
            document_id: body.document_id,
            markdown: body.markdown,
            revision_id: body.revision_id,
        })
    }
}
