//! Knowledge-owned headless renderer.
//!
//! This service port is deliberately separate from the Agent `browser`
//! Module. It has no AgentSession, Browser Action grant, Role Provider, or
//! interactive Browser Resource and therefore cannot be selected as an Agent
//! Browser implementation.

use crate::headless_render::HeadlessRenderRuntime;
use nomifun_common::AppError;
use nomifun_knowledge::source_url::{
    BrowserRenderContent, BrowserRenderContentPort, BrowserRenderContentRequest,
};
use std::sync::Arc;

pub(crate) struct KnowledgeHeadlessRenderPort {
    runtime: Arc<HeadlessRenderRuntime>,
}

impl KnowledgeHeadlessRenderPort {
    pub(crate) fn bind(runtime: Arc<HeadlessRenderRuntime>) -> Arc<Self> {
        Arc::new(Self { runtime })
    }
}

#[async_trait::async_trait]
impl BrowserRenderContentPort for KnowledgeHeadlessRenderPort {
    async fn render_content(
        &self,
        input: BrowserRenderContentRequest,
    ) -> Result<BrowserRenderContent, AppError> {
        let url = url::Url::parse(&input.url)
            .map_err(|_| AppError::BadRequest("BROWSER_RENDER_INVALID_URL".into()))?;
        if input.url.len() > 8192
            || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::BadRequest("BROWSER_RENDER_INVALID_URL".into()));
        }
        let output = self.runtime.render(url).await.map_err(|error| {
            AppError::BadGateway(format!("BROWSER_RENDER_FAILED: {error}"))
        })?;
        Ok(BrowserRenderContent {
            final_url: output.final_url,
            html: output.html,
            html_truncated: output.html_truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn knowledge_renderer_has_no_agent_browser_authority() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let port = KnowledgeHeadlessRenderPort::bind(runtime.clone());
        let content = port
            .render_content(BrowserRenderContentRequest::new("https://example.com/"))
            .await
            .unwrap();
        assert_eq!(content.final_url, "https://example.com/");
        assert!(content.html.contains("selected Provider"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn renderer_rejects_non_http_sources_before_dispatch() {
        let (runtime, calls) = crate::headless_render::test_support::runtime();
        let port = KnowledgeHeadlessRenderPort::bind(runtime.clone());
        assert!(port
            .render_content(BrowserRenderContentRequest::new("file:///private"))
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        runtime.shutdown().await.unwrap();
    }
}
