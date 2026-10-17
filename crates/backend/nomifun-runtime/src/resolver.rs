//! User-owned command discovery with verified managed caches as a fallback.
use crate::{ToolError, ToolId};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
pub type ResolveError = ToolError;

/// Discover system Bun first, then an existing valid managed cache.
/// Synchronous discovery never initiates preparation, probes or network I/O.
pub fn resolve_bun() -> Result<PathBuf, ResolveError> {
    if let Some(path) = system_command("bun") {
        return Ok(path);
    }
    if let Some(resolved) = existing_managed_tool(ToolId::Bun) {
        return resolved.map(|tool| tool.executable_path);
    }
    Err(ToolError::NotPrepared)
}

pub fn bun_bin_dir() -> Option<PathBuf> {
    resolve_bun()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
}

/// Discover user-owned commands first, then an existing verified runtime.
pub fn resolve_command_path(command: &str) -> Option<PathBuf> {
    if let Some(path) = system_command(command) {
        return Some(path);
    }
    match command {
        "bun" => resolve_bun().ok(),
        "bunx" => bun_bin_dir()
            .map(|dir| dir.join(crate::extract::bunx_filename()))
            .filter(|path| path.is_file()),
        "node" => existing_managed_tool(ToolId::Node).and_then(Result::ok).map(|tool| tool.executable_path),
        "npm" | "npx" => existing_managed_tool(ToolId::Node).and_then(Result::ok)
            .and_then(|tool| tool.executable_path.parent().map(|bin| bin.join(if cfg!(windows) {
                if command == "npm" { "npm.cmd" } else { "npx.cmd" }
            } else { command }))).filter(|path| path.is_file()),
        _ => None,
    }
}

pub fn resolve_command_in(command: &str, dir: &Path) -> Option<PathBuf> {
    nomi_process_runtime::resolve_command_in(command, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_in_cannot_escape_exact_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_command_in("definitely-missing", tmp.path()).is_none());
    }

    #[test]
    fn explicit_child_path_uses_child_cwd_and_preserves_bunx_arguments() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("tools");
        std::fs::create_dir(&directory).unwrap();
        let publish = |name: &str| {
            let filename = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
            let path = directory.join(filename);
            std::fs::write(&path, b"fixture executable discovery only").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };
        let bun = publish("bun");
        let path_key = if cfg!(windows) { "pAtH" } else { "PATH" };
        let env = HashMap::from([(path_key.to_owned(), "tools".to_owned())]);
        let args = vec!["fixture-package".to_owned()];
        let command = resolve_cached_managed_command_with_env("bun", &args, &env, Some(root.path()))
            .unwrap().unwrap();
        assert_eq!(command.executable_path, bun);
        assert_eq!(command.args, args);
        let command = resolve_cached_managed_command_with_env("bunx", &args, &env, Some(root.path()))
            .unwrap().unwrap();
        assert_eq!(command.executable_path, bun);
        assert_eq!(command.args, ["x", "fixture-package"]);
        let bunx = publish("bunx");
        let command = resolve_cached_managed_command_with_env("bunx", &args, &env, Some(root.path()))
            .unwrap().unwrap();
        assert_eq!(command.executable_path, bunx);
        assert_eq!(command.args, args);
        // The user's empty PATH can search their cwd, but never the host's PATH.
        assert!(command_in_environment("bun", Some(OsStr::new("")), Some(root.path()))
            .unwrap().is_none());
        assert!(command_in_environment("bun", Some(directory.as_os_str()), Some(root.path()))
            .unwrap().is_some());
    }

    #[test]
    fn managed_runtime_classification_preserves_explicit_paths_and_other_user_tools() {
        assert_eq!(managed_command_tool("bun"), Some(ToolId::Bun));
        assert_eq!(managed_command_tool("bunx"), Some(ToolId::Bun));
        for command in ["node", "npm", "npx"] { assert_eq!(managed_command_tool(command), Some(ToolId::Node)); }
        for user_program in [
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

/// Classify known bare runtime launchers; explicit paths remain user-owned.
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
        "node" | "npm" | "npx" => Some(ToolId::Node),
        "bun.exe" | "bunx.exe" if cfg!(windows) => Some(ToolId::Bun),
        "node.exe" | "npm.cmd" | "npx.cmd" if cfg!(windows) => Some(ToolId::Node),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedCommand {
    pub executable_path: PathBuf,
    pub args: Vec<String>,
}

/// Prepare a declared launcher, automatically supplying a missing runtime.
pub async fn prepare_managed_command(
    program: &str,
    args: &[String],
) -> Result<Option<PreparedCommand>, ToolError> {
    let prepared = prepare_managed_command_with_env(program, args, &HashMap::new(), None).await?;
    if prepared.is_none() && managed_command_tool(program).is_some() {
        return Err(ToolError::NotPrepared);
    }
    Ok(prepared)
}

/// Resolve a launch's command without blocking the async caller on filesystem I/O.
/// Explicit local development overrides retain their verified first-use admission.
pub async fn prepare_managed_command_with_env(
    program: &str,
    args: &[String],
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
) -> Result<Option<PreparedCommand>, ToolError> {
    let Some(id) = managed_command_tool(program) else { return Ok(None); };
    let local_program = program.to_owned();
    let local_args = args.to_vec();
    let local_env = env.clone();
    let local_cwd = cwd.map(Path::to_path_buf);
    let prepared = tokio::task::spawn_blocking(move || {
        resolve_cached_managed_command_with_env(
            &local_program, &local_args, &local_env, local_cwd.as_deref(),
        )
    }).await.map_err(|error| ToolError::Worker(error.to_string()))?;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) if crate::managed::cache_repairable(&error) => None,
        Err(error) => return Err(error),
    };
    if let Some(prepared) = prepared {
        // npm's native launcher may use /usr/bin/env node. Supply that required
        // runtime when the launch's own environment has no Node.
        if id == ToolId::Node && is_npm_launcher(program)
            && command_in_environment("node", env_path(env), cwd)?.is_none()
        {
            crate::managed::prepare_managed_tool(ToolId::Node).await?;
        }
        return Ok(Some(prepared));
    }
    let tool = crate::managed::prepare_managed_tool(id).await?;
    Ok(Some(bind_managed_command(program, args, tool.executable_path)))
}

/// User commands take priority; fallback requires an existing valid managed cache.
/// This never starts or waits for preparation.
pub fn resolve_cached_managed_command(
    program: &str,
    args: &[String],
) -> Result<Option<PreparedCommand>, ToolError> {
    resolve_cached_managed_command_in(program, args, None, None)
}

/// Respect the launch's explicit PATH and working directory before cache fallback.
/// An explicit PATH, including an empty value, never falls back to the host PATH.
pub fn resolve_cached_managed_command_with_env(
    program: &str,
    args: &[String],
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
) -> Result<Option<PreparedCommand>, ToolError> {
    let path = env_path(env);
    resolve_cached_managed_command_in(program, args, path, cwd)
}

fn env_path(env: &HashMap<String, String>) -> Option<&OsStr> {
    env.iter().find(|(key, _)| {
        if cfg!(windows) { key.eq_ignore_ascii_case("PATH") } else { key.as_str() == "PATH" }
    }).map(|(_, value)| OsStr::new(value))
}

fn resolve_cached_managed_command_in(
    program: &str,
    args: &[String],
    path: Option<&OsStr>,
    cwd: Option<&Path>,
) -> Result<Option<PreparedCommand>, ToolError> {
    let Some(id) = managed_command_tool(program) else {
        return Ok(None);
    };
    if let Some(path) = command_in_environment(program, path, cwd)? {
        return Ok(Some(PreparedCommand { executable_path: path, args: args.to_vec() }));
    }
    // bunx may be absent even when system Bun is installed: preserve `bun x`.
    if id == ToolId::Bun && let Some(path) = command_in_environment("bun", path, cwd)? {
        return Ok(Some(bind_managed_command(program, args, path)));
    }
    let Some(tool) = existing_managed_tool(id) else {
        return Ok(None);
    };
    Ok(Some(bind_managed_command(
        program,
        args,
        tool?.executable_path,
    )))
}

pub(crate) fn command_in_environment(
    command: &str,
    configured_path: Option<&OsStr>,
    cwd: Option<&Path>,
) -> Result<Option<PathBuf>, ToolError> {
    if let Some(path) = configured_path {
        return command_in_search_path(command, path, cwd);
    }
    if cfg!(debug_assertions) && managed_command_tool(command)
        .is_some_and(|id| std::env::var_os(id.override_variable()).is_some()) {
        return Ok(None);
    }
    if cwd.is_some() && let Some(path) = std::env::var_os("PATH") {
        return command_in_search_path(command, &path, cwd);
    }
    Ok(system_command(command))
}

fn command_in_search_path(
    command: &str,
    search: &OsStr,
    cwd: Option<&Path>,
) -> Result<Option<PathBuf>, ToolError> {
    for directory in std::env::split_paths(search) {
        let directory = if directory.is_absolute() {
            directory
        } else {
            let base = match cwd {
                Some(cwd) => std::path::absolute(cwd)?,
                None => std::env::current_dir()?,
            };
            base.join(directory)
        };
        if let Some(path) = nomi_process_runtime::resolve_command_in(command, &directory)
            && !crate::managed::is_managed_path(&path)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn system_command(command: &str) -> Option<PathBuf> {
    // Explicit development paths keep their established precedence and admission checks.
    if managed_command_tool(command).is_some()
        && cfg!(debug_assertions)
        && managed_command_tool(command).is_some_and(|id| std::env::var_os(id.override_variable()).is_some())
    {
        return None;
    }
    nomi_process_runtime::resolve_command_path(command)
        .filter(|path| !crate::managed::is_managed_path(path))
}

fn existing_managed_tool(id: ToolId) -> Option<Result<crate::VerifiedTool, ToolError>> {
    if cfg!(debug_assertions)
        && let Some(raw) = std::env::var_os(id.override_variable())
        && let Err(error) = crate::managed::development_path(id, raw.into())
    {
        return Some(Err(error));
    }
    crate::managed::cached_tool(id).or_else(|| {
        if cfg!(debug_assertions) && std::env::var_os(id.override_variable()).is_some() {
            None
        } else {
            crate::managed::published_tool(id)
        }
    })
}

fn bind_managed_command(
    program: &str,
    args: &[String],
    executable_path: PathBuf,
) -> PreparedCommand {
    let mut prepared = Vec::new();
    if is_npm_launcher(program) {
        let bin = executable_path.parent().expect("absolute managed Node executable");
        let root = if cfg!(windows) { bin.to_path_buf() } else { bin.parent().expect("Node distribution root").join("lib") };
        let launcher = if program.to_ascii_lowercase().starts_with("npx") { "npx-cli.js" } else { "npm-cli.js" };
        prepared.push(root.join("node_modules/npm/bin").join(launcher).to_string_lossy().into_owned());
    }
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

fn is_npm_launcher(program: &str) -> bool {
    matches!(program, "npm" | "npx") || (cfg!(windows) && matches!(program.to_ascii_lowercase().as_str(), "npm.cmd" | "npx.cmd" | "npm" | "npx"))
}
