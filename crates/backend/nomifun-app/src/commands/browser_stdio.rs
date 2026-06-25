//! `nomicore mcp-browser-stdio` subcommand: an MCP stdio server exposing the
//! browser-use capability as DISCRETE tools (navigate / observe / click / type /
//! …) for external ACP agents (codex / claude / gemini).
//!
//! It is a thin facade over the in-tree `nomi_browser::BrowserTool` — every tool
//! here translates to a `BrowserTool` action and forwards the result — so codex
//! gets the exact same self-hosted-CDP automation (own Rust CDP engine + the
//! vendored Playwright InjectedScript aria tree, multi-frame / shadow-DOM stitch,
//! `observe`→`[ref]`→act loop) that the nomi engine gets, with zero duplicated
//! logic. Injected when the `browser-use` feature is built (see `BrowserMcpConfig`,
//! P4-2 wiring); compiled only with that feature so headless/web builds never
//! pull the browser engine / Chromium stack.
//!
//! Discrete tools (vs the nomi engine's single `Browser(action)` tool) match the
//! shape models are trained on, which raises adoption — the same form picked for
//! the computer-use bridge.
//!
//! # R1 — no orchestration approval layer here (P4 decision D1)
//!
//! This bridge is a short-lived process spawned by the ACP CLI; it has NO
//! NomiFun orchestration / supervision layer. The `BrowserTool` redline gate runs
//! with default policy (`session_bypasses_approval() == false` → normal-session
//! semantics: it does NOT hard-deny irreversible actions, leaving them to the
//! orchestration approval the nomi engine path provides). For the ACP path the
//! human-in-the-loop is the **ACP CLI's own per-tool approval UI** (claude / codex
//! prompt before each tool call). A stricter nomi-side hard-deny for the bridge is
//! deferred to P6.
//!
//! # R2 — no per-pet session context (P4 decision D2)
//!
//! Constructed as `BrowserTool::new(&BrowserConfig::default())` — STATELESS, with
//! NO env-borne session context (we deliberately do not pass machine-bound keys
//! across the env boundary; attack-surface reduction). Consequences, all fail-safe
//! by the facade's defaults:
//! - `secret:NAME` fails CLOSED (no secret source → empty store → blocked; the
//!   credential never leaks into the conversation).
//! - `evaluate` is gated off (full-power not enabled) → reported Unsupported.
//! - downloads / screenshots land in the data-dir sandbox (still isolated), not a
//!   per-pet workspace.
//! - the egress firewall (IP block + cross-origin POST gate) is on by default.
//!
//! Per-pet credentials / workspace / persistent login stay on the nomi engine
//! path (the desktop companion uses it).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use nomi_browser::tool::BrowserTool;
use nomi_config::config::BrowserConfig;
use nomi_tools::Tool;
use nomi_types::tool::ToolResult;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::Deserialize;
use serde_json::{Value, json};

/// Resolve the bundled Chrome-for-Testing resource directory.
///
/// Convention: the desktop build places Chrome-for-Testing at
/// `<app_resource_dir>/chrome-for-testing/chrome-<platform>/...`.
/// This function computes `<app_resource_dir>/chrome-for-testing` from
/// `current_exe().canonicalize().parent()` (mirrors `services.rs` resource-dir
/// resolution) and returns `Some(dir)` ONLY if the directory exists on disk —
/// so non-packaged / dev runs get `None` (unchanged behavior: env > data_dir > download).
///
/// The stdio subprocess is the SAME binary as the desktop app, so current_exe-relative
/// resolution is correct for both the stdio and gateway paths.
pub(crate) fn bundled_chrome_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe()
        .ok()?
        .canonicalize()
        .ok()?
        .parent()?
        .join("chrome-for-testing");
    dir.is_dir().then_some(dir)
}

pub async fn run_browser_stdio() -> ExitCode {
    eprintln!("[mcp-browser-stdio] Started OK.");

    // R1/R2: stateless, default config — no session context. See module docs.
    // PKG-1: inject bundled Chrome dir so packaged builds prefer it over download.
    let server = BrowserStdioServer {
        tool: Arc::new(BrowserTool::new(&BrowserConfig::default()).bundled_dir(bundled_chrome_dir())),
    };

    let transport = transport::io::stdio();
    match server.serve(transport).await {
        Ok(peer) => {
            eprintln!("[mcp-browser-stdio] MCP session started, waiting for completion...");
            if let Err(e) = peer.waiting().await {
                eprintln!("[mcp-browser-stdio] Session ended with error: {e}");
            } else {
                eprintln!("[mcp-browser-stdio] Session ended normally");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[mcp-browser-stdio] Failed to start MCP server: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone)]
struct BrowserStdioServer {
    tool: Arc<BrowserTool>,
}

/// Translate a `BrowserTool` `ToolResult` into an MCP `CallToolResult`: the text
/// content plus any images (e.g. the `screenshot` PNG) as image blocks. Identical
/// to the computer bridge's `to_mcp` — the multimodal path is shared.
fn to_mcp(tr: ToolResult) -> CallToolResult {
    let mut content = vec![Content::text(tr.content)];
    for img in tr.images {
        content.push(Content::image(img.data, img.media_type));
    }
    if tr.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

// ---- tool parameter structs --------------------------------------------

#[derive(Deserialize, schemars::JsonSchema)]
struct NavigateParams {
    /// URL to load, e.g. "https://example.com".
    url: String,
    /// Open the URL in a new tab instead of the current one (default false).
    #[serde(default)]
    new_tab: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ObserveParams {
    /// Max accessibility-tree depth to serialize (default 12 — lower it for huge pages).
    #[serde(default)]
    max_depth: Option<u32>,
    /// Use the injected-side diff for this observe (default true).
    #[serde(default)]
    diff: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RefParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TypeParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
    /// Text to type. Use "secret:NAME" to inject a stored credential bound to the
    /// current origin WITHOUT the value passing through this conversation.
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SetValueParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
    /// Value to set on the control. Also accepts "secret:NAME".
    value: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SelectOptionParams {
    /// A `[ref=f<seq>e<n>]` <select> element from the most recent `observe`.
    r#ref: String,
    /// Option values/labels to select.
    options: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PressKeyParams {
    /// Key or combo to press, e.g. "Enter", "Control+a", "Tab".
    keys: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ScrollParams {
    /// Scroll direction: up, down, left, or right.
    direction: String,
    /// Scroll amount; optional, engine default applies.
    #[serde(default)]
    amount: Option<f64>,
    /// Optional element `[ref]` to scroll into view; omit to scroll the viewport.
    #[serde(default)]
    r#ref: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ScrollToTextParams {
    /// Text to scroll to.
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchPageParams {
    /// Text to grep the page for.
    query: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct FindElementsParams {
    /// CSS selector to find elements by.
    selector: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WaitParams {
    /// Milliseconds to wait.
    ms: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WaitForParams {
    /// Condition kind: "url_contains", "text_visible", or "ref_actionable".
    condition: String,
    /// Paired with url_contains / text_visible conditions.
    #[serde(default)]
    text: Option<String>,
    /// Paired with the ref_actionable condition: a `[ref]` from the latest `observe`.
    #[serde(default)]
    r#ref: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct UploadFileParams {
    /// A `[ref=f<seq>e<n>]` file-input element from the most recent `observe`.
    r#ref: String,
    /// File path, or array of file paths, to set on the file input.
    file_path: Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct DownloadParams {
    /// URL to download into the sandboxed downloads folder.
    url: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ExtractParams {
    /// JSON schema describing the fields to extract from the page (optional — the
    /// page is returned as a structured, redacted representation to extract against).
    #[serde(default)]
    schema: Option<Value>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TabIdParams {
    /// Tab id from the `tabs` action.
    tab_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct UrlParams {
    /// URL to load, e.g. "https://example.com".
    url: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EvaluateParams {
    /// Script to evaluate in the page. Disabled unless full-power mode is enabled —
    /// on this stateless ACP bridge it is gated off and returns Unsupported.
    script: String,
}

#[tool_router(server_handler)]
impl BrowserStdioServer {
    async fn run(&self, input: Value) -> CallToolResult {
        to_mcp(self.tool.execute(input).await)
    }

    // ---- read-only -----------------------------------------------------

    #[tool(
        name = "navigate",
        description = "Load a `url` in the browser (optionally in a `new_tab`). Do this first, then `observe` to read the page, then act on a `[ref]`."
    )]
    async fn navigate(&self, Parameters(p): Parameters<NavigateParams>) -> CallToolResult {
        self.run(json!({"action": "navigate", "url": p.url, "new_tab": p.new_tab})).await
    }

    #[tool(
        name = "observe",
        description = "Read the current page's accessibility tree as YAML with numbered `[ref=f<seq>e<n>]` elements. Do this AFTER navigation and act on a control with `click`/`type` and its `[ref]`. Re-run after any navigation or UI change — a `[ref]` is only valid for the latest observe. Optional `max_depth` (default 12) and `diff` (default true)."
    )]
    async fn observe(&self, Parameters(p): Parameters<ObserveParams>) -> CallToolResult {
        self.run(json!({"action": "observe", "max_depth": p.max_depth, "diff": p.diff})).await
    }

    #[tool(
        name = "screenshot",
        description = "Capture the current page as a PNG when you need raw pixels."
    )]
    async fn screenshot(&self) -> CallToolResult {
        self.run(json!({"action": "screenshot"})).await
    }

    #[tool(
        name = "capabilities",
        description = "Report which browser features are available in this session (e.g. whether `evaluate`/full-power is enabled)."
    )]
    async fn capabilities(&self) -> CallToolResult {
        self.run(json!({"action": "capabilities"})).await
    }

    #[tool(name = "get_page_text", description = "Return the visible text content of the current page.")]
    async fn get_page_text(&self) -> CallToolResult {
        self.run(json!({"action": "get_page_text"})).await
    }

    #[tool(name = "search_page", description = "Grep the current page for `query` text and report matches.")]
    async fn search_page(&self, Parameters(p): Parameters<SearchPageParams>) -> CallToolResult {
        self.run(json!({"action": "search_page", "query": p.query})).await
    }

    #[tool(name = "find_elements", description = "Find elements on the page matching a CSS `selector`.")]
    async fn find_elements(&self, Parameters(p): Parameters<FindElementsParams>) -> CallToolResult {
        self.run(json!({"action": "find_elements", "selector": p.selector})).await
    }

    #[tool(
        name = "get_dropdown_options",
        description = "List the options of a `<select>` dropdown element by its `[ref]` from the latest `observe`."
    )]
    async fn get_dropdown_options(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "get_dropdown_options", "ref": p.r#ref})).await
    }

    #[tool(name = "cursor", description = "List clickable (pointer-cursor) elements on the page.")]
    async fn cursor(&self) -> CallToolResult {
        self.run(json!({"action": "cursor"})).await
    }

    #[tool(name = "tabs", description = "List the open browser tabs with their ids.")]
    async fn tabs(&self) -> CallToolResult {
        self.run(json!({"action": "tabs"})).await
    }

    #[tool(name = "wait", description = "Pause for `ms` milliseconds to let the page settle.")]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        self.run(json!({"action": "wait", "ms": p.ms})).await
    }

    #[tool(
        name = "wait_for",
        description = "Wait until a `condition` holds. Conditions: \"url_contains\"/\"text_visible\" (pair with `text`), \"ref_actionable\" (pair with `ref`)."
    )]
    async fn wait_for(&self, Parameters(p): Parameters<WaitForParams>) -> CallToolResult {
        self.run(json!({
            "action": "wait_for",
            "condition": p.condition,
            "text": p.text,
            "ref": p.r#ref
        }))
        .await
    }

    // ---- write / interaction -------------------------------------------

    #[tool(
        name = "click",
        description = "Click the element with the given `ref` from the latest `observe`. May be IRREVERSIBLE (submit / pay / delete / send) — the ACP CLI's per-tool approval is the human gate."
    )]
    async fn click(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "click", "ref": p.r#ref})).await
    }

    #[tool(name = "hover", description = "Hover the pointer over the element with the given `ref`.")]
    async fn hover(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "hover", "ref": p.r#ref})).await
    }

    #[tool(
        name = "type",
        description = "Type `text` into the element with the given `ref`. Use \"secret:NAME\" to inject a stored credential bound to the current origin without the value passing through this conversation (fails closed on this bridge if no secret store is configured)."
    )]
    async fn type_text(&self, Parameters(p): Parameters<TypeParams>) -> CallToolResult {
        self.run(json!({"action": "type", "ref": p.r#ref, "text": p.text})).await
    }

    #[tool(
        name = "set_value",
        description = "Set the `value` of the control with the given `ref` (good for text fields). Also accepts \"secret:NAME\"."
    )]
    async fn set_value(&self, Parameters(p): Parameters<SetValueParams>) -> CallToolResult {
        self.run(json!({"action": "set_value", "ref": p.r#ref, "value": p.value})).await
    }

    #[tool(
        name = "select_option",
        description = "Select one or more `options` (values/labels) in the `<select>` element with the given `ref`."
    )]
    async fn select_option(&self, Parameters(p): Parameters<SelectOptionParams>) -> CallToolResult {
        self.run(json!({"action": "select_option", "ref": p.r#ref, "options": p.options})).await
    }

    #[tool(
        name = "press_key",
        description = "Press a key or combo, e.g. \"Enter\", \"Control+a\", \"Tab\". An Enter inside a form may submit it (IRREVERSIBLE)."
    )]
    async fn press_key(&self, Parameters(p): Parameters<PressKeyParams>) -> CallToolResult {
        self.run(json!({"action": "press_key", "keys": p.keys})).await
    }

    #[tool(
        name = "scroll",
        description = "Scroll in `direction` (up/down/left/right) by an optional `amount`. Pass a `ref` to scroll that element into view instead of the viewport."
    )]
    async fn scroll(&self, Parameters(p): Parameters<ScrollParams>) -> CallToolResult {
        self.run(json!({
            "action": "scroll",
            "direction": p.direction,
            "amount": p.amount,
            "ref": p.r#ref
        }))
        .await
    }

    #[tool(name = "scroll_to_text", description = "Scroll until the given `text` is in view.")]
    async fn scroll_to_text(&self, Parameters(p): Parameters<ScrollToTextParams>) -> CallToolResult {
        self.run(json!({"action": "scroll_to_text", "text": p.text})).await
    }

    #[tool(
        name = "upload_file",
        description = "Set `file_path` (a path string or array of paths) on the file-input element with the given `ref`."
    )]
    async fn upload_file(&self, Parameters(p): Parameters<UploadFileParams>) -> CallToolResult {
        self.run(json!({"action": "upload_file", "ref": p.r#ref, "file_path": p.file_path})).await
    }

    #[tool(
        name = "download",
        description = "Download `url` into the sandboxed downloads folder (not opened)."
    )]
    async fn download(&self, Parameters(p): Parameters<DownloadParams>) -> CallToolResult {
        self.run(json!({"action": "download", "url": p.url})).await
    }

    #[tool(
        name = "save_as_pdf",
        description = "Save the current page as a PDF into the sandboxed downloads folder."
    )]
    async fn save_as_pdf(&self) -> CallToolResult {
        self.run(json!({"action": "save_as_pdf"})).await
    }

    #[tool(
        name = "extract",
        description = "Extract structured data from the page against an optional JSON `schema` (the page is returned as a structured, redacted representation)."
    )]
    async fn extract(&self, Parameters(p): Parameters<ExtractParams>) -> CallToolResult {
        self.run(json!({"action": "extract", "schema": p.schema})).await
    }

    #[tool(name = "switch_frame", description = "Switch into the iframe element with the given `ref`.")]
    async fn switch_frame(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "switch_frame", "ref": p.r#ref})).await
    }

    #[tool(name = "switch_tab", description = "Switch to the tab with the given `tab_id` (from `tabs`).")]
    async fn switch_tab(&self, Parameters(p): Parameters<TabIdParams>) -> CallToolResult {
        self.run(json!({"action": "switch_tab", "tab_id": p.tab_id})).await
    }

    #[tool(name = "close_tab", description = "Close the tab with the given `tab_id` (from `tabs`).")]
    async fn close_tab(&self, Parameters(p): Parameters<TabIdParams>) -> CallToolResult {
        self.run(json!({"action": "close_tab", "tab_id": p.tab_id})).await
    }

    #[tool(name = "open_link_new_tab", description = "Open `url` in a new tab.")]
    async fn open_link_new_tab(&self, Parameters(p): Parameters<UrlParams>) -> CallToolResult {
        self.run(json!({"action": "open_link_new_tab", "url": p.url})).await
    }

    #[tool(name = "back", description = "Navigate back in the browser history.")]
    async fn back(&self) -> CallToolResult {
        self.run(json!({"action": "back"})).await
    }

    #[tool(name = "forward", description = "Navigate forward in the browser history.")]
    async fn forward(&self) -> CallToolResult {
        self.run(json!({"action": "forward"})).await
    }

    #[tool(
        name = "reload",
        description = "Reload the current page. Reloading a page that submitted a form re-submits it (IRREVERSIBLE)."
    )]
    async fn reload(&self) -> CallToolResult {
        self.run(json!({"action": "reload"})).await
    }

    #[tool(
        name = "evaluate",
        description = "Evaluate a `script` in the page. Gated — disabled unless full-power mode is on; on this stateless ACP bridge it is OFF and returns Unsupported."
    )]
    async fn evaluate(&self, Parameters(p): Parameters<EvaluateParams>) -> CallToolResult {
        self.run(json!({"action": "evaluate", "script": p.script})).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_schemas_have_properties_field() {
        let router = BrowserStdioServer::tool_router();
        let tools = router.list_all();
        assert!(!tools.is_empty(), "browser bridge must register tools");
        for tool in &tools {
            assert!(
                tool.input_schema.contains_key("properties"),
                "Tool '{}' schema missing 'properties' field: {:?}. OpenAI API rejects schemas without it.",
                tool.name,
                tool.input_schema,
            );
        }
    }

    #[test]
    fn registers_expected_discrete_tools() {
        let router = BrowserStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        for expected in [
            // read-only
            "navigate", "observe", "screenshot", "capabilities", "get_page_text", "search_page",
            "find_elements", "get_dropdown_options", "cursor", "tabs", "wait", "wait_for",
            // write / interaction
            "click", "hover", "type", "set_value", "select_option", "press_key", "scroll",
            "scroll_to_text", "upload_file", "download", "save_as_pdf", "extract", "switch_frame",
            "switch_tab", "close_tab", "open_link_new_tab", "back", "forward", "reload", "evaluate",
        ] {
            assert!(names.contains(&expected.to_string()), "missing tool {expected}; got {names:?}");
        }
        // The full BrowserTool action surface (tool.rs input_schema enum) is 32
        // discrete tools — guard the count so a dropped/extra tool is caught.
        assert_eq!(names.len(), 32, "expected 32 discrete browser tools; got {}: {names:?}", names.len());
    }
}
