use std::collections::HashMap;
use std::time::Duration;

use nomi_process_runtime::ChildProcessBuilder as CmdBuilder;
use nomifun_runtime::resolve_command_path;

use crate::adapter::DetectedServer;
use crate::error::McpError;
use crate::types::McpServerTransport;

/// Timeout for detect/list operations (30 seconds).
pub const DETECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for install/remove operations (5 seconds).
pub const MUTATE_TIMEOUT: Duration = Duration::from_secs(5);

/// Check whether a CLI binary is available on `$PATH`.
///
/// Uses `nomifun_runtime::resolve_command_path` so the lookup respects
/// the bundled-bun shim and Windows `PATHEXT` rules. Previously this
/// shelled out to `which`, which does not exist on Windows and made
/// every MCP adapter report "not installed" there.
pub async fn is_cli_installed(name: &str) -> Result<bool, McpError> {
    Ok(resolve_command_path(name).is_some())
}

/// Run a CLI command with a timeout and clean environment variables.
///
/// Returns `(stdout, stderr)` on success. Returns an error if the command
/// fails to start, times out, or exits with a non-zero status.
pub async fn run_cli(program: &str, args: &[&str], timeout: Duration) -> Result<(String, String), McpError> {
    let mut builder = CmdBuilder::clean_cli(program);
    builder.args(args);
    let result = tokio::time::timeout(timeout, builder.output()).await;

    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(McpError::AgentOperationFailed(format!(
                "`{program}` failed to start: {e}"
            )));
        }
        Err(_) => {
            return Err(McpError::AgentOperationFailed(format!(
                "`{program} {}` timed out after {}s",
                args.join(" "),
                timeout.as_secs()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Non-zero exit is not always fatal — callers inspect stdout/stderr.
    Ok((stdout, stderr))
}

/// Run a CLI command and require zero exit status.
pub async fn run_cli_strict(program: &str, args: &[&str], timeout: Duration) -> Result<String, McpError> {
    let mut builder = CmdBuilder::clean_cli(program);
    builder.args(args);
    let result = tokio::time::timeout(timeout, builder.output()).await;

    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(McpError::AgentOperationFailed(format!(
                "`{program}` failed to start: {e}"
            )));
        }
        Err(_) => {
            return Err(McpError::AgentOperationFailed(format!(
                "`{program} {}` timed out after {}s",
                args.join(" "),
                timeout.as_secs()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(McpError::AgentOperationFailed(format!(
            "`{program} {}` exited with {}: {}",
            args.join(" "),
            output.status,
            if stderr.is_empty() { &stdout } else { stderr.as_ref() }
        )));
    }

    Ok(stdout)
}

/// Strip ANSI escape codes from CLI output.
pub fn strip_ansi(input: &str) -> String {
    // Matches: ESC[ ... m  (SGR sequences) and other CSI sequences.
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Consume the '[' and everything until a letter in @ ..~ range.
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() || c == '~' || c == '@' {
                        break;
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Normalize a CLI-reported MCP status string by stripping leading symbols
/// such as `✓`, `✗`, `!`, bullets, and extra whitespace.
pub fn normalize_detection_status(status: &str) -> String {
    status
        .trim()
        .trim_start_matches(|c: char| {
            matches!(c, '✓' | '✗' | '!' | '•' | '-' | '*' | '✔' | '✘' | ':' | '[' | ']') || c.is_whitespace()
        })
        .trim()
        .to_owned()
}

/// Parse the "standard" `mcp list` text output shared by Gemini and Qwen.
///
/// Pattern: `[checkmark] name: command (transport_type) - Status`
///
/// Each matching line produces a `DetectedServer`.
pub fn parse_standard_list_output(output: &str) -> Vec<DetectedServer> {
    let cleaned = strip_ansi(output);
    let mut servers = Vec::new();

    for line in cleaned.lines() {
        let trimmed = line.trim();
        if let Some(server) = parse_standard_list_line(trimmed) {
            servers.push(server);
        }
    }

    servers
}

/// Parse a single line of standard list output.
///
/// Expected pattern:
/// `[✓|✗] <name>: <command_or_url> (<transport_type>) - <Status>`
fn parse_standard_list_line(line: &str) -> Option<DetectedServer> {
    // Must start with a check/cross mark (Unicode or ASCII fallback)
    let rest = if line.starts_with('\u{2713}') || line.starts_with('\u{2717}') {
        &line[3..] // UTF-8 multibyte ✓/✗
    } else if line.starts_with("✓") || line.starts_with("✗") {
        // Already handled above via char check
        return parse_standard_list_line_inner(line);
    } else {
        return None;
    };

    parse_standard_list_line_inner_rest(rest.trim())
}

fn parse_standard_list_line_inner(line: &str) -> Option<DetectedServer> {
    // Skip the leading mark character
    let rest = line.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
    parse_standard_list_line_inner_rest(rest)
}

fn parse_standard_list_line_inner_rest(rest: &str) -> Option<DetectedServer> {
    // Find "name: command_or_url (type) - Status"
    let status_sep = rest.rfind(" - ")?;
    let status = normalize_detection_status(&rest[status_sep + 3..]);

    let rest = &rest[..status_sep];

    let colon_pos = rest.find(':')?;
    let name = rest[..colon_pos].trim();
    if name.is_empty() {
        return None;
    }

    let after_colon = rest[colon_pos + 1..].trim();

    // Find the transport type in parentheses
    let paren_open = after_colon.rfind('(')?;
    let paren_close = after_colon.rfind(')')?;
    if paren_close <= paren_open {
        return None;
    }

    let transport_type = after_colon[paren_open + 1..paren_close].trim();
    let command_or_url = after_colon[..paren_open].trim();

    let transport = match transport_type {
        "stdio" => McpServerTransport::Stdio {
            command: command_or_url.to_owned(),
            args: Vec::new(),
            env: HashMap::new(),
        },
        "sse" => McpServerTransport::Sse {
            url: command_or_url.to_owned(),
            headers: HashMap::new(),
        },
        "http" | "streamable_http" => McpServerTransport::Http {
            url: command_or_url.to_owned(),
            headers: HashMap::new(),
        },
        _ => return None,
    };

    Some(DetectedServer {
        name: name.to_owned(),
        transport,
        importable: status.eq_ignore_ascii_case("connected"),
        import_skip_reason: if status.eq_ignore_ascii_case("connected") {
            None
        } else {
            Some(status)
        },
    })
}

/// Build `--env "KEY=VALUE"` argument pairs for CLI commands.
pub fn build_env_args(env: &HashMap<String, String>, flag: &str) -> Vec<String> {
    env.iter()
        .flat_map(|(k, v)| [flag.to_owned(), format!("{k}={v}")])
        .collect()
}

/// Build `--header "Key: Value"` or `-H "Key: Value"` argument pairs.
pub fn build_header_args(headers: &HashMap<String, String>, flag: &str) -> Vec<String> {
    headers
        .iter()
        .flat_map(|(k, v)| [flag.to_owned(), format!("{k}: {v}")])
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_cli_installed_finds_known_binary() {
        // Both platforms ship a usable shell on PATH out of the box: `sh`
        // on Unix, `cmd` on Windows. resolve_command_path must locate it.
        #[cfg(unix)]
        let probe = "sh";
        #[cfg(windows)]
        let probe = "cmd";

        assert!(is_cli_installed(probe).await.unwrap(), "expected `{probe}` on PATH");
    }

    #[tokio::test]
    async fn is_cli_installed_returns_false_for_missing_binary() {
        let result = is_cli_installed("nomifun-definitely-not-a-real-binary-xyz")
            .await
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn strip_ansi_removes_color_codes() {
        let input = "\x1b[32m✓\x1b[0m my-server: npx (stdio) - \x1b[32mConnected\x1b[0m";
        let cleaned = strip_ansi(input);
        assert_eq!(cleaned, "✓ my-server: npx (stdio) - Connected");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        let input = "hello world";
        assert_eq!(strip_ansi(input), "hello world");
    }

    #[test]
    fn strip_ansi_handles_complex_sequences() {
        let input = "\x1b[1;34mBold Blue\x1b[0m normal \x1b[38;5;196mRed\x1b[0m";
        assert_eq!(strip_ansi(input), "Bold Blue normal Red");
    }

    #[test]
    fn normalize_detection_status_strips_prefix_symbols() {
        assert_eq!(normalize_detection_status("✓ Connected"), "Connected");
        assert_eq!(normalize_detection_status("✗ Failed to connect"), "Failed to connect");
        assert_eq!(
            normalize_detection_status("! Needs authentication"),
            "Needs authentication"
        );
    }

    #[test]
    fn parse_standard_list_stdio() {
        let output = "✓ my-server: npx -y @test/server (stdio) - Connected";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "my-server");
        match &servers[0].transport {
            McpServerTransport::Stdio { command, .. } => {
                assert_eq!(command, "npx -y @test/server");
            }
            _ => panic!("expected Stdio"),
        }
    }

    #[test]
    fn parse_standard_list_http() {
        let output = "✗ remote-srv: https://example.com/mcp (http) - Disconnected";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        assert!(!servers[0].importable);
        assert_eq!(servers[0].import_skip_reason.as_deref(), Some("Disconnected"));
    }

    #[test]
    fn parse_standard_list_sse() {
        let output = "✓ sse-srv: https://example.com/sse (sse) - Connected";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        match &servers[0].transport {
            McpServerTransport::Sse { url, .. } => {
                assert_eq!(url, "https://example.com/sse");
            }
            _ => panic!("expected Sse"),
        }
    }

    #[test]
    fn parse_standard_list_multiple_servers() {
        let output = "\
Configured MCP servers:
✓ server-a: npx -y @a/srv (stdio) - Connected
✗ server-b: https://b.com/mcp (http) - Disconnected
✓ server-c: https://c.com/sse (sse) - Connected
Some footer text";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].name, "server-a");
        assert_eq!(servers[1].name, "server-b");
        assert!(!servers[1].importable);
        assert_eq!(servers[2].name, "server-c");
    }

    #[test]
    fn parse_standard_list_with_ansi() {
        let output = "\x1b[32m✓\x1b[0m my-mcp: npx -y @test/mcp (stdio) - \x1b[32mConnected\x1b[0m";
        let servers = parse_standard_list_output(output);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "my-mcp");
    }

    #[test]
    fn parse_standard_list_empty_output() {
        let servers = parse_standard_list_output("");
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_standard_list_no_matching_lines() {
        let output = "No MCP servers configured.\nTry `mcp add` to get started.";
        let servers = parse_standard_list_output(output);
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_standard_list_unknown_transport_skipped() {
        let output = "✓ srv: cmd (websocket) - Connected";
        let servers = parse_standard_list_output(output);
        assert!(servers.is_empty());
    }

    #[test]
    fn build_env_args_produces_pairs() {
        let mut env = HashMap::new();
        env.insert("K1".into(), "V1".into());
        let args = build_env_args(&env, "--env");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "--env");
        assert_eq!(args[1], "K1=V1");
    }

    #[test]
    fn build_env_args_empty() {
        let env = HashMap::new();
        let args = build_env_args(&env, "--env");
        assert!(args.is_empty());
    }

    #[test]
    fn build_header_args_produces_pairs() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer tok".into());
        let args = build_header_args(&headers, "--header");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "--header");
        assert_eq!(args[1], "Authorization: Bearer tok");
    }
}
