//! Merge-safe project registration for the external knowledge MCP.
//!
//! Persisted config contains only `command = <nomicore>` plus
//! `mcp-knowledge-stdio`. Authorization is issued at runtime by the protected
//! local broker, so these files are safe to commit and survive backend restarts.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nomifun_terminal::AgentCli;
use serde::Serialize;
use serde_json::{Value, json};

pub const KNOWLEDGE_MCP_SERVER_NAME: &str = "nomifun-knowledge";

#[derive(Debug, Clone, Serialize)]
pub struct RegisterOutcome {
    pub written_path: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn knowledge_server_value(nomicore: &str) -> Value {
    json!({ "command": nomicore, "args": ["mcp-knowledge-stdio"] })
}

/// Merge without destroying malformed or structurally incompatible user
/// configuration. Callers must surface `InvalidData` and leave the file intact.
pub fn merge_mcp_json(existing: Option<&str>, nomicore: &str) -> io::Result<String> {
    let mut document = match existing {
        Some(raw) => serde_json::from_str::<Value>(raw).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("existing MCP config is invalid JSON: {error}"),
            )
        })?,
        None => json!({}),
    };
    let object = document.as_object_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "existing MCP config must be a JSON object",
        )
    })?;
    let servers = object.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "existing mcpServers value must be a JSON object",
        )
    })?;
    servers.insert(
        KNOWLEDGE_MCP_SERVER_NAME.to_owned(),
        knowledge_server_value(nomicore),
    );
    serde_json::to_string_pretty(&document)
        .map_err(|error| io::Error::other(format!("could not serialize MCP config: {error}")))
}

pub fn merge_gemini_settings(existing: Option<&str>, nomicore: &str) -> io::Result<String> {
    merge_mcp_json(existing, nomicore)
}

pub fn register_into_workpath(
    cwd: &str,
    family: AgentCli,
    nomicore: &str,
) -> io::Result<RegisterOutcome> {
    let cwd = canonical_directory(cwd)?;
    match family {
        AgentCli::Claude => {
            let path = cwd.join(".mcp.json");
            merge_registration_file(&path, family, nomicore)?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "project".into(),
                note: Some("已写入项目 .mcp.json（无密钥，可提交）；Claude 在此目录启动即加载".into()),
            })
        }
        AgentCli::Gemini => {
            let path = cwd.join(".gemini").join("settings.json");
            merge_registration_file(&path, family, nomicore)?;
            Ok(RegisterOutcome {
                written_path: path.to_string_lossy().into_owned(),
                scope: "project".into(),
                note: Some("已写入项目 .gemini/settings.json（无密钥，可提交）".into()),
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
                note: Some("Codex 无项目级 MCP 配置，已注册到用户级配置（对所有目录生效）".into()),
            })
        }
    }
}

pub(super) fn merge_registration_file(
    path: &Path,
    family: AgentCli,
    nomicore: &str,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = match std::fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let merged = match family {
        AgentCli::Gemini => merge_gemini_settings(existing.as_deref(), nomicore)?,
        AgentCli::Claude | AgentCli::Codex => merge_mcp_json(existing.as_deref(), nomicore)?,
    };
    atomic_write(path, merged.as_bytes())
}

fn canonical_directory(raw: &str) -> io::Result<PathBuf> {
    if raw.is_empty() || raw.trim() != raw {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cwd must be a non-empty exact path",
        ));
    }
    let path = std::fs::canonicalize(raw)?;
    if !std::fs::metadata(&path)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "cwd must be a directory"));
    }
    Ok(path)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp-config");
    let temporary = parent.join(format!(
        ".{file_name}.nomifun-{}.tmp",
        nomifun_common::generate_id()
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both buffers are NUL-terminated UTF-16 paths and remain live for
    // the synchronous MoveFileExW call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn run_codex(args: &[&str]) -> io::Result<()> {
    let output = std::process::Command::new("codex")
        .args(args)
        .output()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Codex CLI not found or failed to spawn: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "codex {} failed (exit {}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_secret_free_and_preserves_user_config() {
        let existing = r#"{"mcpServers":{"mine":{"command":"mine"}},"theme":"dark"}"#;
        let merged = merge_mcp_json(Some(existing), "/bin/nomicore").unwrap();
        let document: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(document["mcpServers"]["mine"]["command"], "mine");
        assert_eq!(document["theme"], "dark");
        assert_eq!(
            document["mcpServers"][KNOWLEDGE_MCP_SERVER_NAME]["args"],
            json!(["mcp-knowledge-stdio"])
        );
        let lower = merged.to_ascii_lowercase();
        assert!(!lower.contains("token"));
        assert!(!lower.contains("capability"));
        assert!(!lower.contains("nomi_kb_mcp"));
        assert!(!lower.contains("\"port\""));
    }

    #[test]
    fn malformed_existing_config_is_never_clobbered() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".mcp.json");
        std::fs::write(&path, "not json {{{").unwrap();
        assert!(merge_registration_file(&path, AgentCli::Claude, "/bin/nomicore").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "not json {{{");
    }

    #[test]
    fn project_registration_canonicalizes_symlink_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&workspace, temp.path().join("alias")).unwrap();
            let result = register_into_workpath(
                temp.path().join("alias").to_str().unwrap(),
                AgentCli::Claude,
                "/bin/nomicore",
            )
            .unwrap();
            assert_eq!(
                result.written_path,
                std::fs::canonicalize(&workspace)
                    .unwrap()
                    .join(".mcp.json")
                    .to_string_lossy()
            );
        }
    }
}
