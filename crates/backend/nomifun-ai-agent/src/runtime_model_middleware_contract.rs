//! Request-local model middleware. The engine remains the only owner of the
//! conversation, tool authority and provider call; middleware returns data.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context_contributor::TurnContext;

pub const MAX_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_PATCH_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct ModelToolMetadata {
    pub name: String,
    pub description: String,
}

/// No history, image bytes, tool schemas, handles or mutable engine references.
#[derive(Clone, Debug, Serialize)]
pub struct BeforeModelInput {
    pub phase: &'static str,
    pub turn: TurnContext,
    pub system: String,
    pub tools: Vec<ModelToolMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequestPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_names: Option<Vec<String>>,
}

#[async_trait]
pub trait ModelRequestMiddleware: Send + Sync {
    async fn before_model(&self, input: BeforeModelInput) -> Result<ModelRequestPatch, String>;
    fn label(&self) -> &str;
}
