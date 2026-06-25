//! Manual MCP registration template generator.
//!
//! Produces shell/JSON/TOML snippets that users can paste into external/wrapper
//! CLIs to register the platform `knowledge_search` bridge. CRITICAL: templates
//! contain NO port, token, or `NOMI_KB_MCP` env — the bridge discovers them at
//! runtime via the beacon file.

use serde::Serialize;

/// Registration snippets for all supported CLI formats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterTemplate {
    /// `claude mcp add ...` one-liner.
    pub claude_cmd: String,
    /// Claude `.mcp.json` / `--mcp-config` JSON fragment.
    pub claude_json: String,
    /// Codex `config.toml` fragment.
    pub codex_toml: String,
    /// Gemini `settings.json` MCP fragment (same shape as claude).
    pub gemini_json: String,
}

/// Generate all four registration template strings for the given backend binary
/// path (`nomicore_path`). The path is quoted/escaped per format so spaced paths
/// are safe. No token/port is ever included.
pub fn knowledge_register_template(nomicore_path: &str) -> RegisterTemplate {
    let shell_quoted = shell_quote_arg(nomicore_path);
    let json_escaped = json_escape_string(nomicore_path);
    let toml_escaped = toml_basic_string(nomicore_path);

    // claude mcp add command (shell)
    let claude_cmd = format!(
        "claude mcp add nomifun-knowledge --scope user -- {} mcp-knowledge-stdio",
        shell_quoted
    );

    // claude JSON (mcpServers shape)
    let claude_json = format!(
        "{{\n  \"mcpServers\": {{\n    \"nomifun-knowledge\": {{\n      \"command\": \"{}\",\n      \"args\": [\"mcp-knowledge-stdio\"]\n    }}\n  }}\n}}",
        json_escaped
    );

    // codex TOML
    let codex_toml = format!(
        "[mcp_servers.nomifun-knowledge]\ncommand = {}\nargs = [\"mcp-knowledge-stdio\"]",
        toml_escaped
    );

    // gemini JSON (same mcpServers shape)
    let gemini_json = claude_json.clone();

    RegisterTemplate {
        claude_cmd,
        claude_json,
        codex_toml,
        gemini_json,
    }
}

// --- Quoting helpers (mirrored from nomifun-terminal/src/enhance.rs) ---

/// Double-quote a string for shell use (handles spaces, escapes `\` and `"`).
fn shell_quote_arg(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Escape a string for embedding inside a JSON `"..."` value (the outer quotes
/// are NOT included — caller wraps). Handles `\`, `"`, and control chars.
fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// TOML basic-string literal: wrapped in quotes, internal `\`, `"`, and control
/// chars escaped.
fn toml_basic_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_contains_path_and_subcommand() {
        let t = knowledge_register_template("/usr/local/bin/nomicore");
        assert!(t.claude_cmd.contains("/usr/local/bin/nomicore"));
        assert!(t.claude_cmd.contains("mcp-knowledge-stdio"));
        assert!(t.claude_json.contains("/usr/local/bin/nomicore"));
        assert!(t.claude_json.contains("mcp-knowledge-stdio"));
        assert!(t.codex_toml.contains("/usr/local/bin/nomicore"));
        assert!(t.codex_toml.contains("mcp-knowledge-stdio"));
        assert!(t.gemini_json.contains("/usr/local/bin/nomicore"));
        assert!(t.gemini_json.contains("mcp-knowledge-stdio"));
    }

    #[test]
    fn template_never_contains_token_or_port_or_env() {
        let t = knowledge_register_template("/Users/John Doe/bin/nomicore");
        for field in [&t.claude_cmd, &t.claude_json, &t.codex_toml, &t.gemini_json] {
            let lower = field.to_lowercase();
            assert!(!lower.contains("token"), "field contains 'token': {field}");
            assert!(
                !lower.contains("nomi_kb_mcp"),
                "field contains 'NOMI_KB_MCP': {field}"
            );
            // No port number patterns (port = digits after "port" keyword)
            assert!(!lower.contains("\"port\""), "field contains port key: {field}");
            assert!(!lower.contains("port ="), "field contains port key: {field}");
        }
    }

    #[test]
    fn spaced_path_is_correctly_quoted_in_each_format() {
        let path = "/Users/John Doe/bin/nomicore";
        let t = knowledge_register_template(path);

        // Shell: path must be wrapped in double-quotes
        assert!(
            t.claude_cmd.contains("\"/Users/John Doe/bin/nomicore\""),
            "shell quoting broken: {}",
            t.claude_cmd
        );

        // JSON: path in a JSON string value (spaces are literal, no escaping needed for space)
        assert!(
            t.claude_json.contains("/Users/John Doe/bin/nomicore"),
            "json missing spaced path: {}",
            t.claude_json
        );
        // Verify valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&t.claude_json)
            .unwrap_or_else(|e| panic!("claude_json is not valid JSON: {e}\n{}", t.claude_json));
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"].as_str().unwrap(),
            path
        );

        // TOML: command value must be a quoted string containing the spaced path
        assert!(
            t.codex_toml.contains("\"/Users/John Doe/bin/nomicore\""),
            "toml quoting broken: {}",
            t.codex_toml
        );

        // Gemini JSON: same as claude
        let parsed_g: serde_json::Value = serde_json::from_str(&t.gemini_json)
            .unwrap_or_else(|e| panic!("gemini_json is not valid JSON: {e}\n{}", t.gemini_json));
        assert_eq!(
            parsed_g["mcpServers"]["nomifun-knowledge"]["command"].as_str().unwrap(),
            path
        );
    }

    #[test]
    fn path_with_backslash_and_quotes_is_safe() {
        // Windows-ish path with both tricky chars
        let path = r#"C:\Program Files\Nomi "Fun"\nomicore.exe"#;
        let t = knowledge_register_template(path);

        // JSON must be parseable
        let parsed: serde_json::Value = serde_json::from_str(&t.claude_json)
            .unwrap_or_else(|e| panic!("claude_json parse failed: {e}\n{}", t.claude_json));
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"].as_str().unwrap(),
            path
        );

        // TOML command value — parse just the value portion
        // The full TOML should contain escaped backslashes and quotes
        assert!(t.codex_toml.contains("\\\\"), "TOML must escape backslash");
        assert!(t.codex_toml.contains("\\\""), "TOML must escape quote");
    }

    #[test]
    fn claude_cmd_format() {
        let t = knowledge_register_template("/bin/nomicore");
        assert_eq!(
            t.claude_cmd,
            "claude mcp add nomifun-knowledge --scope user -- \"/bin/nomicore\" mcp-knowledge-stdio"
        );
    }
}
