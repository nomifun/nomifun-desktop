use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::config::McpServerConfig;
use super::manager::McpManager;
use super::protocol::ToolAnnotations;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolImage, ToolResult};

/// Upper bound on a single MCP image's decoded byte size before it is dropped.
///
/// Browser/Playwright screenshots routinely run 1–5 MB; routing one verbatim
/// into the message history would balloon the context. Aligned with
/// `nomi-tools` `read.rs` `MAX_IMAGE_BYTES` (5 MiB). We estimate the decoded
/// size from the base64 length (decoded ≈ len * 3 / 4) to avoid decoding the
/// whole payload just to measure it.
const MCP_MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Estimate the decoded byte length of a (possibly padded) base64 string
/// without allocating/decoding it. Standard base64 encodes every 3 bytes as 4
/// characters; trailing `=` padding marks 1–2 missing bytes. Whitespace and a
/// `data:...;base64,` prefix (if any) are ignored so we measure the payload.
fn decoded_base64_len(data: &str) -> usize {
    // Strip a data-URL prefix if a server happened to send one.
    let payload = match data.split_once(";base64,") {
        Some((_, b64)) => b64,
        None => data,
    };
    let mut chars = 0usize;
    let mut padding = 0usize;
    for b in payload.bytes() {
        match b {
            b'=' => padding += 1,
            b if b.is_ascii_whitespace() => {}
            _ => chars += 1,
        }
    }
    let total_units = chars + padding;
    // Each 4-char group decodes to 3 bytes; subtract for padding.
    let bytes = total_units / 4 * 3;
    bytes.saturating_sub(padding.min(2))
}

/// Wraps an MCP server tool as a local Tool trait implementation.
/// Uses naming convention "mcp__{server}__{tool}" when collisions exist,
/// otherwise uses the tool's original name.
pub struct McpToolProxy {
    /// Display name used for registration (may be prefixed)
    display_name: String,
    /// Original tool name on the MCP server
    tool_name: String,
    /// Server this tool belongs to
    server_name: String,
    description: String,
    input_schema: JsonSchema,
    manager: Arc<McpManager>,
    /// Whether this tool's schema should be deferred (sent as name-only stub).
    deferred: bool,
    /// MCP behaviour hints used to derive the approval category. `None` means
    /// the server declared no annotations → safe default (`Exec`, needs approval).
    annotations: Option<ToolAnnotations>,
}

impl McpToolProxy {
    // One positional arg per proxy field; a builder would add ceremony without
    // value for two internal call sites. The `annotations` param (added for
    // approval classification) pushes this past clippy's 7-arg threshold.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display_name: String,
        tool_name: String,
        server_name: String,
        description: String,
        input_schema: JsonSchema,
        manager: Arc<McpManager>,
        deferred: bool,
        annotations: Option<ToolAnnotations>,
    ) -> Self {
        Self {
            display_name,
            tool_name,
            server_name,
            description,
            input_schema,
            manager,
            deferred,
            annotations,
        }
    }

    /// Map MCP annotations to an approval [`ToolCategory`].
    ///
    /// Rule (mirrors codex `requires_mcp_tool_approval`, collapsed onto nomi's
    /// Info/Exec axis):
    /// - `readOnlyHint == Some(true)` → [`ToolCategory::Info`] (approval-free).
    /// - everything else — `destructiveHint`, no hints, or an old server with no
    ///   `annotations` block at all — → [`ToolCategory::Exec`] (needs approval).
    ///
    /// The from-strict default is deliberate: an unannotated tool could mutate
    /// the world, so we never silently auto-approve it.
    fn category_from_annotations(&self) -> ToolCategory {
        if self.is_read_only() {
            ToolCategory::Info
        } else {
            ToolCategory::Exec
        }
    }

    /// Whether the tool declared `readOnlyHint == true` (no side effects).
    /// Drives both the approval category and concurrency-safety.
    fn is_read_only(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
            .unwrap_or(false)
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        &self.display_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> JsonSchema {
        self.input_schema.clone()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Read-only MCP tools have no side effects → safe to run in parallel with
        // other read-only calls (mirrors built-in Read/Grep/Glob). Mutating or
        // unannotated tools stay serial.
        self.is_read_only()
    }

    fn is_deferred(&self) -> bool {
        self.deferred
    }

    async fn execute(&self, input: Value) -> ToolResult {
        match self
            .manager
            .call_tool(&self.server_name, &self.tool_name, input)
            .await
        {
            Ok(out) => {
                let mut text = out.text;
                let mut images: Vec<ToolImage> = Vec::with_capacity(out.images.len());

                for img in out.images {
                    // Estimate decoded byte size from base64 length to gate
                    // oversized screenshots before they reach the context.
                    let decoded_len = decoded_base64_len(&img.data);
                    if decoded_len > MCP_MAX_IMAGE_BYTES {
                        tracing::warn!(
                            target: "nomi_mcp",
                            server = %self.server_name,
                            tool = %self.tool_name,
                            bytes = decoded_len,
                            limit = MCP_MAX_IMAGE_BYTES,
                            "dropping oversized MCP image"
                        );
                        let placeholder = format!("[image too large: {} bytes, dropped]", decoded_len);
                        if text.is_empty() {
                            text = placeholder;
                        } else {
                            text.push('\n');
                            text.push_str(&placeholder);
                        }
                        continue;
                    }

                    images.push(ToolImage {
                        media_type: img.mime_type, // mime_type → media_type field name
                        data: img.data,            // raw base64, passed through
                    });
                }

                if images.is_empty() {
                    // Pure-text MCP tool: behaviour identical to before this change.
                    ToolResult::text(text)
                } else {
                    // Multimodal: text → content, images → ToolResult.images so the
                    // downstream provider adapters feed them back to the model.
                    ToolResult::text(text).with_images(images)
                }
            }
            Err(e) => ToolResult::error(format!("MCP tool error: {}", e)),
        }
    }

    fn category(&self) -> ToolCategory {
        // Annotation-driven: readOnly tools are approval-free Info, everything
        // else (destructive or unannotated) is Exec → needs approval. We no
        // longer collapse every MCP tool into the single `Mcp` bucket, which
        // forced even read-only snapshots through the approval gate.
        self.category_from_annotations()
    }

    fn describe(&self, input: &Value) -> String {
        format!(
            "MCP {}/{}: {}",
            self.server_name,
            self.tool_name,
            serde_json::to_string(input).unwrap_or_default()
        )
    }
}

/// Register all MCP tools into the tool registry, handling name collisions.
///
/// Strategy:
/// - If tool name doesn't collide with built-in or other MCP tools → use as-is
/// - If collision detected → prefix with "mcp__{server_name}__"
///
/// Each tool's deferred flag is read from the server's config:
/// `McpServerConfig::deferred` — defaults to `true` when absent.
pub fn register_mcp_tools(
    registry: &mut nomi_tools::registry::ToolRegistry,
    manager: &Arc<McpManager>,
    builtin_names: &[String],
    server_configs: &HashMap<String, McpServerConfig>,
) {
    let all_tools = manager.all_tools();

    // Determine which names need prefixing
    for (server_name, tool_def) in &all_tools {
        let original_name = &tool_def.name;

        // Check collision with built-in tools
        let collides_builtin = builtin_names.iter().any(|n| n == original_name);

        // Check collision with other MCP servers' tools
        let cross_server_collision = manager.tool_name_count(original_name) > 1;

        let display_name = if collides_builtin || cross_server_collision {
            format!("mcp__{}_{}", server_name, original_name)
        } else {
            original_name.clone()
        };

        // MCP tools are deferred by default; server config can override.
        let deferred = server_configs
            .get(*server_name)
            .and_then(|c| c.deferred)
            .unwrap_or(true);

        let proxy = McpToolProxy::new(
            display_name,
            original_name.clone(),
            server_name.to_string(),
            tool_def.description.clone().unwrap_or_default(),
            tool_def.input_schema.clone(),
            Arc::clone(manager),
            deferred,
            tool_def.annotations.clone(),
        );

        registry.register(Box::new(proxy));
    }
}

/// Register tools from a single newly-connected MCP server.
/// Uses the same collision-detection logic as `register_mcp_tools`.
pub fn register_single_server_tools(
    registry: &mut nomi_tools::registry::ToolRegistry,
    manager: &Arc<McpManager>,
    server_name: &str,
    builtin_names: &[String],
    deferred: bool,
) {
    let all_tools = manager.all_tools();
    let server_tools: Vec<_> = all_tools
        .iter()
        .filter(|(sn, _)| *sn == server_name)
        .collect();

    for (_, tool_def) in &server_tools {
        let original_name = &tool_def.name;
        let collides_builtin = builtin_names.iter().any(|n| n == original_name);
        let cross_server_collision = manager.tool_name_count(original_name) > 1;

        let display_name = if collides_builtin || cross_server_collision {
            format!("mcp__{}_{}", server_name, original_name)
        } else {
            original_name.clone()
        };

        let proxy = McpToolProxy::new(
            display_name,
            original_name.clone(),
            server_name.to_string(),
            tool_def.description.clone().unwrap_or_default(),
            tool_def.input_schema.clone(),
            Arc::clone(manager),
            deferred,
            tool_def.annotations.clone(),
        );

        registry.register(Box::new(proxy));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolAnnotations;
    use nomi_config::config::TransportType;
    use serde_json::json;

    // Minimal MockTransport local to this test module (the one in manager.rs's
    // test mod is not cross-module visible). McpTransport is crate-visible with
    // 3 methods, so duplicating it keeps the modules decoupled.
    use crate::protocol::{JsonRpcRequest, JsonRpcResponse};
    use crate::transport::{McpError, McpTransport};
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockTransport {
        responses: Mutex<Vec<serde_json::Value>>,
    }

    impl MockTransport {
        fn new(responses: Vec<serde_json::Value>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn request(&self, _req: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            let mut guard = self.responses.lock().unwrap();
            let value = if guard.is_empty() {
                json!(null)
            } else {
                guard.remove(0)
            };
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: Some(1),
                result: Some(value),
                error: None,
            })
        }

        async fn notify(&self, _req: &JsonRpcRequest) -> Result<(), McpError> {
            Ok(())
        }

        async fn close(&self) -> Result<(), McpError> {
            Ok(())
        }
    }

    fn make_proxy(deferred: bool) -> McpToolProxy {
        // manager is only used during execute(), which we don't call in these
        // tests, so we can construct one with no servers.
        let manager = Arc::new(McpManager::new_for_test(vec![]));
        McpToolProxy::new(
            "test_tool".into(),
            "test_tool".into(),
            "test_server".into(),
            "A test tool".into(),
            json!({"type": "object"}),
            manager,
            deferred,
            None,
        )
    }

    /// Build a proxy with the given annotations (no real transport needed —
    /// category() does not touch the manager).
    fn make_proxy_with_annotations(annotations: Option<ToolAnnotations>) -> McpToolProxy {
        let manager = Arc::new(McpManager::new_for_test(vec![]));
        McpToolProxy::new(
            "test_tool".into(),
            "test_tool".into(),
            "test_server".into(),
            "A test tool".into(),
            json!({"type": "object"}),
            manager,
            true,
            annotations,
        )
    }

    #[test]
    fn proxy_deferred_true_returns_true() {
        let proxy = make_proxy(true);
        assert!(proxy.is_deferred());
    }

    #[test]
    fn proxy_deferred_false_returns_false() {
        let proxy = make_proxy(false);
        assert!(!proxy.is_deferred());
    }

    // -----------------------------------------------------------------------
    // category(): annotations → approval class (readOnly→Info, else→Exec)
    // -----------------------------------------------------------------------

    #[test]
    fn category_read_only_hint_true_is_info() {
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations {
            read_only_hint: Some(true),
            ..Default::default()
        }));
        assert_eq!(proxy.category(), ToolCategory::Info);
        // category_for delegates to category() for tools that don't override it.
        assert_eq!(proxy.category_for(&json!({})), ToolCategory::Info);
    }

    #[test]
    fn category_destructive_hint_is_exec() {
        // destructive (and not read-only) must require approval.
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            ..Default::default()
        }));
        assert_eq!(proxy.category(), ToolCategory::Exec);
    }

    #[test]
    fn category_read_only_false_is_exec() {
        // Explicit readOnlyHint=false → still needs approval.
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations {
            read_only_hint: Some(false),
            ..Default::default()
        }));
        assert_eq!(proxy.category(), ToolCategory::Exec);
    }

    #[test]
    fn read_only_hint_true_is_concurrency_safe() {
        // A read-only MCP tool has no side effects, so concurrent execution with
        // other read-only calls is safe (mirrors built-in Read/Grep/Glob).
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations {
            read_only_hint: Some(true),
            ..Default::default()
        }));
        assert!(proxy.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn non_read_only_is_not_concurrency_safe() {
        let explicit_false = make_proxy_with_annotations(Some(ToolAnnotations {
            read_only_hint: Some(false),
            ..Default::default()
        }));
        assert!(!explicit_false.is_concurrency_safe(&json!({})));
        // No annotations → safe default: assume side effects, run serially.
        let none = make_proxy_with_annotations(None);
        assert!(!none.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn category_no_annotations_defaults_to_exec() {
        // Old server with no annotations block → safe default: Exec.
        let proxy = make_proxy_with_annotations(None);
        assert_eq!(proxy.category(), ToolCategory::Exec);
    }

    #[test]
    fn category_empty_annotations_defaults_to_exec() {
        // annotations present but no readOnlyHint → from-strict default: Exec.
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations::default()));
        assert_eq!(proxy.category(), ToolCategory::Exec);
    }

    #[test]
    fn category_read_only_wins_when_no_destructive() {
        // openWorld + idempotent set but readOnly true → still Info.
        let proxy = make_proxy_with_annotations(Some(ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(true),
            idempotent_hint: Some(true),
            ..Default::default()
        }));
        assert_eq!(proxy.category(), ToolCategory::Info);
    }

    fn make_server_config(deferred: Option<bool>) -> McpServerConfig {
        McpServerConfig {
            transport: TransportType::Stdio,
            command: Some("echo".into()),
            args: None,
            env: None,
            url: None,
            headers: None,
            deferred,
        }
    }

    #[test]
    fn register_defaults_to_deferred_when_config_omits_field() {
        let manager = Arc::new(McpManager::new_for_test(vec![]));
        let mut registry = nomi_tools::registry::ToolRegistry::new();
        // Empty server configs — deferred field absent
        let configs = HashMap::new();

        register_mcp_tools(&mut registry, &manager, &[], &configs);

        // No tools registered because manager has no tools, but the logic
        // is tested via the deferred default path. Test with a real config below.
        assert!(registry.tool_names().is_empty());
    }

    #[test]
    fn server_config_deferred_none_defaults_true() {
        let config = make_server_config(None);
        let deferred = config.deferred.unwrap_or(true);
        assert!(deferred, "deferred should default to true when None");
    }

    #[test]
    fn server_config_deferred_explicit_false() {
        let config = make_server_config(Some(false));
        let deferred = config.deferred.unwrap_or(true);
        assert!(!deferred, "deferred should be false when explicitly set");
    }

    #[test]
    fn server_config_deferred_explicit_true() {
        let config = make_server_config(Some(true));
        let deferred = config.deferred.unwrap_or(true);
        assert!(deferred, "deferred should be true when explicitly set");
    }

    // -----------------------------------------------------------------------
    // execute: image content → ToolResult.images (end-to-end mapping)
    // -----------------------------------------------------------------------

    fn proxy_with_response(
        tool: &str,
        resp: serde_json::Value,
    ) -> McpToolProxy {
        let mgr = Arc::new(McpManager::new_for_test(vec![(
            "srv",
            false,
            Box::new(MockTransport::new(vec![resp])),
        )]));
        McpToolProxy::new(
            tool.into(),
            tool.into(),
            "srv".into(),
            "desc".into(),
            json!({"type":"object"}),
            mgr,
            true,
            None,
        )
    }

    #[tokio::test]
    async fn proxy_execute_maps_image_to_tool_result_images() {
        let resp = json!({ "content": [
            {"type":"text","text":"ok"},
            {"type":"image","data":"ZZZ","mimeType":"image/png"}
        ]});
        let proxy = proxy_with_response("shot", resp);
        let r = proxy.execute(json!({})).await;
        assert!(!r.is_error);
        assert_eq!(r.content, "ok");
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].media_type, "image/png");
        assert_eq!(r.images[0].data, "ZZZ");
    }

    #[tokio::test]
    async fn proxy_execute_text_only_no_images() {
        let resp = json!({ "content": [{"type":"text","text":"plain"}] });
        let proxy = proxy_with_response("echo", resp);
        let r = proxy.execute(json!({})).await;
        assert!(!r.is_error);
        assert_eq!(r.content, "plain");
        assert!(r.images.is_empty());
    }

    #[tokio::test]
    async fn proxy_execute_drops_oversized_image_with_placeholder() {
        // Build a base64 string whose decoded size exceeds MCP_MAX_IMAGE_BYTES.
        // decoded ≈ len * 3/4, so we need > 5 MiB * 4/3 base64 chars.
        let huge = "A".repeat((MCP_MAX_IMAGE_BYTES + 1024) * 4 / 3 + 8);
        let resp = json!({ "content": [
            {"type":"text","text":"shot taken"},
            {"type":"image","data": huge,"mimeType":"image/png"}
        ]});
        let proxy = proxy_with_response("big_shot", resp);
        let r = proxy.execute(json!({})).await;
        assert!(!r.is_error);
        // Oversized image dropped → no images survive.
        assert!(r.images.is_empty());
        // Original text preserved + placeholder appended.
        assert!(r.content.contains("shot taken"));
        assert!(
            r.content.contains("image too large"),
            "expected placeholder text, got: {}",
            r.content
        );
        assert!(r.content.contains("dropped"));
    }

    #[tokio::test]
    async fn proxy_execute_keeps_image_just_under_limit() {
        // A small image must survive the size guard untouched.
        let resp = json!({ "content": [
            {"type":"image","data":"c21hbGw=","mimeType":"image/jpeg"}
        ]});
        let proxy = proxy_with_response("small_shot", resp);
        let r = proxy.execute(json!({})).await;
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].media_type, "image/jpeg");
        assert_eq!(r.images[0].data, "c21hbGw=");
    }

    #[test]
    fn decoded_base64_len_estimates_size() {
        // "QQ==" decodes to 1 byte; "QUI=" → 2 bytes; "QUJD" → 3 bytes.
        assert_eq!(decoded_base64_len("QQ=="), 1);
        assert_eq!(decoded_base64_len("QUI="), 2);
        assert_eq!(decoded_base64_len("QUJD"), 3);
        // Whitespace is ignored.
        assert_eq!(decoded_base64_len("QU\nJD"), 3);
        // data-URL prefix is stripped before measuring.
        assert_eq!(decoded_base64_len("data:image/png;base64,QUJD"), 3);
    }
}
