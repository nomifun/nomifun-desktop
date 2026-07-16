//! `nomicore mcp-open-stdio` subcommand: MCP stdio server exposing a single
//! reliable `open` tool that ShellExecutes a URL / file / folder / application.
//!
//! Spawned by ACP agent CLIs (claude / codex / gemini) when the open MCP is
//! injected into a session (Windows only — see `OpenMcpConfig`). It gives the
//! agent a dependable launch path instead of fragile `cmd /c start` /
//! `Start-Process` shell commands, whose `start` window-title-argument quirk
//! mis-handles URLs/paths and surfaces "Windows cannot find 'X'" dialogs.
//!
//! Unlike the requirement/gateway bridges this is STATELESS: opening is a pure
//! local OS call (`open::that_detached` via `ShellService::launch`), so there is
//! no HTTP hop back to the main process — no port/token, no `reqwest`.

use std::process::ExitCode;
use std::sync::Arc;

use nomifun_shell::{DefaultSystemOpener, ShellService};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::Deserialize;

use super::stdio_common::{ForwardToolOutcome, into_mcp_tool_result};

pub async fn run_open_stdio() -> ExitCode {
    eprintln!("[mcp-open-stdio] Started OK.");

    let server = OpenStdioServer {
        shell: Arc::new(ShellService::new(Arc::new(DefaultSystemOpener))),
    };

    let transport = transport::io::stdio();
    match server.serve(transport).await {
        Ok(peer) => {
            eprintln!("[mcp-open-stdio] MCP session started, waiting for completion...");
            if let Err(e) = peer.waiting().await {
                eprintln!("[mcp-open-stdio] Session ended with error: {e}");
            } else {
                eprintln!("[mcp-open-stdio] Session ended normally");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[mcp-open-stdio] Failed to start MCP server: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone)]
struct OpenStdioServer {
    shell: Arc<ShellService>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct OpenParams {
    /// What to open: a URL (e.g. "https://www.example.com"), a file or folder
    /// path, or an application name/path (e.g. "msedge", "notepad").
    target: String,
    /// Optional application to open the target WITH — e.g. set target to a URL
    /// and app to "msedge" to open that URL in Microsoft Edge specifically.
    /// Omit to use the system's default handler.
    #[serde(default)]
    app: Option<String>,
}

#[tool_router(server_handler)]
impl OpenStdioServer {
    #[tool(
        name = "open",
        description = "Open a URL, file, folder, or application on the user's desktop reliably via the OS shell (ShellExecute). ALWAYS prefer this over running `cmd /c start`, `Start-Process`, or `explorer` in the shell to launch things — those are unreliable on Windows (the `start` builtin mis-parses URLs/paths as window titles and pops 'Windows cannot find' dialogs). `target` is a URL (https://…), a filesystem path, or an application name/path. Optionally pass `app` to open the target with a specific application (e.g. target a URL and app=\"msedge\" to open it in Microsoft Edge)."
    )]
    async fn open(&self, Parameters(params): Parameters<OpenParams>) -> CallToolResult {
        let OpenParams { target, app } = params;
        eprintln!("[mcp-open-stdio] tools/call: open target={target:?} app={app:?}");
        let outcome = match self.shell.launch(&target, app.as_deref()).await {
            Ok(()) => match &app {
                Some(a) => ForwardToolOutcome::Success(format!("Opened {target:?} with {a:?}.")),
                None => ForwardToolOutcome::Success(format!("Opened {target:?}.")),
            },
            Err(e) => ForwardToolOutcome::Error(format!("Error: {e}")),
        };
        into_mcp_tool_result(outcome)
    }
}

#[cfg(test)]
mod tests {
    use nomifun_shell::NoopSystemOpener;

    use super::*;

    fn test_server() -> OpenStdioServer {
        OpenStdioServer {
            shell: Arc::new(ShellService::new(Arc::new(NoopSystemOpener))),
        }
    }

    #[test]
    fn all_tool_schemas_have_properties_field() {
        let router = OpenStdioServer::tool_router();
        let tools = router.list_all();
        assert!(!tools.is_empty(), "open bridge must register at least one tool");
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
    fn registers_open_tool() {
        let router = OpenStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"open".to_string()), "got {names:?}");
    }

    #[tokio::test]
    async fn failed_open_sets_mcp_is_error() {
        let result = test_server()
            .open(Parameters(OpenParams {
                target: String::new(),
                app: None,
            }))
            .await;

        assert_eq!(result.is_error, Some(true));
        assert!(serde_json::to_string(&result).unwrap().contains("Error:"));
    }

    #[tokio::test]
    async fn successful_open_text_is_not_keyword_classified_as_error() {
        let result = test_server()
            .open(Parameters(OpenParams {
                target: "Error report.txt".to_string(),
                app: None,
            }))
            .await;

        assert_ne!(result.is_error, Some(true));
        assert!(
            serde_json::to_string(&result)
                .unwrap()
                .contains("Error report.txt")
        );
    }
}
