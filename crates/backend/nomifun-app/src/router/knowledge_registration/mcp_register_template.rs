//! Secret-free manual registration templates for the external knowledge MCP.
//!
//! Every format stores only the canonical executable plus
//! `mcp-knowledge-stdio`. Runtime authority is obtained from the OS-authenticated
//! local broker, never from a persisted port, token, environment variable, or
//! endpoint beacon.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterTemplate {
    pub claude_cmd: String,
    pub claude_json: String,
    pub codex_toml: String,
    pub gemini_json: String,
}

pub fn knowledge_register_template(nomicore_path: &str) -> RegisterTemplate {
    let shell_quoted = shell_quote_arg(nomicore_path);
    let json_escaped = json_escape_string(nomicore_path);
    let toml_escaped = toml_basic_string(nomicore_path);
    let claude_cmd = format!(
        "claude mcp add nomifun-knowledge --scope user -- {} mcp-knowledge-stdio",
        shell_quoted
    );
    let claude_json = format!(
        "{{\n  \"mcpServers\": {{\n    \"nomifun-knowledge\": {{\n      \"command\": \"{}\",\n      \"args\": [\"mcp-knowledge-stdio\"]\n    }}\n  }}\n}}",
        json_escaped
    );
    let codex_toml = format!(
        "[mcp_servers.nomifun-knowledge]\ncommand = {}\nargs = [\"mcp-knowledge-stdio\"]",
        toml_escaped
    );
    RegisterTemplate {
        claude_cmd,
        gemini_json: claude_json.clone(),
        claude_json,
        codex_toml,
    }
}

#[cfg(not(windows))]
fn shell_quote_arg(value: &str) -> String {
    // POSIX shells do not expand $, backticks, backslashes, or command
    // substitutions inside single quotes. A literal quote is represented by
    // closing the quote, emitting one double-quoted quote, then reopening it.
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn shell_quote_arg(value: &str) -> String {
    // The Windows command template targets PowerShell. PowerShell single quotes
    // suppress $, backtick, and subexpression expansion; two adjacent quotes
    // encode one literal quote.
    format!("'{}'", value.replace('\'', "''"))
}

fn json_escape_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn toml_basic_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_command_only_and_secret_free() {
        let template = knowledge_register_template("/Users/John Doe/bin/nomicore");
        for value in [
            &template.claude_cmd,
            &template.claude_json,
            &template.codex_toml,
            &template.gemini_json,
        ] {
            assert!(value.contains("mcp-knowledge-stdio"));
            let lower = value.to_ascii_lowercase();
            assert!(!lower.contains("token"));
            assert!(!lower.contains("capability"));
            assert!(!lower.contains("nomi_kb_mcp"));
            assert!(!lower.contains("\"port\""));
            assert!(!lower.contains("port ="));
        }
        let json: serde_json::Value = serde_json::from_str(&template.claude_json).unwrap();
        assert_eq!(
            json["mcpServers"]["nomifun-knowledge"]["command"],
            "/Users/John Doe/bin/nomicore"
        );
    }

    #[test]
    fn quoting_preserves_windows_path() {
        let path = r#"C:\Program Files\Nomi \"Fun\"\nomicore.exe"#;
        let template = knowledge_register_template(path);
        let json: serde_json::Value = serde_json::from_str(&template.gemini_json).unwrap();
        assert_eq!(json["mcpServers"]["nomifun-knowledge"]["command"], path);
        assert!(template.codex_toml.contains("\\\\"));
        assert!(template.codex_toml.contains("\\\""));
    }

    #[cfg(unix)]
    #[test]
    fn shell_template_roundtrips_without_expansion() {
        let path = r#"/tmp/Nomi $HOME `printf backtick` $(printf subshell) it's/nomicore"#;
        let quoted = shell_quote_arg(path);
        let script = format!("set -- {quoted}; printf '%s' \"$1\"");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, path.as_bytes());
    }

    #[cfg(windows)]
    #[test]
    fn powershell_template_suppresses_expansion() {
        let path = r#"C:\Nomi $env:TEMP `whoami` $(whoami) it's\nomicore.exe"#;
        assert_eq!(
            shell_quote_arg(path),
            r#"'C:\Nomi $env:TEMP `whoami` $(whoami) it''s\nomicore.exe'"#
        );
    }
}
