use std::sync::Arc;

use async_trait::async_trait;
use nomifun_api_types::CreateRequirementRequest;
use nomifun_common::RequirementCreator;

use crate::service::RequirementService;

/// Creates tracked requirements for the opt-in channel inbound pipeline.
pub struct RequirementServiceSink {
    service: Arc<RequirementService>,
}

impl RequirementServiceSink {
    /// Build a [`RequirementCreator`] trait object for the
    /// opt-in IM → requirement pipeline (channel inbound → tracked requirement).
    pub fn creator_arc(service: Arc<RequirementService>) -> Arc<dyn RequirementCreator> {
        Arc::new(Self { service })
    }
}

#[async_trait]
impl RequirementCreator for RequirementServiceSink {
    async fn create_from_message(
        &self,
        title: &str,
        content: &str,
        tag: &str,
        created_by: &str,
    ) -> Result<String, String> {
        let req = CreateRequirementRequest {
            title: title.to_string(),
            content: content.to_string(),
            tag: tag.to_string(),
            order_key: None,
            status: None, // None → Pending → wakes AutoWork
            created_by: Some(created_by.to_string()),
            attachments: Vec::new(),
        };
        self.service
            .create(req)
            .await
            .map(|r| r.requirement_id)
            .map_err(|e| e.to_string())
    }
}
