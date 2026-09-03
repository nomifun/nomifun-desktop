//! CLI argument definitions for the `nomicore` binary.
//!
//! Kept separate from `main.rs` to isolate the clap surface (struct + enum +
//! attribute soup) from the runtime entry point. Visibility is `pub(crate)`
//! because only `main.rs` consumes it.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// The default data directory shared by all hosts built for the same channel
/// (desktop shell, `nomifun-web`, the `nomicore` bin): the per-user
/// application-data dir joined with `NomiFun<channel-suffix>`. Stable builds
/// use `NomiFun`; non-stable builds use a suffixed sibling such as
/// `NomiFun-dev`. Extreme fallback when the OS reports no user dir:
/// `<system temp>/nomifun-data<channel-suffix>`.
///
/// Sharing within a channel is deliberate, while isolating non-stable channels
/// in sibling directories prevents development loops from touching
/// installed-app state — and no channel ever nests inside the stable data
/// root. The `NOMIFUN_DATA_DIR` env / `--data-dir` flag remain the escape
/// hatch for an explicitly selected directory; the env value is the FINAL
/// data root on every host (the desktop shell no longer appends anything).
/// Concurrent use of one dir is prevented by the exclusive server lock (see
/// `bootstrap::server_lock`).
///
/// Historical self-export locations are recognized by
/// `bootstrap::data_root`, but Fresh-v4 never migrates their entries. The
/// existing canonical root is isolated only by the one-time whole-root
/// cutover before a new v4 root is created.
///
/// This is only the *unset* default — it does NOT consult `NOMIFUN_DATA_DIR`
/// itself (clap's `env` binding and the desktop shell resolve the env).
pub fn default_data_dir() -> PathBuf {
    let leaf = vendor_leaf(&crate::channel::dir_suffix());
    dirs::data_local_dir()
        .map(|dir| dir.join(leaf))
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join(fallback_leaf(&crate::channel::dir_suffix()))
        })
}

/// The pre-0.3.4 default data directory for the active channel:
/// `<app-data>/NomiFun/Nomi<channel-suffix>` (or the historic temp fallback
/// `<system temp>/nomifun-data/Nomi<channel-suffix>`). Retained only for
/// inherited self-export normalization; Fresh-v4 does not open or migrate
/// entries from this path.
pub fn legacy_default_data_dir() -> PathBuf {
    let leaf = legacy_nomi_leaf(&crate::channel::dir_suffix());
    dirs::data_local_dir()
        .map(|dir| dir.join("NomiFun"))
        .unwrap_or_else(|| std::env::temp_dir().join("nomifun-data"))
        .join(leaf)
}

/// The data-dir leaf for the active build channel: `NomiFun` on stable,
/// `NomiFun-dev` (etc.) on non-stable channels. The channel suffix attaches to
/// the vendor directory itself, so a non-stable build lands in a *sibling*
/// directory next to the production one (`…/NomiFun-dev`) — never inside the
/// stable `NomiFun` data root. Pure, for unit testing.
fn vendor_leaf(suffix: &str) -> String {
    format!("NomiFun{suffix}")
}

/// Temp-dir fallback leaf mirroring [`vendor_leaf`] channel isolation.
fn fallback_leaf(suffix: &str) -> String {
    format!("nomifun-data{suffix}")
}

/// The pre-0.3.4 leaf under the `NomiFun` vendor directory (`Nomi`,
/// `Nomi-dev`, …). Retained only for inherited-path normalization.
fn legacy_nomi_leaf(suffix: &str) -> String {
    format!("Nomi{suffix}")
}

/// Reject empty `--data-dir` / `NOMIFUN_DATA_DIR` values. clap's env binding
/// takes an empty env var (a common `.env` slip) literally, which would
/// resolve the data dir to `""` — scattering a `./logs` dir into the CWD
/// before failing cryptically. `NOMIFUN_WORK_DIR` already gets the same
/// non-empty filter in `bootstrap::work_dir`.
pub fn parse_non_empty_path(s: &str) -> Result<PathBuf, String> {
    if s.trim().is_empty() {
        return Err(
            "must not be empty (unset NOMIFUN_DATA_DIR instead of setting it to an empty string)"
                .into(),
        );
    }
    Ok(PathBuf::from(s))
}

#[derive(Parser)]
#[command(name = "nomicore", about = "Nomi Backend Server", version)]
pub struct Cli {
    /// Host address to listen on.
    #[arg(long, default_value_t = String::from(nomifun_common::constants::DEFAULT_HOST))]
    pub host: String,

    /// Port number to listen on.
    #[arg(long, default_value_t = nomifun_common::constants::DEFAULT_PORT)]
    pub port: u16,

    /// Data directory for database and file storage.
    #[arg(long, env = "NOMIFUN_DATA_DIR", default_value_os_t = default_data_dir(), value_parser = parse_non_empty_path)]
    pub data_dir: PathBuf,

    /// Working directory for conversation workspaces.
    /// Falls back to NOMIFUN_WORK_DIR env, then to data-dir.
    #[arg(long)]
    pub work_dir: Option<PathBuf>,

    /// Host application version used for extension engine compatibility.
    #[arg(long, default_value_t = env!("CARGO_PKG_VERSION").to_string())]
    pub app_version: String,

    /// Run in local embedded mode (skip authentication and use the
    /// database-resolved installation owner).
    #[arg(long)]
    pub local: bool,

    /// Directory for log files. Defaults to {data-dir}/logs/.
    #[arg(long)]
    pub log_dir: Option<PathBuf>,

    /// Log level filter (e.g. "info", "debug", "info,nomifun_mcp=trace").
    #[arg(long)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

// `Mcp` prefix is load-bearing on Mcp* variants — clap derives kebab-case
// subcommand names (`mcp-requirement-stdio`, etc.) that external callers
// (ACP agent CLI, injected MCP bridge specs) depend on verbatim.
#[derive(Subcommand)]
pub enum Command {
    /// MCP stdio server for AutoWork requirement declaration tools
    /// (`requirement_complete` / `requirement_update_status`; spawned by the ACP agent CLI).
    McpRequirementStdio,
    /// MCP stdio server for the per-session knowledge-search tool
    /// (`knowledge_search`; spawned by the ACP agent CLI when knowledge bases are
    /// mounted into the session).
    McpKnowledgeStdio,
    /// MCP stdio server for the Platform Gateway tools (`nomi_*` — conversations,
    /// cron jobs, global memory, requirements; spawned by agent sessions that
    /// receive a process-issued scoped capability).
    McpGatewayStdio,
    /// MCP stdio server exposing a single reliable `open` tool (URL / file /
    /// folder / application via ShellExecute; spawned by the ACP agent CLI on
    /// Windows so the agent stops launching apps with fragile `cmd /c start`).
    McpOpenStdio,
    /// One-shot terminal lifecycle hook relay (invoked by claude/codex native
    /// hooks; reads the event JSON from stdin and POSTs it to the in-process
    /// TerminalLifecycleServer). NOT an MCP server — fire-and-forget.
    TerminalHook {
        /// Lifecycle kind: turn_end | tool_use | notification | session_start.
        #[arg(long)]
        event: String,
    },
    /// Self-check: hydrate the agent registry, probe every CLI on `$PATH`,
    /// and print a per-agent availability table. Useful when the user
    /// reports "no agent works" — running this from the same shell the
    /// app launched from confirms whether each backend is detectable
    /// before involving server logs.
    Doctor,
    /// Use the canonical installation-owner Remote API. Every operation is
    /// explicit and session-bound; there is no generic capability/Registry
    /// dispatch or implicit recent-session state.
    Remote {
        #[command(subcommand)]
        operation: RemoteCommand,
    },
    /// Create a complete offline backup bundle from the current data/work directories.
    ///
    /// The command acquires the same exclusive server lock used by the backend,
    /// so it refuses to race a running instance. It includes the database,
    /// persistent encryption key, companion files, and only backend-managed
    /// `<work-dir>/conversations` workspaces. Custom external workspaces, logs,
    /// and caches are excluded. The output must be outside both source roots.
    /// The bundle contains credentials and must be protected as sensitive data.
    Backup {
        /// Destination directory for the new backup bundle (must not exist).
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore a complete offline backup bundle into a new data directory.
    ///
    /// The destination must be absent or empty; existing data is never
    /// overwritten. Entity IDs, encryption key, companion files, and managed
    /// conversation workspaces are restored below the new data directory while
    /// storage-generation is rotated. Custom external workspaces are not in the
    /// bundle and must be restored separately by their owner.
    Restore {
        /// Source backup bundle directory.
        #[arg(long)]
        bundle: PathBuf,
        /// Destination data directory (must be absent or empty).
        #[arg(long = "destination-data-dir")]
        destination_data_dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Open an AgentSession from an owner-scoped RemoteBinding.
    Open {
        /// RemoteBinding identity created by the local Agent settings API.
        binding_id: String,
        /// Optional canonical JSON value admitted as the initial turn input.
        #[arg(long)]
        initial_input: Option<String>,
        /// Stable retry identity. When omitted, the CLI prints and uses a new key.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Instance base URL (default `$NOMIFUN_URL` or http://127.0.0.1:25808).
        #[arg(long)]
        url: Option<String>,
        /// Installation access token (default `$NOMIFUN_ACCESS_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
    /// Start one turn on an explicitly identified AgentSession.
    Turn {
        /// Canonical UUIDv7 AgentSession identity returned by `remote open`.
        agent_session_id: String,
        /// Canonical JSON turn input.
        input: String,
        /// Stable retry identity. When omitted, the CLI prints and uses a new key.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Instance base URL (default `$NOMIFUN_URL` or http://127.0.0.1:25808).
        #[arg(long)]
        url: Option<String>,
        /// Installation access token (default `$NOMIFUN_ACCESS_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
    /// Observe canonical Session events and message projections after a cursor.
    Observe {
        /// Canonical UUIDv7 AgentSession identity.
        agent_session_id: String,
        /// Exclusive Session event cursor.
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        /// Maximum number of events to return.
        #[arg(long, default_value_t = 100)]
        limit: u32,
        /// Instance base URL (default `$NOMIFUN_URL` or http://127.0.0.1:25808).
        #[arg(long)]
        url: Option<String>,
        /// Installation access token (default `$NOMIFUN_ACCESS_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
    /// Cancel the active turn on an explicitly identified AgentSession.
    Cancel {
        /// Canonical UUIDv7 AgentSession identity.
        agent_session_id: String,
        /// Stable retry identity. When omitted, the CLI prints and uses a new key.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Instance base URL (default `$NOMIFUN_URL` or http://127.0.0.1:25808).
        #[arg(long)]
        url: Option<String>,
        /// Installation access token (default `$NOMIFUN_ACCESS_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use clap::error::ErrorKind;
    use std::path::PathBuf;

    use super::{Cli, Command, RemoteCommand};

    #[test]
    fn default_data_dir_matches_active_channel() {
        // Pure shape check on the unset default — env handling belongs to clap
        // (`env = "NOMIFUN_DATA_DIR"`) and is not exercised here to keep the
        // test independent of the ambient environment.
        let dir = super::default_data_dir();
        let user_leaf = super::vendor_leaf(&crate::channel::dir_suffix());
        let fallback_leaf = super::fallback_leaf(&crate::channel::dir_suffix());
        assert!(
            dir.is_absolute(),
            "default data dir must be absolute, got {dir:?}"
        );
        assert!(
            dir.ends_with(&user_leaf) || dir.ends_with(&fallback_leaf),
            "default data dir should end with {user_leaf:?} (or {fallback_leaf:?}), got {dir:?}"
        );
    }

    #[test]
    fn stable_data_root_is_plain_nomifun_vendor_dir() {
        assert_eq!(super::vendor_leaf(""), "NomiFun");
        assert_eq!(super::fallback_leaf(""), "nomifun-data");
    }

    #[test]
    fn non_stable_channels_get_sibling_vendor_dirs() {
        // The channel suffix must attach to the vendor dir itself, yielding a
        // SIBLING of the production dir (`…/NomiFun-dev`) — never a
        // subdirectory inside the stable `NomiFun` data root.
        assert_eq!(super::vendor_leaf("-dev"), "NomiFun-dev");
        assert_eq!(super::vendor_leaf("-beta"), "NomiFun-beta");
        assert_eq!(super::fallback_leaf("-dev"), "nomifun-data-dev");
    }

    #[test]
    fn legacy_default_keeps_the_historic_nomi_leaf_for_normalization() {
        let legacy = super::legacy_default_data_dir();
        let leaf = super::legacy_nomi_leaf(&crate::channel::dir_suffix());
        assert!(
            legacy.ends_with(std::path::Path::new("NomiFun").join(&leaf))
                || legacy.ends_with(std::path::Path::new("nomifun-data").join(&leaf)),
            "legacy default should end with NomiFun/{leaf}, got {legacy:?}"
        );
        assert_ne!(
            legacy,
            super::default_data_dir(),
            "legacy and current defaults must remain distinguishable"
        );
    }

    #[test]
    fn long_version_flag_uses_workspace_package_version() {
        let result = Cli::try_parse_from(["nomicore", "--version"]);
        let err = match result {
            Ok(_) => panic!("expected --version to exit through clap DisplayVersion"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        let rendered = err.to_string();
        assert!(
            rendered.contains("nomicore"),
            "version output should contain binary name, got: {rendered:?}"
        );
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "version output should contain package version {}, got: {rendered:?}",
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn short_version_flag_uses_workspace_package_version() {
        let result = Cli::try_parse_from(["nomicore", "-V"]);
        let err = match result {
            Ok(_) => panic!("expected -V to exit through clap DisplayVersion"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        let rendered = err.to_string();
        assert!(
            rendered.contains("nomicore"),
            "version output should contain binary name, got: {rendered:?}"
        );
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "version output should contain package version {}, got: {rendered:?}",
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn backup_subcommand_parses_output_and_data_dir() {
        let cli = Cli::try_parse_from([
            "nomicore",
            "--data-dir",
            "/source-data",
            "backup",
            "--output",
            "/backups/backup-1",
        ])
        .unwrap();
        assert_eq!(cli.data_dir, PathBuf::from("/source-data"));
        assert!(matches!(
            cli.command,
            Some(Command::Backup { output }) if output == PathBuf::from("/backups/backup-1")
        ));
    }

    #[test]
    fn restore_subcommand_parses_bundle_and_destination() {
        let cli = Cli::try_parse_from([
            "nomicore",
            "restore",
            "--bundle",
            "/backups/backup-1",
            "--destination-data-dir",
            "/restored-data",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Restore {
                bundle,
                destination_data_dir,
            }) if bundle == PathBuf::from("/backups/backup-1")
                && destination_data_dir == PathBuf::from("/restored-data")
        ));
    }

    #[test]
    fn backup_and_restore_require_their_paths() {
        let backup = match Cli::try_parse_from(["nomicore", "backup"]) {
            Ok(_) => panic!("backup without --output must fail"),
            Err(error) => error,
        };
        assert_eq!(backup.kind(), ErrorKind::MissingRequiredArgument);

        let restore =
            match Cli::try_parse_from(["nomicore", "restore", "--bundle", "/bundle"]) {
                Ok(_) => panic!("restore without --destination-data-dir must fail"),
                Err(error) => error,
            };
        assert_eq!(restore.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn backup_and_restore_long_help_state_portable_scope() {
        let command = Cli::command();
        let backup = command
            .find_subcommand("backup")
            .unwrap()
            .clone()
            .render_long_help()
            .to_string();
        assert!(backup.contains("Custom external workspaces"));
        assert!(backup.contains("logs"));
        assert!(backup.contains("caches"));
        assert!(backup.contains("sensitive data"));

        let restore = command
            .find_subcommand("restore")
            .unwrap()
            .clone()
            .render_long_help()
            .to_string();
        assert!(restore.contains("Custom external workspaces"));
        assert!(restore.contains("storage-generation"));
    }

    #[test]
    fn canonical_remote_subcommands_parse_without_legacy_generic_dispatch() {
        let open = Cli::try_parse_from([
            "nomicore",
            "remote",
            "open",
            "binding-1",
            "--initial-input",
            r#"{"text":"hello"}"#,
            "--idempotency-key",
            "open-1",
        ])
        .unwrap();
        assert!(matches!(
            open.command,
            Some(Command::Remote {
                operation: RemoteCommand::Open {
                    binding_id,
                    initial_input: Some(initial_input),
                    idempotency_key: Some(idempotency_key),
                    ..
                }
            }) if binding_id == "binding-1"
                && initial_input == r#"{"text":"hello"}"#
                && idempotency_key == "open-1"
        ));

        let observe = Cli::try_parse_from([
            "nomicore",
            "remote",
            "observe",
            "0190f5fe-7c00-7a00-8000-000000000001",
            "--after-seq",
            "7",
            "--limit",
            "25",
        ])
        .unwrap();
        assert!(matches!(
            observe.command,
            Some(Command::Remote {
                operation: RemoteCommand::Observe {
                    after_seq: 7,
                    limit: 25,
                    ..
                }
            })
        ));

        let command = Cli::command();
        assert!(command.find_subcommand("tools").is_none());
        assert!(command.find_subcommand("call").is_none());
    }
}
