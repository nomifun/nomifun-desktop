//! Compatibility entry points backed by the same verified component supplier.
use crate::{ToolError, ToolId};
use std::path::{Path, PathBuf};
pub type ResolveError = ToolError;

/// Resolve the managed Bun runtime, including an explicitly verified development override.
/// Cache-only discovery never initiates preparation, probes or network I/O.
pub fn resolve_bun() -> Result<PathBuf, ResolveError> {
    if let Some(resolved) = crate::managed::cached_tool(ToolId::Bun) {
        return resolved.map(|tool| tool.executable_path);
    }
    Err(ToolError::NotPrepared)
}

pub fn bun_bin_dir() -> Option<PathBuf> {
    resolve_bun()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
}

/// Discovery for user-owned commands; product callers use `resolve_tool`.
/// Bun/bunx retain their existing managed command entry points without PATH fallback.
pub fn resolve_command_path(command: &str) -> Option<PathBuf> {
    match command {
        "bun" => resolve_bun().ok(),
        "bunx" => bun_bin_dir()
            .map(|dir| dir.join(crate::extract::bunx_filename()))
            .filter(|path| path.is_file()),
        other => nomi_process_runtime::resolve_command_path(other),
    }
}

pub fn resolve_command_in(command: &str, dir: &Path) -> Option<PathBuf> {
    nomi_process_runtime::resolve_command_in(command, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_bun_resolution_contains_no_ambient_path_fallback() {
        let source = include_str!("resolver.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!source.contains("resolve_command_path(\"bun\")"));
    }
    #[test]
    fn command_in_cannot_escape_exact_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_command_in("definitely-missing", tmp.path()).is_none());
    }

    #[test]
    fn managed_bun_classification_preserves_explicit_paths_and_other_user_tools() {
        assert_eq!(managed_command_tool("bun"), Some(ToolId::Bun));
        assert_eq!(managed_command_tool("bunx"), Some(ToolId::Bun));
        for user_program in [
            "node",
            "officecli",
            "/usr/local/bin/bun",
            "./bun",
            "C:\\tools\\bun.exe",
            "BunOther",
            "",
        ] {
            assert_eq!(managed_command_tool(user_program), None, "{user_program}");
        }
        #[cfg(windows)]
        assert_eq!(managed_command_tool("BUNX.EXE"), Some(ToolId::Bun));
        #[cfg(not(windows))]
        assert_eq!(managed_command_tool("BUNX.EXE"), None);
    }
}

/// Classify only declared bare Bun launchers; paths remain user-owned.
pub fn managed_command_tool(program: &str) -> Option<ToolId> {
    if program.contains(['/', '\\']) {
        return None;
    }
    let normalized = if cfg!(windows) {
        program.to_ascii_lowercase()
    } else {
        program.to_string()
    };
    match normalized.as_str() {
        "bun" | "bunx" => Some(ToolId::Bun),
        "bun.exe" | "bunx.exe" if cfg!(windows) => Some(ToolId::Bun),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCommand {
    pub executable_path: PathBuf,
    pub args: Vec<String>,
}

/// Bind a declared launcher to the single approved supplier, including bunx semantics.
pub async fn prepare_managed_command(
    program: &str,
    args: &[String],
) -> Result<Option<PreparedCommand>, ToolError> {
    let Some(id) = managed_command_tool(program) else {
        return Ok(None);
    };
    let tool = crate::resolve_tool(id).await?;
    Ok(Some(bind_managed_command(
        program,
        args,
        tool.executable_path,
    )))
}

/// HTTP launches require a ready supplier; this never starts or waits for preparation.
pub fn resolve_cached_managed_command(
    program: &str,
    args: &[String],
) -> Result<Option<PreparedCommand>, ToolError> {
    let Some(id) = managed_command_tool(program) else {
        return Ok(None);
    };
    let Some(tool) = crate::managed::cached_tool(id) else {
        return Ok(None);
    };
    Ok(Some(bind_managed_command(
        program,
        args,
        tool?.executable_path,
    )))
}

fn bind_managed_command(
    program: &str,
    args: &[String],
    executable_path: PathBuf,
) -> PreparedCommand {
    let mut prepared = Vec::new();
    if program.eq_ignore_ascii_case("bunx")
        || (cfg!(windows) && program.eq_ignore_ascii_case("bunx.exe"))
    {
        prepared.push("x".into());
    }
    prepared.extend_from_slice(args);
    PreparedCommand {
        executable_path,
        args: prepared,
    }
}
