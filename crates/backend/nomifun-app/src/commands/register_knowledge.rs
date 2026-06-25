//! One-click registration of the platform knowledge MCP into the working path's
//! CLI auto-discovery file. Secret-free (command=<nomicore> mcp-knowledge-stdio;
//! the bridge discovers port/token at runtime via the endpoint beacon), so the
//! written file is safe to commit and survives restarts. Merge-safe: never
//! clobbers the user's own servers.

use std::path::Path;

use nomifun_terminal::AgentCli;
use serde::Serialize;
use serde_json::{Value, json};

/// Outcome of a one-click registration into a workpath (or global for codex).
#[derive(Debug, Clone, Serialize)]
pub struct RegisterOutcome {
    pub written_path: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Build the nomifun-knowledge server JSON value (NO token, NO port).
fn knowledge_server_value(nomicore: &str) -> Value {
    json!({ "command": nomicore, "args": ["mcp-knowledge-stdio"] })
}

/// Merge the nomifun-knowledge server into an existing (or new) `.mcp.json`-shaped
/// JSON document, preserving any other mcpServers + top-level keys.
///
/// Tolerant of malformed/missing existing content: falls back to an empty object.
pub fn merge_mcp_json(existing: Option<&str>, nomicore: &str) -> String {
    let mut doc: Value = existing
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let obj = doc.as_object_mut().unwrap();
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers
        .as_object_mut()
        .unwrap()
        .insert("nomifun-knowledge".to_owned(), knowledge_server_value(nomicore));
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Gemini `settings.json` uses the same `mcpServers` shape as claude's `.mcp.json`.
pub fn merge_gemini_settings(existing: Option<&str>, nomicore: &str) -> String {
    merge_mcp_json(existing, nomicore)
}

/// Write/merge the platform knowledge MCP registration into the workpath for
/// the given agent family.
///
/// - `Claude` → `<cwd>/.mcp.json` (project scope)
/// - `Gemini` → `<cwd>/.gemini/settings.json` (project scope, creates dir)
/// - `Codex` → runs `codex mcp add nomifun-knowledge -- <nomicore> mcp-knowledge-stdio`
///   (global scope; codex has no cwd-scoped config)
pub fn register_into_workpath(
    cwd: &str,
    family: AgentCli,
    nomicore: &str,
) -> std::io::Result<RegisterOutcome> {
    match family {
        AgentCli::Claude => {
            let path = Path::new(cwd).join(".mcp.json");
            let existing = std::fs::read_to_string(&path).ok();
            std::fs::write(&path, merge_mcp_json(existing.as_deref(), nomicore))?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "project".into(),
                note: Some(
                    "已写入项目 .mcp.json（无密钥，可提交）；claude 在此目录启动即加载".into(),
                ),
            })
        }
        AgentCli::Gemini => {
            let dir = Path::new(cwd).join(".gemini");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("settings.json");
            let existing = std::fs::read_to_string(&path).ok();
            std::fs::write(&path, merge_gemini_settings(existing.as_deref(), nomicore))?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "project".into(),
                note: None,
            })
        }
        AgentCli::Codex => {
            // codex has no cwd-scoped config; let codex itself merge its global TOML.
            let out = std::process::Command::new("codex")
                .args(["mcp", "add", "nomifun-knowledge", "--", nomicore, "mcp-knowledge-stdio"])
                .output()
                .map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("codex CLI not found or failed to spawn: {e}"),
                    )
                })?;
            if !out.status.success() {
                return Err(std::io::Error::other(format!(
                    "codex mcp add failed (exit {}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                )));
            }
            Ok(RegisterOutcome {
                written_path: "~/.codex/config.toml".into(),
                scope: "global".into(),
                note: Some(
                    "codex 无工作路径级配置，已注册到全局 ~/.codex（对所有项目生效）".into(),
                ),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_mcp_json_new_creates_server() {
        let result = merge_mcp_json(None, "/usr/local/bin/nomicore");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/usr/local/bin/nomicore"
        );
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["args"],
            json!(["mcp-knowledge-stdio"])
        );
    }

    #[test]
    fn merge_mcp_json_preserves_existing_server() {
        let existing = r#"{
            "mcpServers": {
                "my-custom-server": { "command": "foo", "args": ["bar"] }
            },
            "topLevel": "preserved"
        }"#;
        let result = merge_mcp_json(Some(existing), "/bin/nomicore");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        // User's server preserved
        assert_eq!(parsed["mcpServers"]["my-custom-server"]["command"], "foo");
        // New server added
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
        // Top-level key preserved
        assert_eq!(parsed["topLevel"], "preserved");
    }

    #[test]
    fn merge_mcp_json_no_token_no_port() {
        let result = merge_mcp_json(None, "/bin/nomicore");
        let lower = result.to_lowercase();
        assert!(!lower.contains("token"), "must not contain token: {result}");
        assert!(!lower.contains("\"port\""), "must not contain port key: {result}");
    }

    #[test]
    fn merge_mcp_json_tolerates_malformed_existing() {
        // Completely invalid JSON → treated as empty
        let result = merge_mcp_json(Some("not json at all {{{"), "/bin/nomicore");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );

        // Existing is a JSON array, not object → treated as empty
        let result2 = merge_mcp_json(Some("[1,2,3]"), "/bin/nomicore");
        let parsed2: Value = serde_json::from_str(&result2).unwrap();
        assert_eq!(
            parsed2["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
    }

    #[test]
    fn merge_gemini_settings_same_shape() {
        let existing = r#"{ "theme": "dark", "mcpServers": {} }"#;
        let result = merge_gemini_settings(Some(existing), "/bin/nomicore");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["theme"], "dark");
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
    }

    #[test]
    fn register_into_workpath_claude_writes_mcp_json() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap();
        let outcome = register_into_workpath(cwd, AgentCli::Claude, "/bin/nomicore").unwrap();

        assert_eq!(outcome.scope, "project");
        assert!(outcome.written_path.ends_with(".mcp.json"));

        let content = std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["args"],
            json!(["mcp-knowledge-stdio"])
        );
    }

    #[test]
    fn register_into_workpath_gemini_creates_dir_and_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap();
        let outcome = register_into_workpath(cwd, AgentCli::Gemini, "/bin/nomicore").unwrap();

        assert_eq!(outcome.scope, "project");
        assert!(outcome.written_path.contains(".gemini"));
        assert!(outcome.written_path.ends_with("settings.json"));

        let path = tmp.path().join(".gemini/settings.json");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
    }

    #[test]
    fn register_into_workpath_claude_merge_preserves_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap();
        let existing = r#"{"mcpServers":{"user-server":{"command":"us","args":[]}}}"#;
        std::fs::write(tmp.path().join(".mcp.json"), existing).unwrap();

        let outcome = register_into_workpath(cwd, AgentCli::Claude, "/bin/nomicore").unwrap();
        assert_eq!(outcome.scope, "project");

        let content = std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap();
        let parsed: Value = serde_json::from_str(&content).unwrap();
        // User's server preserved
        assert_eq!(parsed["mcpServers"]["user-server"]["command"], "us");
        // New server added
        assert_eq!(
            parsed["mcpServers"]["nomifun-knowledge"]["command"],
            "/bin/nomicore"
        );
    }
}
