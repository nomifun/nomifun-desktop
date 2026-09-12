//! Deferred activation gate for the native image-input context path.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};
use serde_json::{Value, json};

pub const VISION_ACTIVATE_TOOL_NAME: &str = "activate_vision_input";

pub struct VisionActivationTool {
    active: Arc<AtomicBool>,
}

impl VisionActivationTool {
    pub fn new(active: Arc<AtomicBool>) -> Self {
        Self { active }
    }
}

#[async_trait]
impl Tool for VisionActivationTool {
    fn name(&self) -> &str {
        VISION_ACTIVATE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Activate image attachments for subsequent turns in this AgentSession. The exact Chat model was already verified to support image input."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        self.active.store(true, Ordering::Release);
        ToolResult::text(json!({"vision_input":"active"}).to_string())
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

pub fn is_active(active: &Arc<AtomicBool>) -> bool {
    active.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn activation_is_explicit_and_monotonic() {
        let active = Arc::new(AtomicBool::new(false));
        let tool = VisionActivationTool::new(Arc::clone(&active));
        assert!(!is_active(&active));
        let result = tool.execute(json!({})).await;
        assert!(!result.is_error);
        assert!(is_active(&active));
    }
}
