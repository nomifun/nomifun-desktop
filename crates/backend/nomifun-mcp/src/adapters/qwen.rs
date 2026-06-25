use std::collections::HashMap;

use nomifun_common::McpSource;

use crate::adapter::{DetectedServer, McpAgentAdapter};
use crate::error::McpError;
use crate::types::McpServerTransport;

use super::cli_helpers::{
    DETECT_TIMEOUT, MUTATE_TIMEOUT, build_env_args, build_header_args, is_cli_installed, parse_standard_list_output,
    run_cli,
};

const CLI_NAME: &str = "qwen";

/// Scopes tried when removing (user first, then project).
const REMOVE_SCOPES: &[&str] = &["user", "project"];

/// MCP Agent adapter for Qwen CLI.
///
/// # CLI Commands
///
/// - **detect**: `qwen mcp list`
/// - **install (stdio)**: `qwen mcp add <name> <command> [args...] [--env K=V]... -s user`
/// - **install (http/sse)**: `qwen mcp add <name> <url> --transport <type> [--header K: V]... -s user`
/// - **remove**: `qwen mcp remove <name> -s user` → `-s project` → file fallback
///
/// If CLI remove fails, falls back to editing `~/.qwen/client_config.json`.
pub struct QwenAdapter;

#[async_trait::async_trait]
impl McpAgentAdapter for QwenAdapter {
    fn source(&self) -> McpSource {
        McpSource::Qwen
    }

    async fn is_installed(&self) -> Result<bool, McpError> {
        is_cli_installed(CLI_NAME).await
    }

    async fn detect_existing(&self) -> Result<Vec<DetectedServer>, McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        let (stdout, _stderr) = run_cli(CLI_NAME, &["mcp", "list"], DETECT_TIMEOUT).await?;
        Ok(parse_standard_list_output(&stdout))
    }

    async fn install_server(&self, name: &str, transport: &McpServerTransport) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        match transport {
            McpServerTransport::Stdio { command, args, env } => {
                let mut cli_args = vec!["mcp".to_owned(), "add".to_owned(), name.to_owned(), command.clone()];
                cli_args.extend(args.iter().cloned());
                cli_args.extend(build_env_args(env, "--env"));
                cli_args.push("-s".to_owned());
                cli_args.push("user".to_owned());

                let arg_refs: Vec<&str> = cli_args.iter().map(|s| s.as_str()).collect();
                run_cli(CLI_NAME, &arg_refs, MUTATE_TIMEOUT).await?;
            }
            McpServerTransport::Sse { url, headers } => {
                install_http_like(name, "sse", url, headers).await?;
            }
            McpServerTransport::Http { url, headers } => {
                install_http_like(name, "http", url, headers).await?;
            }
        }

        Ok(())
    }

    async fn remove_server(&self, name: &str) -> Result<(), McpError> {
        if !self.is_installed().await? {
            return Err(McpError::AgentNotInstalled(CLI_NAME.into()));
        }

        // Try CLI removal with each scope.
        for scope in REMOVE_SCOPES {
            let (stdout, _stderr) = run_cli(CLI_NAME, &["mcp", "remove", name, "-s", scope], MUTATE_TIMEOUT).await?;
            let lower = stdout.to_lowercase();
            if lower.contains("removed") {
                return Ok(());
            }
        }

        // Fallback: directly edit ~/.qwen/client_config.json
        remove_from_config_file(name).await
    }
}

/// Install an HTTP-like (sse/http) server via `qwen mcp add`.
async fn install_http_like(
    name: &str,
    transport_type: &str,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<(), McpError> {
    let mut cli_args = vec![
        "mcp".to_owned(),
        "add".to_owned(),
        name.to_owned(),
        url.to_owned(),
        "--transport".to_owned(),
        transport_type.to_owned(),
    ];
    cli_args.extend(build_header_args(headers, "--header"));
    cli_args.push("-s".to_owned());
    cli_args.push("user".to_owned());

    let arg_refs: Vec<&str> = cli_args.iter().map(|s| s.as_str()).collect();
    run_cli(CLI_NAME, &arg_refs, MUTATE_TIMEOUT).await?;
    Ok(())
}

/// Fallback: remove server from `~/.qwen/client_config.json` directly.
///
/// Reads the file, deletes the key from `mcpServers`, writes back.
/// Silently succeeds if the file doesn't exist or the key is absent.
async fn remove_from_config_file(name: &str) -> Result<(), McpError> {
    let home = home_dir()?;
    let config_path = home.join(".qwen").join("client_config.json");

    if !config_path.exists() {
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| McpError::AgentOperationFailed(format!("read qwen config: {e}")))?;

    let mut config: serde_json::Value = serde_json::from_str(&content).map_err(McpError::from)?;

    let removed = config
        .get_mut("mcpServers")
        .and_then(|servers| servers.as_object_mut())
        .map(|servers| servers.remove(name).is_some())
        .unwrap_or(false);

    if removed {
        let new_content = serde_json::to_string_pretty(&config).map_err(McpError::from)?;
        tokio::fs::write(&config_path, new_content)
            .await
            .map_err(|e| McpError::AgentOperationFailed(format!("write qwen config: {e}")))?;
    }

    Ok(())
}

/// Get the user's home directory.
fn home_dir() -> Result<std::path::PathBuf, McpError> {
    dirs::home_dir().ok_or_else(|| McpError::AgentOperationFailed("cannot determine home directory".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_qwen() {
        assert_eq!(QwenAdapter.source(), McpSource::Qwen);
    }

    #[test]
    fn parse_qwen_list_output() {
        let output = "\
✓ my-server: npx -y @test/server (stdio) - Connected
✗ broken: node bad.js (stdio) - Disconnected";

        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "my-server");
        assert_eq!(servers[1].name, "broken");
    }

    #[test]
    fn parse_qwen_http_server() {
        let output = "✓ remote: https://example.com/mcp (http) - Connected";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Http { url, .. } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let adapter: Box<dyn McpAgentAdapter> = Box::new(QwenAdapter);
        assert_eq!(adapter.source(), McpSource::Qwen);
    }
}
