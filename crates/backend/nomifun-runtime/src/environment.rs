//! The common per-launch user environment for Agent, terminals and MCP.
//!
//! This module never mutates the multi-threaded host environment. The user owns
//! command precedence and configuration; verified runtimes are fallback tools.
use crate::{ToolError, ToolId};
use std::{collections::HashMap, ffi::OsString, path::{Path, PathBuf}};

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedProcessLaunch {
    pub program: String,
    pub args: Vec<String>,
    pub environment: HashMap<String, String>,
}

impl std::fmt::Debug for PreparedProcessLaunch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("PreparedProcessLaunch")
            .field("program", &self.program)
            .field("argument_count", &self.args.len())
            .field("environment_entries", &self.environment.len())
            .finish()
    }
}

/// User-installed tools are durable user data, outside the application dataset
/// and replaceable runtime cache. Agent reset and runtime upgrades do not own it.
pub fn user_cli_root() -> Option<PathBuf> {
    dirs::data_local_dir().map(|root| root.join("nomifun-user-tools").join("npm"))
}

fn same_key(left: &str, right: &str) -> bool {
    if cfg!(windows) { left.eq_ignore_ascii_case(right) } else { left == right }
}

fn value<'a>(environment: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    environment.iter().find(|(key, _)| same_key(key, name)).map(|(_, value)| value.as_str())
}

fn insert(environment: &mut HashMap<String, String>, name: &str, value: String) {
    environment.retain(|key, _| !same_key(key, name));
    environment.insert(name.into(), value);
}

fn inherited_environment(
    entries: impl IntoIterator<Item = (OsString, OsString)>,
    configured: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ToolError> {
    let unrepresentable = |part| ToolError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("An inherited environment {part} cannot be represented by the UTF-8 process contract; the requested process was not started"),
    ));
    let mut environment = HashMap::new();
    for (key, entry) in entries {
        if nomi_process_runtime::is_inherited_application_environment_key(&key) { continue; }
        let key = key.into_string().map_err(|_| unrepresentable("name"))?;
        // A valid explicit task value replaces this inherited entry completely.
        if configured.keys().any(|configured| same_key(configured, &key)) { continue; }
        let entry = entry.into_string().map_err(|_| unrepresentable("value"))?;
        environment.insert(key, entry);
    }
    Ok(environment)
}

fn npm_prefix_setting(environment: &HashMap<String, String>) -> Option<String> {
    if let Some(prefix) = value(environment, "NPM_CONFIG_PREFIX") { return Some(prefix.to_owned()); }
    let configured = value(environment, "NPM_CONFIG_USERCONFIG").map(PathBuf::from);
    let home = value(environment, if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let path = configured.or_else(|| home.map(|home| home.join(".npmrc")));
    path.and_then(|path| std::fs::read_to_string(path).ok()).and_then(|contents| {
        contents.lines().filter_map(|line| line.split_once('='))
            .filter(|(key, _)| key.trim().eq_ignore_ascii_case("prefix"))
            .map(|(_, prefix)| prefix.trim().trim_matches('"').trim_matches('\'').to_owned()).last()
    })
}

fn npm_user_prefix_configured(environment: &HashMap<String, String>) -> bool {
    npm_prefix_setting(environment).is_some()
}

fn configured_npm_bin(environment: &HashMap<String, String>) -> Option<PathBuf> {
    let mut prefix = npm_prefix_setting(environment)?;
    for _ in 0..32 {
        let Some(start) = prefix.find("${") else { break; };
        let end = prefix[start + 2..].find('}')? + start + 2;
        let replacement = value(environment, &prefix[start + 2..end])?;
        prefix.replace_range(start..=end, replacement);
    }
    if prefix.contains("${") { return None; }
    if let Some(suffix) = prefix.strip_prefix("~/") {
        prefix = PathBuf::from(value(environment, if cfg!(windows) { "USERPROFILE" } else { "HOME" })?)
            .join(suffix).to_string_lossy().into_owned();
    }
    let prefix = PathBuf::from(prefix);
    if !prefix.is_absolute() { return None; }
    Some(if cfg!(windows) { prefix } else { prefix.join("bin") })
}

fn executable_in_path(command: &str, path: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|directory| {
        let directory = if directory.is_absolute() { directory } else { cwd.unwrap_or(Path::new(".")).join(directory) };
        crate::resolve_command_in(command, &directory)
    })
}

/// Compose a new child environment without downloads, directory creation or
/// global environment mutation. Explicit PATH (including empty) is authoritative
/// for user discovery; durable tools and verified runtime fallbacks follow it.
pub fn process_environment(configured: &HashMap<String, String>) -> Result<HashMap<String, String>, ToolError> {
    process_environment_in(configured, None)
}

pub(crate) fn process_environment_in(configured: &HashMap<String, String>, cwd: Option<&Path>) -> Result<HashMap<String, String>, ToolError> {
    let mut environment = inherited_environment(std::env::vars_os(), configured)?;
    if configured.keys().any(|key| same_key(key, "LANG") || same_key(key, "LC_CTYPE"))
        && !configured.keys().any(|key| same_key(key, "LC_ALL"))
    {
        environment.retain(|key, _| !same_key(key, "LC_ALL"));
    }
    if configured.keys().any(|key| same_key(key, "LANG"))
        && !configured.keys().any(|key| same_key(key, "LC_CTYPE"))
    {
        environment.retain(|key, _| !same_key(key, "LC_CTYPE"));
    }
    for (key, entry) in configured { insert(&mut environment, key, entry.clone()); }
    for (key, entry) in nomifun_net::proxy::child_proxy_env(configured.keys().map(String::as_str)) {
        if value(&environment, &key).is_none() { insert(&mut environment, &key, entry); }
    }
    let explicit_path = value(configured, "PATH");
    let original = value(&environment, "PATH").unwrap_or_default().to_owned();
    let mut bins: Vec<PathBuf> = std::env::split_paths(&original).collect();
    if explicit_path.is_none() { bins.extend(crate::shell_env::platform_extra_bins()); }
    if let Some(bin) = configured_npm_bin(&environment) { bins.push(bin); }
    let user_root = user_cli_root();
    if let Some(root) = &user_root {
        bins.push(if cfg!(windows) { root.clone() } else { root.join("bin") });
    }
    // Check the user's own Node before adding the managed fallback. A genuine
    // user toolchain keeps npm's own prefix/config semantics.
    let user_path = nomi_process_runtime::merge_process_path(bins.iter().cloned())
        .map_err(|error| ToolError::Io(std::io::Error::other(error)))?;
    let user_node = executable_in_path("node", &user_path.to_string_lossy(), cwd)
        .is_some_and(|path| !crate::managed::is_managed_path(&path));
    let mut managed_node = false;
    for id in [ToolId::Bun, ToolId::Node] {
        if let Some(Ok(tool)) = crate::managed::cached_tool(id).or_else(|| crate::managed::published_tool(id)) {
            if let Some(bin) = tool.executable_path.parent() { bins.push(bin.to_owned()); }
            managed_node |= id == ToolId::Node;
        }
    }
    let path = nomi_process_runtime::merge_process_path(bins)
        .map_err(|error| ToolError::Io(std::io::Error::other(error)))?;
    insert(&mut environment, "PATH", path.to_string_lossy().into_owned());
    if managed_node && !user_node && !npm_user_prefix_configured(&environment)
        && let Some(root) = user_root
    {
        insert(&mut environment, "NPM_CONFIG_PREFIX", root.to_string_lossy().into_owned());
    }
    Ok(environment)
}

/// Prepare the actual launch once for all process consumers. Native arguments
/// remain literal; explicit shell calls arrive here as shell executable + argv.
pub async fn prepare_process_launch(
    command: &str,
    args: &[String],
    cwd: &Path,
    configured: &HashMap<String, String>,
) -> Result<PreparedProcessLaunch, ToolError> {
    let configured_owned = configured.clone();
    let local_cwd = cwd.to_owned();
    let mut environment = tokio::task::spawn_blocking(move || process_environment_in(&configured_owned, Some(&local_cwd)))
        .await.map_err(|error| ToolError::Worker(error.to_string()))??;
    let prepared = crate::prepare_managed_command_with_env(command, args, &environment, Some(cwd)).await?;
    // Preparation may have published a runtime. Compose again so shebangs,
    // lifecycle scripts and later subcommands immediately see it in PATH.
    if prepared.is_some() {
        let configured_owned = configured.clone();
        let local_cwd = cwd.to_owned();
        environment = tokio::task::spawn_blocking(move || process_environment_in(&configured_owned, Some(&local_cwd)))
            .await.map_err(|error| ToolError::Worker(error.to_string()))??;
    }
    // A user may have Node but lack npm. The managed npm fallback executes its
    // own pinned Node, whose default prefix would otherwise be the immutable
    // distribution directory; keep installed CLIs in durable user data.
    if prepared.as_ref().is_some_and(|prepared| crate::managed::is_managed_path(&prepared.executable_path))
        && matches!(command.to_ascii_lowercase().as_str(), "npm" | "npx" | "npm.cmd" | "npx.cmd")
        && !npm_user_prefix_configured(&environment)
        && let Some(root) = user_cli_root()
    {
        insert(&mut environment, "NPM_CONFIG_PREFIX", root.to_string_lossy().into_owned());
    }
    let native = if prepared.is_none() {
        if command.contains(['/', '\\']) {
            let path = PathBuf::from(command);
            Some(if path.is_absolute() { path } else { cwd.join(path) }).filter(|path| path.is_file())
        } else {
            crate::resolver::command_in_environment(command,
                value(&environment, "PATH").map(std::ffi::OsStr::new), Some(cwd))?
        }
    } else { None };
    if let Some(native) = &native {
        let default_bin = user_cli_root().map(|root| if cfg!(windows) { root } else { root.join("bin") });
        let npm_bin = configured_npm_bin(&environment).or(default_bin);
        let node_ready = executable_in_path("node", value(&environment, "PATH").unwrap_or_default(), Some(cwd))
            .is_some_and(|path| !crate::managed::is_managed_path(&path)
                || matches!(crate::managed::cached_tool(ToolId::Node), Some(Ok(_))));
        // A durable npm CLI still needs Node after runtime-cache cleanup. Its
        // install directory identifies the ecosystem without rewriting argv or
        // guessing arbitrary command text.
        if npm_bin.is_some_and(|bin| native.starts_with(bin))
            && !node_ready
        {
            crate::managed::prepare_managed_tool(ToolId::Node).await?;
            let configured_owned = configured.clone();
            let local_cwd = cwd.to_owned();
            environment = tokio::task::spawn_blocking(move || process_environment_in(&configured_owned, Some(&local_cwd)))
                .await.map_err(|error| ToolError::Worker(error.to_string()))??;
        }
    }
    if value(&environment, "NPM_CONFIG_PREFIX").is_some()
        && let Some(root) = user_cli_root()
        && value(&environment, "NPM_CONFIG_PREFIX") == root.to_str()
    {
        tokio::fs::create_dir_all(&root).await?;
    }
    let (program, args) = match prepared {
        Some(prepared) => (prepared.executable_path.to_string_lossy().into_owned(), prepared.args),
        None => {
            let program = if command.contains(['/', '\\']) {
                command.to_owned()
            } else {
                native
                    .map_or_else(|| command.to_owned(), |path| path.to_string_lossy().into_owned())
            };
            (program, args.to_vec())
        },
    };
    Ok(PreparedProcessLaunch { program, args, environment })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_application_controls_are_filtered_before_explicit_values_are_applied() {
        let configured = HashMap::from([("NOMIFUN_TASK_VALUE".into(), "explicit-user-value".into())]);
        let mut actual = inherited_environment([
            ("NOMIFUN_INTERNAL".into(), "internal".into()),
            ("CLAUDECODE".into(), "internal".into()),
            ("NOMIFUN_TASK_VALUE".into(), "inherited".into()),
            ("NODE_OPTIONS".into(), "--max-old-space-size=4096".into()),
        ], &configured).unwrap();
        for (key, entry) in configured { insert(&mut actual, &key, entry); }
        assert_eq!(actual.len(), 2);
        assert_eq!(actual["NODE_OPTIONS"], "--max-old-space-size=4096");
        assert_eq!(actual["NOMIFUN_TASK_VALUE"], "explicit-user-value");
    }

    #[cfg(unix)]
    #[test]
    fn inherited_non_utf8_environment_returns_a_private_typed_error_without_global_mutation() {
        use std::os::unix::ffi::OsStringExt;
        let private = OsString::from_vec(b"private-value-\xff".to_vec());
        for entries in [
            vec![(private.clone(), OsString::from("value"))],
            vec![(OsString::from("USER_TOOL_SETTING"), private.clone())],
        ] {
            let error = inherited_environment(entries, &HashMap::new()).unwrap_err();
            assert!(matches!(&error, ToolError::Io(error) if error.kind() == std::io::ErrorKind::InvalidData));
            assert!(!error.to_string().contains("private-value"));
            assert!(!format!("{error:?}").contains("private-value"));
        }
        // Filter app controls in their native encoding before decoding values.
        assert!(inherited_environment([(OsString::from("NOMIFUN_INTERNAL"), private.clone())], &HashMap::new()).unwrap().is_empty());
        assert!(inherited_environment([(OsString::from("USER_TOOL_SETTING"), private)],
            &HashMap::from([("USER_TOOL_SETTING".into(), "explicit".into())])).unwrap().is_empty());
    }

    #[test]
    fn explicit_configuration_and_locale_are_preserved_without_host_path_fallback() {
        let configured = HashMap::from([
            ("PATH".into(), "explicit-task-bin".into()),
            ("HOME".into(), "/real/task/home".into()),
            ("NODE_OPTIONS".into(), "--max-old-space-size=4096".into()),
            ("NPM_CONFIG_PREFIX".into(), "/user/chosen/prefix".into()),
            ("NODE_EXTRA_CA_CERTS".into(), "/user/cert.pem".into()),
            ("HTTPS_PROXY".into(), "https://user-proxy.invalid".into()),
            ("LC_CTYPE".into(), "zh_CN.UTF-8".into()),
            ("NOMIFUN_TASK_VALUE".into(), "explicit-user-value".into()),
        ]);
        let actual = process_environment(&configured).unwrap();
        for (key, entry) in &configured {
            if key != "PATH" { assert_eq!(value(&actual, key), Some(entry.as_str()), "{key}"); }
        }
        assert_eq!(std::env::split_paths(&actual["PATH"]).next(), Some(PathBuf::from("explicit-task-bin")));
        assert!(value(&actual, "LC_ALL").is_none());
        assert_eq!(configured.len(), 8);
    }

    #[test]
    fn persisted_npm_user_configuration_is_authoritative() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("npmrc");
        std::fs::write(&path, "registry=https://registry.invalid\nprefix = /user/prefix\n").unwrap();
        let configured = HashMap::from([("NPM_CONFIG_USERCONFIG".into(), path.to_string_lossy().into_owned())]);
        assert!(npm_user_prefix_configured(&configured));
        std::fs::write(&path, "# prefix=/comment\nregistry=https://registry.invalid\n").unwrap();
        assert!(!npm_user_prefix_configured(&configured));
    }

    #[tokio::test]
    async fn native_launch_keeps_empty_unicode_and_shell_metacharacters_literal() {
        let args = vec!["".into(), "有 空格".into(), "'\";$(printf injected)&".into()];
        let environment = HashMap::from([("PATH".into(), "".into())]);
        let result = prepare_process_launch("/usr/bin/printf", &args, Path::new("/"), &environment).await.unwrap();
        assert_eq!(result.program, "/usr/bin/printf");
        assert_eq!(result.args, args);
    }

    #[test]
    fn user_tools_do_not_live_in_runtime_cache() {
        let root = user_cli_root().unwrap();
        assert!(root.components().any(|part| part.as_os_str() == "nomifun-user-tools"));
        assert!(!root.starts_with(crate::runtime_root().unwrap()));
    }
}
