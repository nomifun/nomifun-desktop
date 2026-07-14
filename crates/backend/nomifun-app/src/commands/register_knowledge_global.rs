//! Merge-safe user-scope registration for external Claude/Gemini/Codex CLIs.
//! Files remain command-only; runtime capabilities come from the local broker.

use std::io;
use std::path::{Path, PathBuf};

use nomifun_terminal::AgentCli;
use serde::Serialize;
use serde_json::Value;

use super::register_knowledge::{
    KNOWLEDGE_MCP_SERVER_NAME, RegisterOutcome, atomic_write, merge_registration_file, run_codex,
};

#[derive(Debug, Clone, Serialize)]
pub struct UnregisterOutcome {
    pub path: String,
    pub removed: bool,
}

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir().filter(|path| !path.as_os_str().is_empty())
}

fn user_config_path(family: AgentCli) -> Option<PathBuf> {
    let home = home_dir()?;
    match family {
        AgentCli::Claude => Some(home.join(".claude.json")),
        AgentCli::Gemini => Some(home.join(".gemini").join("settings.json")),
        AgentCli::Codex => None,
    }
}

pub fn register_global(family: AgentCli, nomicore: &str) -> io::Result<RegisterOutcome> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let path = user_config_path(family).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "cannot resolve user home directory")
            })?;
            merge_registration_file(&path, family, nomicore)?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "user".into(),
                note: Some("已写入用户级配置（无密钥）；该 CLI 在任意目录启动即加载知识库工具".into()),
            })
        }
        AgentCli::Codex => {
            run_codex(&[
                "mcp",
                "add",
                KNOWLEDGE_MCP_SERVER_NAME,
                "--",
                nomicore,
                "mcp-knowledge-stdio",
            ])?;
            Ok(RegisterOutcome {
                written_path: "~/.codex/config.toml".into(),
                scope: "user".into(),
                note: Some("已注册到 Codex 用户级配置（对所有目录生效）".into()),
            })
        }
    }
}

pub fn unregister_global(family: AgentCli) -> io::Result<UnregisterOutcome> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let path = user_config_path(family).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "cannot resolve user home directory")
            })?;
            let removed = unregister_from_file(&path)?;
            Ok(UnregisterOutcome {
                path: path.to_string_lossy().into_owned(),
                removed,
            })
        }
        AgentCli::Codex => {
            run_codex(&["mcp", "remove", KNOWLEDGE_MCP_SERVER_NAME])?;
            Ok(UnregisterOutcome {
                path: "~/.codex/config.toml".into(),
                removed: true,
            })
        }
    }
}

pub fn is_registered_global(family: AgentCli) -> Option<bool> {
    match family {
        AgentCli::Claude | AgentCli::Gemini => {
            let document: Value = serde_json::from_str(
                &std::fs::read_to_string(user_config_path(family)?).ok()?,
            )
            .ok()?;
            Some(
                document
                    .get("mcpServers")
                    .and_then(|servers| servers.get(KNOWLEDGE_MCP_SERVER_NAME))
                    .is_some_and(is_command_only_registration),
            )
        }
        // Codex owns and may evolve its TOML schema. We intentionally avoid
        // parsing or rewriting that file behind the CLI's back.
        AgentCli::Codex => None,
    }
}

fn is_command_only_registration(value: &Value) -> bool {
    let Some(server) = value.as_object() else {
        return false;
    };
    let args_are_canonical = server
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| {
            args.len() == 1 && args[0].as_str() == Some("mcp-knowledge-stdio")
        });
    if server.get("command").and_then(Value::as_str).is_none() || !args_are_canonical {
        return false;
    }
    !server.keys().any(|key| {
        matches!(
            key.to_ascii_lowercase().as_str(),
            "env" | "url" | "headers" | "token" | "port" | "capability"
        )
    })
}

fn unregister_from_file(path: &Path) -> io::Result<bool> {
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let mut document: Value = serde_json::from_str(&existing).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("existing MCP config is invalid JSON: {error}"),
        )
    })?;
    let removed = document
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .map(|servers| servers.remove(KNOWLEDGE_MCP_SERVER_NAME).is_some())
        .unwrap_or(false);
    if removed {
        let serialized = serde_json::to_vec_pretty(&document)
            .map_err(|error| io::Error::other(format!("could not serialize MCP config: {error}")))?;
        atomic_write(path, &serialized)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_unregister_roundtrip_preserves_other_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"mine":{"command":"x"}},"top":"keep"}"#,
        )
        .unwrap();
        merge_registration_file(&path, AgentCli::Claude, "/bin/nomicore").unwrap();
        assert!(unregister_from_file(&path).unwrap());
        assert!(!unregister_from_file(&path).unwrap());
        let document: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(document["mcpServers"]["mine"]["command"], "x");
        assert_eq!(document["top"], "keep");
    }

    #[test]
    fn malformed_global_config_is_not_modified() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(unregister_from_file(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn legacy_static_credentials_are_not_reported_as_secure_registration() {
        assert!(is_command_only_registration(&serde_json::json!({
            "command": "/bin/nomicore",
            "args": ["mcp-knowledge-stdio"]
        })));
        assert!(!is_command_only_registration(&serde_json::json!({
            "command": "/bin/nomicore",
            "args": ["mcp-knowledge-stdio"],
            "env": {"NOMI_KB_MCP_CAPABILITY": "stale"}
        })));
    }
}
