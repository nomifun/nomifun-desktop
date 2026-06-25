//! User-scope (global) one-click registration of the platform knowledge MCP
//! into a CLI's USER-level config, so an agent CLI started in ANY directory —
//! not just a NomiFun-managed terminal — picks up knowledge_search / knowledge_read
//! / knowledge_write. Secret-free (the bridge resolves port/token from the
//! endpoint beacon at runtime), so the written file is safe to keep and survives
//! app restarts. Merge-safe: never clobbers the user's own servers.
//!
//! Mirrors [`super::register_knowledge`] (project scope) but targets the user
//! config:
//!   - Claude → `~/.claude.json`
//!   - Gemini → `~/.gemini/settings.json`
//!   - Codex  → `codex mcp add` (codex config is global; no project/user split)

use std::path::{Path, PathBuf};

use nomifun_terminal::AgentCli;
use serde::Serialize;
use serde_json::Value;

use super::register_knowledge::{RegisterOutcome, merge_gemini_settings, merge_mcp_json};

/// MCP server name (must match the bridge subcommand registration).
const SERVER_NAME: &str = "nomifun-knowledge";

/// Outcome of a global unregistration.
#[derive(Debug, Clone, Serialize)]
pub struct UnregisterOutcome {
    pub path: String,
    /// `true` when the server entry was present and removed; `false` when it was
    /// already absent (idempotent no-op).
    pub removed: bool,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// User-level config path for the family. `Codex` is `None` — it is managed via
/// the `codex` CLI (global config), not a path we write directly.
fn user_config_path(family: AgentCli) -> Option<PathBuf> {
    let home = home_dir()?;
    match family {
        AgentCli::Claude => Some(home.join(".claude.json")),
        AgentCli::Gemini => Some(home.join(".gemini").join("settings.json")),
        AgentCli::Codex => None,
    }
}

/// Remove the `nomifun-knowledge` server from an mcpServers-shaped JSON doc.
/// Returns `(serialized_doc, removed)`. Tolerant of malformed/missing content.
fn remove_server_from_mcp_json(existing: &str) -> (String, bool) {
    let Ok(mut doc) = serde_json::from_str::<Value>(existing) else {
        return (existing.to_owned(), false);
    };
    let removed = doc
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .map(|servers| servers.remove(SERVER_NAME).is_some())
        .unwrap_or(false);
    let out = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| existing.to_owned());
    (out, removed)
}

/// Merge the registration into a specific config file (path-injectable core,
/// used by [`register_global`] and unit tests).
fn register_to_file(path: &Path, family: AgentCli, nomicore: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(path).ok();
    let merged = match family {
        AgentCli::Gemini => merge_gemini_settings(existing.as_deref(), nomicore),
        _ => merge_mcp_json(existing.as_deref(), nomicore),
    };
    std::fs::write(path, merged)
}

/// Remove the registration from a specific config file. Returns whether an
/// entry was actually removed.
fn unregister_from_file(path: &Path) -> std::io::Result<bool> {
    let Some(existing) = std::fs::read_to_string(path).ok() else {
        return Ok(false);
    };
    let (doc, removed) = remove_server_from_mcp_json(&existing);
    if removed {
        std::fs::write(path, doc)?;
    }
    Ok(removed)
}

fn run_codex(args: &[&str]) -> std::io::Result<()> {
    let out = std::process::Command::new("codex")
        .args(args)
        .output()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, format!("codex CLI not found or failed to spawn: {e}")))?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "codex {} failed (exit {}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

/// Register the knowledge MCP into the family's USER-level config.
pub fn register_global(family: AgentCli, nomicore: &str) -> std::io::Result<RegisterOutcome> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let path =
                user_config_path(family).ok_or_else(|| std::io::Error::other("cannot resolve user home directory"))?;
            register_to_file(&path, family, nomicore)?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "user".into(),
                note: Some("已写入用户级配置（无密钥）；该 CLI 在任意目录启动即加载知识库工具".into()),
            })
        }
        AgentCli::Codex => {
            run_codex(&["mcp", "add", SERVER_NAME, "--", nomicore, "mcp-knowledge-stdio"])?;
            Ok(RegisterOutcome {
                written_path: "~/.codex/config.toml".into(),
                scope: "user".into(),
                note: Some("codex 无项目级配置，已注册到全局（对所有目录生效）".into()),
            })
        }
    }
}

/// Remove the knowledge MCP from the family's USER-level config (idempotent).
pub fn unregister_global(family: AgentCli) -> std::io::Result<UnregisterOutcome> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let path =
                user_config_path(family).ok_or_else(|| std::io::Error::other("cannot resolve user home directory"))?;
            let removed = unregister_from_file(&path)?;
            Ok(UnregisterOutcome { path: path.to_string_lossy().into_owned(), removed })
        }
        AgentCli::Codex => {
            run_codex(&["mcp", "remove", SERVER_NAME])?;
            Ok(UnregisterOutcome { path: "~/.codex/config.toml".into(), removed: true })
        }
    }
}

/// Whether the family's USER-level config currently registers the server.
/// File-based for Claude/Gemini; `None` for Codex (would require invoking the
/// CLI to inspect global config — the UI treats `None` as "unknown").
pub fn is_registered_global(family: AgentCli) -> Option<bool> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let path = user_config_path(family)?;
            let existing = std::fs::read_to_string(&path).ok()?;
            let doc: Value = serde_json::from_str(&existing).ok()?;
            Some(doc.get("mcpServers").and_then(|s| s.get(SERVER_NAME)).is_some())
        }
        AgentCli::Codex => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_unregister_roundtrip_claude() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");

        // Pre-seed a user server that must survive the merge.
        std::fs::write(&path, r#"{"mcpServers":{"mine":{"command":"x"}},"top":"keep"}"#).unwrap();

        register_to_file(&path, AgentCli::Claude, "/usr/bin/nomicore").unwrap();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"][SERVER_NAME]["command"], "/usr/bin/nomicore");
        assert_eq!(doc["mcpServers"]["mine"]["command"], "x", "user server preserved");
        assert_eq!(doc["top"], "keep", "top-level preserved");

        let removed = unregister_from_file(&path).unwrap();
        assert!(removed);
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(doc["mcpServers"].get(SERVER_NAME).is_none(), "server removed");
        assert_eq!(doc["mcpServers"]["mine"]["command"], "x", "user server still preserved");

        // Idempotent: removing again is a no-op.
        assert!(!unregister_from_file(&path).unwrap());
    }

    #[test]
    fn register_creates_gemini_settings_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gemini").join("settings.json");
        register_to_file(&path, AgentCli::Gemini, "/bin/nomicore").unwrap();
        assert!(path.exists(), "settings.json created with parent dir");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"][SERVER_NAME]["command"], "/bin/nomicore");
    }

    #[test]
    fn remove_is_tolerant_and_no_token() {
        // Absent server → not removed, doc unchanged-ish.
        let (_doc, removed) = remove_server_from_mcp_json(r#"{"mcpServers":{}}"#);
        assert!(!removed);
        // Malformed → no removal, original returned.
        let (doc, removed) = remove_server_from_mcp_json("not json");
        assert_eq!(doc, "not json");
        assert!(!removed);
    }

    #[test]
    fn unregister_missing_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert!(!unregister_from_file(&path).unwrap());
    }
}
