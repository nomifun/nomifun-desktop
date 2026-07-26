//! `nomicore mcp-browser-stdio`: discrete browser tools for ACP agents.
//!
//! The child is deliberately only an authenticated stdio-to-loopback proxy. It
//! cannot create a browser engine, profile, or Chromium process. Every call is
//! forwarded under a main-process-signed user/conversation/runtime capability
//! to the singleton `BrowserSessionHub`.
//!
//! The bootstrap contains no CDP endpoint, Chromium debugging port, profile
//! path, cookie, or storage value. Tool and browser-operation allowlists fail
//! closed, and arbitrary page-script evaluation is not granted by default.

use std::process::ExitCode;

use nomifun_api_types::{
    BROWSER_CAPABILITY_DOMAIN, BROWSER_MCP_TOOL_NAMES, BrowserCapabilityClaims,
    BrowserCapabilityOperation, BrowserCapabilityScope, BrowserCapabilitySurface,
    BrowserMcpConfig,
    browser_tool_operation,
};
use nomifun_common::{LoopbackCapabilityError, LoopbackSessionKind};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::de;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use super::stdio_common::{ForwardToolOutcome, into_mcp_tool_result};

/// Resolve the bundled Chrome-for-Testing resource directory.
///
/// Convention: the desktop build places Chrome-for-Testing at
/// `<app_resource_dir>/chrome-for-testing/chrome-<platform>/...`.
/// This function computes `<app_resource_dir>/chrome-for-testing` from
/// `current_exe().canonicalize().parent()` (mirrors `services.rs` resource-dir
/// resolution) and returns `Some(dir)` ONLY if the directory exists on disk —
/// so non-packaged / dev runs get `None` (unchanged behavior: env > data_dir > download).
///
/// The application composition root uses this to configure the sole managed
/// Host factory before any ACP proxy can open a Lane.
pub async fn run_browser_stdio() -> ExitCode {
    let client = match super::stdio_common::ScopedBridgeClient::from_env(
        BrowserMcpConfig::ENV_CAPABILITY,
        BROWSER_CAPABILITY_DOMAIN,
        "mcp-browser-stdio",
        validate_browser_claims,
    )
    .await
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[mcp-browser-stdio] ERROR: {error}");
            return ExitCode::from(1);
        }
    };
    let claims = client.access().await.expect("startup renewal succeeded").claims;
    eprintln!(
        "[mcp-browser-stdio] Started OK. SESSION={}, RUNTIME={}, EXP={}",
        claims.session.session_id,
        claims.scope.runtime_instance_id,
        claims.expires_at_unix_secs,
    );

    let lifecycle = client.clone();
    let server = BrowserStdioServer { client };

    let transport = transport::io::stdio();
    let exit = match server.serve(transport).await {
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
    };
    lifecycle.revoke().await;
    exit
}

#[derive(Clone)]
struct BrowserStdioServer {
    client: super::stdio_common::ScopedBridgeClient<BrowserCapabilityScope>,
}

/// Validate the immutable ACP audience, runtime scope, and two-level
/// tool/operation allowlist before exposing any MCP method.
fn validate_browser_claims(
    claims: &BrowserCapabilityClaims,
) -> Result<(), LoopbackCapabilityError> {
    claims.validate_renewable_shape()?;
    claims.scope.validate(&claims.session)?;
    if claims.session.kind != LoopbackSessionKind::Conversation
        || !matches!(claims.scope.surface, BrowserCapabilitySurface::Acp)
        || !claims
            .scope
            .allows(BrowserCapabilityOperation::Manage)
        || claims.allowed_tools.iter().any(|tool| {
            !BROWSER_MCP_TOOL_NAMES.contains(&tool.as_str())
                || browser_tool_operation(tool)
                    .is_none_or(|operation| !claims.scope.allows(operation))
        })
    {
        return Err(LoopbackCapabilityError::InvalidIdentity);
    }
    Ok(())
}

// ---- tool parameter structs --------------------------------------------

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LaneParams {
    /// Optional owner-scoped Lane handle returned by browser_open,
    /// browser_fork, or browser_list. Existing tools use the default Lane when
    /// omitted.
    #[serde(default)]
    lane_id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NavigateParams {
    /// URL to load, e.g. "https://example.com".
    url: String,
    /// Open the URL in a new tab instead of the current one (default false).
    #[serde(default)]
    new_tab: Option<bool>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObserveParams {
    /// Max accessibility-tree depth to serialize (default 12 — lower it for huge pages).
    #[serde(default)]
    max_depth: Option<u32>,
    /// Use the injected-side diff for this observe (default true).
    #[serde(default)]
    diff: Option<bool>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RefParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TypeParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
    /// Text to type. Use "secret:NAME" to inject a stored credential bound to the
    /// current origin WITHOUT the value passing through this conversation.
    text: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetValueParams {
    /// A `[ref=f<seq>e<n>]` element from the most recent `observe`.
    r#ref: String,
    /// Value to set on the control. Also accepts "secret:NAME".
    value: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectOptionParams {
    /// A `[ref=f<seq>e<n>]` <select> element from the most recent `observe`.
    r#ref: String,
    /// Option values/labels to select.
    options: Vec<String>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PressKeyParams {
    /// Key or combo to press, e.g. "Enter", "Control+a", "Tab".
    keys: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScrollParams {
    /// Scroll direction: up, down, left, or right.
    direction: String,
    /// Scroll amount; optional, engine default applies.
    #[serde(default)]
    amount: Option<f64>,
    /// Optional element `[ref]` to scroll into view; omit to scroll the viewport.
    #[serde(default)]
    r#ref: Option<String>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScrollToTextParams {
    /// Text to scroll to.
    text: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchPageParams {
    /// Text to grep the page for.
    query: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindElementsParams {
    /// CSS selector to find elements by.
    selector: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NetworkLogParams {
    /// Include request and response bodies. Disabled by default because bodies
    /// are large and may contain secrets.
    #[serde(default)]
    include_bodies: Option<bool>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitParams {
    /// Milliseconds to wait.
    ms: u64,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitForParams {
    /// Condition kind: "url_contains", "text_visible", or "ref_actionable".
    condition: String,
    /// Paired with url_contains / text_visible conditions.
    #[serde(default)]
    text: Option<String>,
    /// Paired with the ref_actionable condition: a `[ref]` from the latest `observe`.
    #[serde(default)]
    r#ref: Option<String>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct UploadFileParams {
    /// A `[ref=f<seq>e<n>]` file-input element from the most recent `observe`.
    r#ref: String,
    /// File path, or array of file paths, to set on the file input.
    file_path: Value,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadParams {
    /// URL to download into the sandboxed downloads folder.
    url: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExtractParams {
    /// JSON schema describing the fields to extract from the page (optional — the
    /// page is returned as a structured, redacted representation to extract against).
    #[serde(default)]
    schema: Option<Value>,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TabIdParams {
    /// Tab id from the `tabs` action.
    tab_id: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct UrlParams {
    /// URL to load, e.g. "https://example.com".
    url: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EvaluateParams {
    /// Script to evaluate in the page. The default scoped ACP capability does
    /// not expose this tool.
    script: String,
    #[serde(flatten)]
    lane: LaneParams,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserOpenParams {
    /// Optional short logical name. browser_open is idempotent for a name.
    #[serde(default)]
    lane_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum BrowserCrawlConcurrency {
    Auto(String),
    Fixed(u8),
}

impl<'de> Deserialize<'de> for BrowserCrawlConcurrency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::String(value) if value == "auto" => Ok(Self::Auto(value)),
            Value::Number(number) => match number.as_u64() {
                Some(value @ 1..=8) => Ok(Self::Fixed(value as u8)),
                _ => Err(de::Error::custom(
                    "`concurrency` must be \"auto\" or an integer from 1 through 8",
                )),
            },
            _ => Err(de::Error::custom(
                "`concurrency` must be \"auto\" or an integer from 1 through 8",
            )),
        }
    }
}

impl schemars::JsonSchema for BrowserCrawlConcurrency {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BrowserCrawlConcurrency".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Bounded browser_crawl_many concurrency: \"auto\" or an integer from 1 through 8.",
            "oneOf": [
                {"type": "string", "enum": ["auto"]},
                {"type": "integer", "minimum": 1, "maximum": 8}
            ]
        })
    }
}

fn deserialize_optional_browser_crawl_concurrency<'de, D>(
    deserializer: D,
) -> Result<Option<BrowserCrawlConcurrency>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    serde_json::from_value(value)
        .map(Some)
        .map_err(de::Error::custom)
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserCrawlManyParams {
    /// Ordered HTTP(S) inputs. One terminal result is returned per URL.
    urls: Vec<String>,
    /// "auto" or an integer from 1 through 8.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_browser_crawl_concurrency"
    )]
    #[schemars(with = "BrowserCrawlConcurrency")]
    concurrency: Option<BrowserCrawlConcurrency>,
    /// Optional extraction schema; otherwise readable page text is returned.
    #[serde(default)]
    schema: Option<Value>,
}

#[tool_router]
impl BrowserStdioServer {
    async fn run(&self, input: Value) -> CallToolResult {
        let Some(tool) = input.get("action").and_then(Value::as_str) else {
            return into_mcp_tool_result(ForwardToolOutcome::Error(
                "browser proxy request is missing its action".into(),
            ));
        };
        let mut args = input.as_object().cloned().unwrap_or_default();
        args.remove("action");
        let body = json!({
            "tool": tool,
            "args": Value::Object(args),
        });
        into_mcp_tool_result(
            self.client
                .forward_tool_outcome(tool, body, false)
                .await,
        )
    }

    // ---- Lane management ------------------------------------------------

    #[tool(
        name = "browser_open",
        description = "Idempotently open the caller's default or named managed Browser Lane using the trusted host-selected interactive identity. Returns an owner-scoped lane_id."
    )]
    async fn browser_open(
        &self,
        Parameters(p): Parameters<BrowserOpenParams>,
    ) -> CallToolResult {
        self.run(json!({
            "action": "browser_open",
            "lane_name": p.lane_name,
        }))
        .await
    }

    #[tool(
        name = "browser_fork",
        description = "Create or open an additional managed Browser Lane using the trusted host-selected interactive identity and return its short owner-scoped lane_id."
    )]
    async fn browser_fork(
        &self,
        Parameters(p): Parameters<BrowserOpenParams>,
    ) -> CallToolResult {
        self.run(json!({
            "action": "browser_fork",
            "lane_name": p.lane_name,
        }))
        .await
    }

    #[tool(
        name = "browser_list",
        description = "List only the managed Browser Lanes owned by this runtime, including queue, capacity, identity, epoch, and recovery state."
    )]
    async fn browser_list(&self) -> CallToolResult {
        self.run(json!({"action": "browser_list"})).await
    }

    #[tool(
        name = "browser_status",
        description = "Read status for an owner-scoped lane_id, or the default Lane when omitted."
    )]
    async fn browser_status(
        &self,
        Parameters(p): Parameters<LaneParams>,
    ) -> CallToolResult {
        self.run(json!({"action": "browser_status", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "browser_close",
        description = "Close one owner-scoped managed Browser Lane; defaults to the caller's default Lane."
    )]
    async fn browser_close(
        &self,
        Parameters(p): Parameters<LaneParams>,
    ) -> CallToolResult {
        self.run(json!({"action": "browser_close", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "browser_close_all",
        description = "Close every managed Browser Lane owned by this runtime and no other runtime."
    )]
    async fn browser_close_all(&self) -> CallToolResult {
        self.run(json!({"action": "browser_close_all"})).await
    }

    #[tool(
        name = "browser_crawl_many",
        description = "Read and optionally extract an ordered, bounded URL batch using managed Lanes. The trusted host selects the crawl identity policy and owns Lane reuse, concurrency, cancellation, result ordering, and cleanup."
    )]
    async fn browser_crawl_many(
        &self,
        Parameters(p): Parameters<BrowserCrawlManyParams>,
    ) -> CallToolResult {
        let mut input = json!({
            "action": "browser_crawl_many",
            "urls": p.urls,
        });
        if let Some(concurrency) = p.concurrency {
            input["concurrency"] = json!(concurrency);
        }
        if let Some(schema) = p.schema {
            input["schema"] = schema;
        }
        self.run(input).await
    }

    // ---- read-only -----------------------------------------------------

    #[tool(
        name = "navigate",
        description = "Load a `url` in the browser (optionally in a `new_tab`). Do this first, then `observe` to read the page, then act on a `[ref]`."
    )]
    async fn navigate(&self, Parameters(p): Parameters<NavigateParams>) -> CallToolResult {
        self.run(json!({
            "action": "navigate",
            "url": p.url,
            "new_tab": p.new_tab,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "observe",
        description = "Read the current page's accessibility tree as YAML with numbered `[ref=f<seq>e<n>]` elements. Do this AFTER navigation and act on a control with `click`/`type` and its `[ref]`. Re-run after any navigation or UI change — a `[ref]` is only valid for the latest observe. Optional `max_depth` (default 12) and `diff` (default true)."
    )]
    async fn observe(&self, Parameters(p): Parameters<ObserveParams>) -> CallToolResult {
        self.run(json!({
            "action": "observe",
            "max_depth": p.max_depth,
            "diff": p.diff,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "screenshot",
        description = "Capture the current page as a PNG when you need raw pixels."
    )]
    async fn screenshot(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "screenshot", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "capabilities",
        description = "Report which browser features are available in this session (e.g. whether `evaluate`/full-power is enabled)."
    )]
    async fn capabilities(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "capabilities", "lane_id": p.lane_id}))
            .await
    }

    #[tool(name = "get_page_text", description = "Return the visible text content of the current page.")]
    async fn get_page_text(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "get_page_text", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "get_console_logs",
        description = "Return console log, warning, and error messages from the current page."
    )]
    async fn get_console_logs(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "get_console_logs", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "get_page_errors",
        description = "Return uncaught exceptions and error-level messages from the current page."
    )]
    async fn get_page_errors(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "get_page_errors", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "get_network_log",
        description = "Return the current page's network log. Response bodies are excluded unless include_bodies is true."
    )]
    async fn get_network_log(
        &self,
        Parameters(p): Parameters<NetworkLogParams>,
    ) -> CallToolResult {
        self.run(json!({
            "action": "get_network_log",
            "include_bodies": p.include_bodies,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "search_page", description = "Grep the current page for `query` text and report matches.")]
    async fn search_page(&self, Parameters(p): Parameters<SearchPageParams>) -> CallToolResult {
        self.run(json!({
            "action": "search_page",
            "query": p.query,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "find_elements", description = "Find elements on the page matching a CSS `selector`.")]
    async fn find_elements(&self, Parameters(p): Parameters<FindElementsParams>) -> CallToolResult {
        self.run(json!({
            "action": "find_elements",
            "selector": p.selector,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "get_dropdown_options",
        description = "List the options of a `<select>` dropdown element by its `[ref]` from the latest `observe`."
    )]
    async fn get_dropdown_options(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({
            "action": "get_dropdown_options",
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "cursor", description = "List clickable (pointer-cursor) elements on the page.")]
    async fn cursor(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "cursor", "lane_id": p.lane_id}))
            .await
    }

    #[tool(name = "tabs", description = "List the open browser tabs with their ids.")]
    async fn tabs(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "tabs", "lane_id": p.lane_id}))
            .await
    }

    #[tool(name = "wait", description = "Pause for `ms` milliseconds to let the page settle.")]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        self.run(json!({
            "action": "wait",
            "ms": p.ms,
            "lane_id": p.lane.lane_id,
        }))
        .await
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
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    // ---- write / interaction -------------------------------------------

    #[tool(
        name = "click",
        description = "Click the element with the given `ref` from the latest `observe`. May be IRREVERSIBLE (submit / pay / delete / send) — the ACP CLI's per-tool approval is the human gate."
    )]
    async fn click(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({
            "action": "click",
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "hover", description = "Hover the pointer over the element with the given `ref`.")]
    async fn hover(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({
            "action": "hover",
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "type",
        description = "Type `text` into the element with the given `ref`. Use \"secret:NAME\" to inject a stored credential bound to the current origin without the value passing through this conversation (fails closed on this bridge if no secret store is configured)."
    )]
    async fn type_text(&self, Parameters(p): Parameters<TypeParams>) -> CallToolResult {
        self.run(json!({
            "action": "type",
            "ref": p.r#ref,
            "text": p.text,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "set_value",
        description = "Set the `value` of the control with the given `ref` (good for text fields). Also accepts \"secret:NAME\"."
    )]
    async fn set_value(&self, Parameters(p): Parameters<SetValueParams>) -> CallToolResult {
        self.run(json!({
            "action": "set_value",
            "ref": p.r#ref,
            "value": p.value,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "select_option",
        description = "Select one or more `options` (values/labels) in the `<select>` element with the given `ref`."
    )]
    async fn select_option(&self, Parameters(p): Parameters<SelectOptionParams>) -> CallToolResult {
        self.run(json!({
            "action": "select_option",
            "ref": p.r#ref,
            "options": p.options,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "press_key",
        description = "Press a key or combo, e.g. \"Enter\", \"Control+a\", \"Tab\". An Enter inside a form may submit it (IRREVERSIBLE)."
    )]
    async fn press_key(&self, Parameters(p): Parameters<PressKeyParams>) -> CallToolResult {
        self.run(json!({
            "action": "press_key",
            "keys": p.keys,
            "lane_id": p.lane.lane_id,
        }))
        .await
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
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "scroll_to_text", description = "Scroll until the given `text` is in view.")]
    async fn scroll_to_text(&self, Parameters(p): Parameters<ScrollToTextParams>) -> CallToolResult {
        self.run(json!({
            "action": "scroll_to_text",
            "text": p.text,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "upload_file",
        description = "Set `file_path` (a path string or array of paths) on the file-input element with the given `ref`."
    )]
    async fn upload_file(&self, Parameters(p): Parameters<UploadFileParams>) -> CallToolResult {
        self.run(json!({
            "action": "upload_file",
            "ref": p.r#ref,
            "file_path": p.file_path,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "download",
        description = "Download `url` into the sandboxed downloads folder (not opened)."
    )]
    async fn download(&self, Parameters(p): Parameters<DownloadParams>) -> CallToolResult {
        self.run(json!({
            "action": "download",
            "url": p.url,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(
        name = "save_as_pdf",
        description = "Save the current page as a PDF into the sandboxed downloads folder."
    )]
    async fn save_as_pdf(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "save_as_pdf", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "extract",
        description = "Extract structured data from the page against an optional JSON `schema` (the page is returned as a structured, redacted representation)."
    )]
    async fn extract(&self, Parameters(p): Parameters<ExtractParams>) -> CallToolResult {
        self.run(json!({
            "action": "extract",
            "schema": p.schema,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "switch_frame", description = "Switch into the iframe element with the given `ref`.")]
    async fn switch_frame(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({
            "action": "switch_frame",
            "ref": p.r#ref,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "switch_tab", description = "Switch to the tab with the given `tab_id` (from `tabs`).")]
    async fn switch_tab(&self, Parameters(p): Parameters<TabIdParams>) -> CallToolResult {
        self.run(json!({
            "action": "switch_tab",
            "tab_id": p.tab_id,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "close_tab", description = "Close the tab with the given `tab_id` (from `tabs`).")]
    async fn close_tab(&self, Parameters(p): Parameters<TabIdParams>) -> CallToolResult {
        self.run(json!({
            "action": "close_tab",
            "tab_id": p.tab_id,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "open_link_new_tab", description = "Open `url` in a new tab.")]
    async fn open_link_new_tab(&self, Parameters(p): Parameters<UrlParams>) -> CallToolResult {
        self.run(json!({
            "action": "open_link_new_tab",
            "url": p.url,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }

    #[tool(name = "back", description = "Navigate back in the browser history.")]
    async fn back(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "back", "lane_id": p.lane_id}))
            .await
    }

    #[tool(name = "forward", description = "Navigate forward in the browser history.")]
    async fn forward(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "forward", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "reload",
        description = "Reload the current page. Reloading a page that submitted a form re-submits it (IRREVERSIBLE)."
    )]
    async fn reload(&self, Parameters(p): Parameters<LaneParams>) -> CallToolResult {
        self.run(json!({"action": "reload", "lane_id": p.lane_id}))
            .await
    }

    #[tool(
        name = "evaluate",
        description = "Evaluate a `script` in the page. The default scoped ACP capability does not expose this tool."
    )]
    async fn evaluate(&self, Parameters(p): Parameters<EvaluateParams>) -> CallToolResult {
        self.run(json!({
            "action": "evaluate",
            "script": p.script,
            "lane_id": p.lane.lane_id,
        }))
        .await
    }
}

#[rmcp::tool_handler(router = Self::tool_router())]
impl rmcp::ServerHandler for BrowserStdioServer {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let claims = self
            .client
            .access()
            .await
            .map_err(capability_request_error)?
            .claims;
        let tools = Self::tool_router()
            .list_all()
            .into_iter()
            .filter(|tool| claims.allows(&tool.name))
            .collect();
        Ok(rmcp::model::ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        self.client
            .access_for(&request.name)
            .await
            .map_err(capability_request_error)?;
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        Self::tool_router().call(call).await
    }
}

fn capability_request_error(error: String) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_request(
        format!("browser capability is no longer valid: {error}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_child_capability_hides_evaluate_and_validates() {
        let config = BrowserMcpConfig::from_issuer(
            41_000,
            std::sync::Arc::new(
                nomifun_common::LoopbackCapabilityIssuer::random().unwrap(),
            ),
            "nomicore".into(),
        );
        let child = config
            .issue_for_conversation(
                "0190f5fe-7c00-7a00-8000-000000000081",
                "0190f5fe-7c00-7a00-8000-000000000082",
                Some("agent-test"),
            )
            .unwrap();
        assert!(validate_browser_claims(&child.bootstrap.access.claims).is_ok());
        assert!(!child.bootstrap.access.claims.allows("evaluate"));
        for tool in ["get_console_logs", "get_page_errors", "get_network_log"] {
            assert!(
                child.bootstrap.access.claims.allows(tool),
                "read-only diagnostic tool {tool} must remain available"
            );
        }
    }

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
            // Lane management
            "browser_open", "browser_fork", "browser_list", "browser_status", "browser_close",
            "browser_close_all", "browser_crawl_many",
            // read-only
            "navigate", "observe", "screenshot", "capabilities", "get_page_text",
            "get_console_logs", "get_page_errors", "get_network_log", "search_page",
            "find_elements", "get_dropdown_options", "cursor", "tabs", "wait", "wait_for",
            // write / interaction
            "click", "hover", "type", "set_value", "select_option", "press_key", "scroll",
            "scroll_to_text", "upload_file", "download", "save_as_pdf", "extract", "switch_frame",
            "switch_tab", "close_tab", "open_link_new_tab", "back", "forward", "reload", "evaluate",
        ] {
            assert!(names.contains(&expected.to_string()), "missing tool {expected}; got {names:?}");
        }
        // The discrete bridge surface contains 42 protocol definitions; the
        // signed capability may expose a strict subset (notably no evaluate).
        assert_eq!(names.len(), 42, "expected 42 discrete browser tools; got {}: {names:?}", names.len());
    }

    #[test]
    fn every_existing_action_schema_accepts_optional_lane_id() {
        let router = BrowserStdioServer::tool_router();
        let lane_aware = [
            "navigate", "observe", "screenshot", "capabilities", "get_page_text",
            "get_console_logs", "get_page_errors", "get_network_log", "search_page",
            "find_elements", "get_dropdown_options", "cursor", "tabs", "wait", "wait_for",
            "click", "hover", "type", "set_value", "select_option", "press_key", "scroll",
            "scroll_to_text", "upload_file", "download", "save_as_pdf", "extract",
            "switch_frame", "switch_tab", "close_tab", "open_link_new_tab", "back",
            "forward", "reload", "evaluate",
        ];
        for tool in router.list_all() {
            if lane_aware.contains(&tool.name.as_ref()) {
                assert!(
                    tool.input_schema
                        .get("properties")
                        .and_then(Value::as_object)
                        .is_some_and(|properties| properties.contains_key("lane_id")),
                    "{} must expose optional lane_id: {:?}",
                    tool.name,
                    tool.input_schema,
                );
            }
        }
    }

    #[test]
    fn management_tool_schemas_do_not_expose_identity_policy_inputs() {
        let router = BrowserStdioServer::tool_router();
        for tool in router.list_all() {
            if matches!(
                tool.name.as_ref(),
                "browser_open" | "browser_fork" | "browser_crawl_many"
            ) {
                let properties = tool
                    .input_schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("management tool schema properties");
                assert!(
                    !properties.contains_key("identity_mode"),
                    "{} must not expose identity_mode: {:?}",
                    tool.name,
                    tool.input_schema,
                );
                assert!(
                    !properties.contains_key("authenticated"),
                    "{} must not expose authenticated: {:?}",
                    tool.name,
                    tool.input_schema,
                );
            }
        }
    }

    #[test]
    fn legacy_identity_policy_fields_fail_closed() {
        assert!(
            serde_json::from_value::<BrowserOpenParams>(json!({
                "lane_name": "default",
                "identity_mode": "isolated"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<BrowserCrawlManyParams>(json!({
                "urls": ["https://example.test"],
                "identity_mode": "authenticated_replica"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<BrowserCrawlManyParams>(json!({
                "urls": ["https://example.test"],
                "authenticated": true
            }))
            .is_err()
        );
    }

    #[test]
    fn crawl_many_concurrency_accepts_only_auto_or_integer_one_through_eight() {
        for valid in [json!("auto"), json!(1), json!(8)] {
            assert!(
                serde_json::from_value::<BrowserCrawlManyParams>(json!({
                    "urls": ["https://example.test"],
                    "concurrency": valid,
                }))
                .is_ok()
            );
        }
        for invalid in [
            json!("AUTO"),
            json!("4"),
            json!(0),
            json!(-1),
            json!(9),
            json!(1.5),
            json!(4.0),
            json!(false),
            json!(null),
            json!({}),
            json!([]),
        ] {
            assert!(
                serde_json::from_value::<BrowserCrawlManyParams>(json!({
                    "urls": ["https://example.test"],
                    "concurrency": invalid,
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn crawl_many_concurrency_schema_is_bounded() {
        let router = BrowserStdioServer::tool_router();
        let tool = router
            .list_all()
            .into_iter()
            .find(|tool| tool.name.as_ref() == "browser_crawl_many")
            .expect("crawl tool");
        let schema = &tool.input_schema["properties"]["concurrency"];
        let rendered = schema.to_string();
        assert!(rendered.contains("\"auto\""), "{schema:?}");
        assert!(rendered.contains("\"minimum\":1"), "{schema:?}");
        assert!(rendered.contains("\"maximum\":8"), "{schema:?}");
        assert!(!rendered.contains("\"null\""), "{schema:?}");
    }

    #[test]
    fn lane_id_deserializes_on_legacy_discrete_tools() {
        let navigate: NavigateParams = serde_json::from_value(json!({
            "url": "https://example.test",
            "lane_id": "lane-owned"
        }))
        .unwrap();
        assert_eq!(navigate.lane.lane_id.as_deref(), Some("lane-owned"));

        let click: RefParams = serde_json::from_value(json!({
            "ref": "f1e2",
            "lane_id": "lane-owned"
        }))
        .unwrap();
        assert_eq!(click.lane.lane_id.as_deref(), Some("lane-owned"));

        let network: NetworkLogParams = serde_json::from_value(json!({
            "include_bodies": false,
            "lane_id": "lane-owned"
        }))
        .unwrap();
        assert_eq!(network.lane.lane_id.as_deref(), Some("lane-owned"));
    }
}
