//! Self-healing for `bun x`-spawned ACP agents whose exec cache got wedged.
//!
//! Builtin ACP rows spawn as `bun x <pkg>@<ver>` (bun stages the package in a
//! per-package "bunx" directory under its temp root, then execs the bin).
//! Two observed failure states require deleting that staging directory:
//!
//! 1. An install killed mid-flight (e.g. our initialize-handshake timeout
//!    firing during a cold download) can leave `bun.lock` + a partial
//!    `node_modules` without the `.bin` entry. Every later `bun x` of the same
//!    pinned version then no-ops the install and fails with
//!    `could not determine executable to run` — it never self-heals.
//! 2. Any other partially-extracted state from an aborted install.
//!
//! Purging is safe and cheap: bun's *download* cache (`BUN_INSTALL_CACHE_DIR`,
//! a sibling `bun-cache` dir) is untouched, so a re-created staging dir is
//! rebuilt from already-downloaded tarballs when they completed.
//!
//! `spawn_for_sdk` sets `BUN_TMPDIR = {data_dir}/bun-tmp` for agent children,
//! so the bunx staging dirs normally live there; the OS temp dir is swept as
//! well in case an older bun ignored the override.

use std::path::{Path, PathBuf};

use nomifun_common::CommandSpec;
use tracing::{info, warn};

/// Extract the `pkg[@version]` spec from a `bun x`-style command, if this
/// command is one. Returns `None` for non-bun commands (direct CLIs like
/// `gemini --experimental-acp`) so callers skip recovery entirely.
pub(super) fn bunx_package_spec(spec: &CommandSpec) -> Option<String> {
    let file_name = spec.command.file_stem()?.to_string_lossy().to_ascii_lowercase();
    if file_name != "bun" && file_name != "bunx" {
        return None;
    }
    let mut args = spec.args.iter().map(String::as_str);
    if file_name == "bun" {
        // Skip through to the `x` subcommand.
        loop {
            match args.next() {
                Some("x") => break,
                Some(_) => continue,
                None => return None,
            }
        }
    }
    // First non-flag arg after `x` is the package spec.
    args.find(|arg| !arg.starts_with('-')).map(str::to_owned)
}

/// The bare package name (`codex-acp` for `@zed-industries/codex-acp@0.14.0`)
/// used to match bunx staging directory names. Scope and version are dropped
/// because bun's directory-name encoding of scoped packages is an internal
/// detail; a substring match on the base name is stable across bun versions.
fn package_base_name(package_spec: &str) -> Option<String> {
    let without_version = match package_spec.rfind('@') {
        // A leading '@' is the scope marker, not a version separator.
        Some(idx) if idx > 0 => &package_spec[..idx],
        _ => package_spec,
    };
    let base = without_version.rsplit('/').next()?.trim();
    if base.is_empty() { None } else { Some(base.to_owned()) }
}

/// Best-effort removal of every bunx staging directory for `package_spec`
/// under the agent temp roots. Never fails the caller: each candidate is
/// logged and errors are swallowed (the next spawn re-creates state anyway).
pub(super) async fn purge_bunx_cache(data_dir: &Path, package_spec: &str) {
    let Some(base_name) = package_base_name(package_spec) else {
        return;
    };
    let roots: Vec<PathBuf> = vec![data_dir.join("bun-tmp"), std::env::temp_dir()];
    for root in roots {
        let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("bunx-") || !name.contains(&base_name) {
                continue;
            }
            let path = entry.path();
            match tokio::fs::remove_dir_all(&path).await {
                Ok(()) => info!(
                    path = %path.display(),
                    package = package_spec,
                    "Purged bunx staging dir after failed agent start"
                ),
                Err(e) => warn!(
                    path = %path.display(),
                    package = package_spec,
                    error = %e,
                    "Failed to purge bunx staging dir"
                ),
            }
        }
    }
}

/// The bun failure signature of a wedged bunx staging dir. Matched against
/// the child's stderr tail on startup crashes to trigger the purge.
pub(super) fn stderr_indicates_wedged_bunx(stderr: &str) -> bool {
    stderr
        .to_ascii_lowercase()
        .contains("could not determine executable to run")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::EnvVar;

    fn spec(command: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            command: PathBuf::from(command),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            env: Vec::<EnvVar>::new(),
            cwd: None,
        }
    }

    #[test]
    fn extracts_scoped_package_from_bun_x() {
        let s = spec("bun", &["x", "--bun", "@zed-industries/codex-acp@0.14.0"]);
        assert_eq!(
            bunx_package_spec(&s).as_deref(),
            Some("@zed-industries/codex-acp@0.14.0")
        );
    }

    #[test]
    fn extracts_package_from_absolute_bun_path() {
        let s = spec(
            "C:/data/runtime/bun-1.2.0-abc123/bun.exe",
            &["x", "--bun", "@agentclientprotocol/claude-agent-acp@0.33.1"],
        );
        assert_eq!(
            bunx_package_spec(&s).as_deref(),
            Some("@agentclientprotocol/claude-agent-acp@0.33.1")
        );
    }

    #[test]
    fn non_bun_commands_yield_none() {
        assert_eq!(bunx_package_spec(&spec("gemini", &["--experimental-acp"])), None);
        assert_eq!(bunx_package_spec(&spec("codex-acp", &[])), None);
    }

    #[test]
    fn bun_without_x_subcommand_yields_none() {
        assert_eq!(bunx_package_spec(&spec("bun", &["run", "dev"])), None);
    }

    #[test]
    fn base_name_strips_scope_and_version() {
        assert_eq!(
            package_base_name("@zed-industries/codex-acp@0.14.0").as_deref(),
            Some("codex-acp")
        );
        assert_eq!(package_base_name("cowsay@1.6.0").as_deref(), Some("cowsay"));
        assert_eq!(package_base_name("cowsay").as_deref(), Some("cowsay"));
        assert_eq!(package_base_name("@scope/pkg").as_deref(), Some("pkg"));
        assert_eq!(package_base_name(""), None);
    }

    #[test]
    fn wedge_signature_matches_case_insensitively() {
        assert!(stderr_indicates_wedged_bunx(
            "error: could not determine executable to run for package @zed-industries/codex-acp"
        ));
        assert!(stderr_indicates_wedged_bunx(
            "Error: Could Not Determine Executable To Run"
        ));
        assert!(!stderr_indicates_wedged_bunx("Resolving dependencies"));
    }

    #[tokio::test]
    async fn purge_removes_matching_bunx_dirs_only() {
        let dir = tempfile::tempdir().unwrap();
        let bun_tmp = dir.path().join("bun-tmp");
        let wedged = bun_tmp.join("bunx-501-codex-acp@0.14.0");
        let other = bun_tmp.join("bunx-501-claude-agent-acp@0.33.1");
        let not_bunx = bun_tmp.join("something-codex-acp");
        for p in [&wedged, &other, &not_bunx] {
            tokio::fs::create_dir_all(p).await.unwrap();
        }

        purge_bunx_cache(dir.path(), "@zed-industries/codex-acp@0.14.0").await;

        assert!(!wedged.exists(), "matching bunx dir must be removed");
        assert!(other.exists(), "other packages' bunx dirs must survive");
        assert!(not_bunx.exists(), "non-bunx dirs must survive");
    }

    #[tokio::test]
    async fn purge_tolerates_missing_bun_tmp() {
        let dir = tempfile::tempdir().unwrap();
        // No bun-tmp dir at all — must not panic or error.
        purge_bunx_cache(dir.path(), "@zed-industries/codex-acp@0.14.0").await;
    }
}
