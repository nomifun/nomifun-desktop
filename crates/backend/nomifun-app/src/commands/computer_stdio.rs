//! `nomicore mcp-computer-stdio` subcommand: an MCP stdio server exposing the
//! desktop computer-use capability as DISCRETE tools (Snapshot, Click, Type,
//! Launch, …) for external ACP agents (codex / claude / gemini).
//!
//! It is a thin facade over the in-tree `nomi_computer::ComputerTool` — every
//! tool here translates to a `ComputerTool` action and forwards the result — so
//! codex gets the exact same upgraded automation (multi-window semantic tree,
//! scrollables, reliable `launch`, real right/double click) that the nomi engine
//! gets, with zero duplicated logic. Injected on every desktop OS (macOS /
//! Windows / Linux) when the `computer-use` feature is built (see
//! `ComputerMcpConfig`); compiled only with that feature so headless/web builds
//! never pull xcap/enigo/UI Automation. The underlying `nomi-a11y` backend is
//! per-OS (macOS AX / Windows UIA / Linux AT-SPI); unsupported ops on a given OS
//! (e.g. OCR on Linux) surface as honest errors rather than being gated off.
//!
//! Discrete tools (vs the nomi engine's single `Computer(action)` tool) match
//! the shape models are trained on, which raises adoption — the user picked this
//! form for the bridge.

use std::process::ExitCode;
use std::sync::Arc;

use nomi_computer::tool::ComputerTool;
use nomi_config::config::ComputerConfig;
use nomi_tools::Tool;
use nomi_types::tool::ToolResult;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn run_computer_stdio() -> ExitCode {
    eprintln!("[mcp-computer-stdio] Started OK.");

    let server = ComputerStdioServer {
        tool: Arc::new(ComputerTool::new(&ComputerConfig::default())),
    };

    let transport = transport::io::stdio();
    match server.serve(transport).await {
        Ok(peer) => {
            eprintln!("[mcp-computer-stdio] MCP session started, waiting for completion...");
            if let Err(e) = peer.waiting().await {
                eprintln!("[mcp-computer-stdio] Session ended with error: {e}");
            } else {
                eprintln!("[mcp-computer-stdio] Session ended normally");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[mcp-computer-stdio] Failed to start MCP server: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone)]
struct ComputerStdioServer {
    tool: Arc<ComputerTool>,
}

/// Translate a `ComputerTool` `ToolResult` into an MCP `CallToolResult`: the text
/// content plus any images (screenshots / Set-of-Marks overlay) as image blocks.
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
struct RefParams {
    /// Element number `[ref]` from the most recent `snapshot`.
    r#ref: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SetValueParams {
    /// Element number `[ref]` from the most recent `snapshot`.
    r#ref: u32,
    /// The text to set into the element.
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct XyParams {
    /// X coordinate in pixels of the most recent screenshot.
    x: i64,
    /// Y coordinate in pixels of the most recent screenshot.
    y: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct TypeParams {
    /// The text to type into the focused control.
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct KeyParams {
    /// Key or combo to press, e.g. "enter" or "ctrl+a".
    key: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ScrollParams {
    /// Scroll direction: up, down, left, or right.
    direction: String,
    /// Wheel clicks (default 3).
    #[serde(default)]
    amount: Option<i64>,
    /// Optional X to scroll at (screenshot pixels).
    #[serde(default)]
    x: Option<i64>,
    /// Optional Y to scroll at (screenshot pixels).
    #[serde(default)]
    y: Option<i64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct LaunchParams {
    /// What to open: a URL (https://…), a file/folder path, or an application
    /// name (e.g. "notepad", "msedge").
    target: String,
    /// Optional application to open the target WITH (e.g. target a URL and
    /// app="msedge" to open it in Microsoft Edge).
    #[serde(default)]
    app: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ScreenshotParams {
    /// Optional display index to capture (default: primary).
    #[serde(default)]
    display: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WaitParams {
    /// Seconds to wait (max 5).
    #[serde(default)]
    seconds: Option<f64>,
}

#[tool_router(server_handler)]
impl ComputerStdioServer {
    async fn run(&self, input: Value) -> CallToolResult {
        to_mcp(self.tool.execute(input).await)
    }

    #[tool(
        name = "snapshot",
        description = "Read the desktop's accessibility tree (foreground window + other open windows) as a hierarchical `desktop → window → controls` tree with numbered [ref] elements and a Set-of-Marks overlay screenshot. Do this FIRST, then act on a control with `click`/`type` and its [ref]. Re-run after any UI change — a [ref] is only valid for the latest snapshot."
    )]
    async fn snapshot(&self) -> CallToolResult {
        self.run(json!({"action": "observe"})).await
    }

    #[tool(
        name = "screenshot",
        description = "Capture the screen as a PNG when you need raw pixels (optional `display` index)."
    )]
    async fn screenshot(&self, Parameters(p): Parameters<ScreenshotParams>) -> CallToolResult {
        self.run(json!({"action": "screenshot", "display": p.display})).await
    }

    #[tool(
        name = "click",
        description = "Activate the element with the given `ref` from the latest `snapshot` (accessibility action with an automatic pixel-click fallback)."
    )]
    async fn click(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "click_element", "ref": p.r#ref})).await
    }

    #[tool(
        name = "right_click",
        description = "Right-click the element with the given `ref` from the latest `snapshot` (opens its context menu)."
    )]
    async fn right_click(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "right_click_element", "ref": p.r#ref})).await
    }

    #[tool(
        name = "double_click",
        description = "Double-click the element with the given `ref` from the latest `snapshot`."
    )]
    async fn double_click(&self, Parameters(p): Parameters<RefParams>) -> CallToolResult {
        self.run(json!({"action": "double_click_element", "ref": p.r#ref})).await
    }

    #[tool(
        name = "set_value",
        description = "Set the `text` value of the element with the given `ref` (accessibility set-value with a focus-and-type fallback). Good for text fields."
    )]
    async fn set_value(&self, Parameters(p): Parameters<SetValueParams>) -> CallToolResult {
        self.run(json!({"action": "set_element_value", "ref": p.r#ref, "text": p.text})).await
    }

    #[tool(
        name = "click_xy",
        description = "Left-click at pixel coordinates (`x`, `y`) of the most recent screenshot. Use when the target is not in the accessibility tree; prefer `click` with a [ref] otherwise."
    )]
    async fn click_xy(&self, Parameters(p): Parameters<XyParams>) -> CallToolResult {
        self.run(json!({"action": "left_click", "x": p.x, "y": p.y})).await
    }

    #[tool(name = "type", description = "Type the `text` string into the focused control.")]
    async fn type_text(&self, Parameters(p): Parameters<TypeParams>) -> CallToolResult {
        self.run(json!({"action": "type", "text": p.text})).await
    }

    #[tool(
        name = "key",
        description = "Press a key or combo, e.g. \"enter\" or \"ctrl+a\". Prefer key presses over clicking buttons when both work."
    )]
    async fn key(&self, Parameters(p): Parameters<KeyParams>) -> CallToolResult {
        self.run(json!({"action": "key", "key": p.key})).await
    }

    #[tool(
        name = "scroll",
        description = "Scroll in `direction` (up/down/left/right) by `amount` wheel clicks, optionally at (`x`, `y`)."
    )]
    async fn scroll(&self, Parameters(p): Parameters<ScrollParams>) -> CallToolResult {
        self.run(json!({
            "action": "scroll",
            "direction": p.direction,
            "amount": p.amount,
            "x": p.x,
            "y": p.y
        }))
        .await
    }

    #[tool(
        name = "launch",
        description = "Open an application, URL, file, or folder reliably via the OS shell (ShellExecute). ALWAYS use this to open apps/URLs — do NOT run `cmd /c start`, `Start-Process`, or `explorer` in a shell, which are unreliable on Windows and pop a 'Windows cannot find' dialog. Pass `target` (a URL, path, or app name) and optionally `app` to open the target WITH a specific application."
    )]
    async fn launch(&self, Parameters(p): Parameters<LaunchParams>) -> CallToolResult {
        self.run(json!({"action": "launch", "target": p.target, "app": p.app})).await
    }

    #[tool(name = "list_windows", description = "List open windows with ids, titles, positions and sizes.")]
    async fn list_windows(&self) -> CallToolResult {
        self.run(json!({"action": "list_windows"})).await
    }

    #[tool(name = "cursor_position", description = "Report the mouse cursor position in screenshot coordinates.")]
    async fn cursor_position(&self) -> CallToolResult {
        self.run(json!({"action": "cursor_position"})).await
    }

    #[tool(name = "wait", description = "Pause for `seconds` (max 5) to let the UI settle.")]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        self.run(json!({"action": "wait", "seconds": p.seconds})).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_schemas_have_properties_field() {
        let router = ComputerStdioServer::tool_router();
        let tools = router.list_all();
        assert!(!tools.is_empty(), "computer bridge must register tools");
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
        let router = ComputerStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        for expected in [
            "snapshot", "screenshot", "click", "right_click", "double_click", "set_value",
            "click_xy", "type", "key", "scroll", "launch", "list_windows", "cursor_position",
            "wait",
        ] {
            assert!(names.contains(&expected.to_string()), "missing tool {expected}; got {names:?}");
        }
    }
}
