//! Single shared [`ComputerTool`] for the gateway's computer-use capabilities.
//!
//! Unlike the browser (one Chrome per companion), the desktop is one physical
//! screen, so a single shared `ComputerTool` is the right model — exactly like
//! the inward `mcp-computer-stdio` bridge. `ComputerTool` is stateful
//! (`is_concurrency_safe == false`: shared observe/screenshot caches and `[ref]`
//! resolution), so all calls are serialized behind one lock. Only compiled with
//! the `computer-use` feature.

use std::sync::Arc;

use nomi_computer::tool::ComputerTool;
use nomi_config::config::ComputerConfig;
use nomi_tools::Tool;
use nomi_types::tool::ToolResult;
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// Owns the shared desktop `ComputerTool` and serializes calls to it.
pub struct ComputerRegistry {
    tool: Arc<ComputerTool>,
    /// One global lock: the desktop is a single screen and `ComputerTool` keeps
    /// mutable observe/screenshot caches that concurrent callers would clobber
    /// (a stale `[ref]` resolves against the wrong snapshot).
    lock: Mutex<()>,
}

impl ComputerRegistry {
    pub fn new() -> Self {
        Self {
            tool: Arc::new(ComputerTool::new(&ComputerConfig::default())),
            lock: Mutex::new(()),
        }
    }

    /// Forward a `{"action": ..}` payload to the shared tool, serialized.
    pub async fn execute(&self, input: Value) -> ToolResult {
        let _guard = self.lock.lock().await;
        self.tool.execute(input).await
    }
}

impl Default for ComputerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a `ToolResult` onto the gateway result envelope: error → `{"error": ..}`;
/// success → `{"result": {"text": .., "images": [{media_type, data}]}}` (base64
/// screenshots / Set-of-Marks overlays flow straight through). Mirrors the
/// browser registry's helper of the same name (kept separate so a computer-only
/// build needs no browser-use feature).
pub fn tool_result_to_value(result: ToolResult) -> Value {
    if result.is_error {
        return json!({ "error": result.content });
    }
    let mut payload = json!({ "text": result.content });
    if !result.images.is_empty() {
        let imgs: Vec<Value> = result
            .images
            .iter()
            .map(|img| json!({ "media_type": img.media_type, "data": img.data }))
            .collect();
        payload["images"] = Value::Array(imgs);
    }
    json!({ "result": payload })
}
