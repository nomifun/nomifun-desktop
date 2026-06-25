use std::ffi::OsString;
use std::path::Path;

use nomifun_runtime::Builder as CmdBuilder;
use tracing::{info, warn};

use crate::constants::{
    LIFECYCLE_ON_ACTIVATE_TIMEOUT_SECS, LIFECYCLE_ON_DEACTIVATE_TIMEOUT_SECS, LIFECYCLE_ON_INSTALL_TIMEOUT_SECS,
    LIFECYCLE_ON_UNINSTALL_TIMEOUT_SECS,
};
use crate::error::ExtensionError;
use crate::types::LifecycleHooks;

/// Which lifecycle hook to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    OnInstall,
    OnUninstall,
    OnActivate,
    OnDeactivate,
}

impl HookKind {
    /// Default timeout in seconds for this hook kind.
    pub fn timeout_secs(self) -> u64 {
        match self {
            Self::OnInstall => LIFECYCLE_ON_INSTALL_TIMEOUT_SECS,
            Self::OnUninstall => LIFECYCLE_ON_UNINSTALL_TIMEOUT_SECS,
            Self::OnActivate => LIFECYCLE_ON_ACTIVATE_TIMEOUT_SECS,
            Self::OnDeactivate => LIFECYCLE_ON_DEACTIVATE_TIMEOUT_SECS,
        }
    }

    /// Human-readable label for logging and error messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::OnInstall => "onInstall",
            Self::OnUninstall => "onUninstall",
            Self::OnActivate => "onActivate",
            Self::OnDeactivate => "onDeactivate",
        }
    }
}

/// Resolve the hook script path from the manifest for a given hook kind.
pub fn resolve_hook_path(hooks: &LifecycleHooks, kind: HookKind) -> Option<&str> {
    let value = match kind {
        HookKind::OnInstall => hooks.on_install.as_deref(),
        HookKind::OnUninstall => hooks.on_uninstall.as_deref(),
        HookKind::OnActivate => hooks.on_activate.as_deref(),
        HookKind::OnDeactivate => hooks.on_deactivate.as_deref(),
    };
    value.filter(|s| !s.is_empty())
}

/// Map a hook script to the interpreter (program) and argument list used to
/// run it, dispatching on the script's file extension. This is the single
/// source of truth for how lifecycle hook scripts are executed across
/// platforms.
///
/// `CmdBuilder` resolves a bare program name (no path separators) through
/// `PATH` — including the Windows `.cmd`/`.ps1`/`.bat` shim fallbacks — so we
/// intentionally pass the interpreter as a bare name (`sh`, `cmd`,
/// `powershell` / `pwsh`) and the script path as an argument, rather than
/// spawning the script file directly as the program. Spawning a `.sh`/shebang
/// script directly is fatal on Windows (`CreateProcess` → `ERROR_BAD_EXE_FORMAT`).
///
/// Extension dispatch:
/// - `.sh`              → `sh <script>` (works on unix; on Windows via git-bash/MSYS `sh` on PATH).
/// - `.ps1`             → `pwsh -NoProfile -ExecutionPolicy Bypass -File <script>` on unix,
///                        `powershell -NoProfile -ExecutionPolicy Bypass -File <script>` on Windows.
/// - `.cmd` / `.bat`    → `cmd /C <script>` on Windows; on unix run directly (shebang) as a fallback.
/// - none / other       → unix: run directly (rely on shebang); Windows: `cmd /C <script>`.
///
/// `<script>` is always passed as the absolute path so the interpreter resolves
/// it regardless of the child's working directory.
fn hook_command(script: &Path) -> (OsString, Vec<OsString>) {
    let ext = script
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let script_arg: OsString = script.as_os_str().to_owned();

    match ext.as_deref() {
        Some("sh") => (OsString::from("sh"), vec![script_arg]),
        Some("ps1") => {
            let program = if cfg!(windows) { "powershell" } else { "pwsh" };
            (
                OsString::from(program),
                vec![
                    OsString::from("-NoProfile"),
                    OsString::from("-ExecutionPolicy"),
                    OsString::from("Bypass"),
                    OsString::from("-File"),
                    script_arg,
                ],
            )
        }
        Some("cmd") | Some("bat") => {
            if cfg!(windows) {
                (OsString::from("cmd"), vec![OsString::from("/C"), script_arg])
            } else {
                // No cmd.exe on unix; best effort is to exec the script directly
                // (a `.cmd` on unix is unusual but we honour the shebang if any).
                (script_arg, Vec::new())
            }
        }
        // No / unknown extension: unix executes directly via shebang; Windows
        // cannot exec a shebang script, so route through `cmd /C`.
        _ => {
            if cfg!(windows) {
                (OsString::from("cmd"), vec![OsString::from("/C"), script_arg])
            } else {
                (script_arg, Vec::new())
            }
        }
    }
}

/// Execute a lifecycle hook script in a child process.
///
/// - `ext_dir`: absolute path to the extension root directory (used as cwd).
/// - `hook_path`: script path relative to `ext_dir`.
/// - `kind`: which hook is being executed (determines timeout and label).
/// - `extension_name`: used for logging and error context.
///
/// Returns `Ok(())` on success. Returns an error if the script is not found,
/// times out, or exits with a non-zero status.
pub async fn execute_hook(
    ext_dir: &Path,
    hook_path: &str,
    kind: HookKind,
    extension_name: &str,
) -> Result<(), ExtensionError> {
    let script = ext_dir.join(hook_path);

    if !script.exists() {
        warn!(
            extension = extension_name,
            hook = kind.label(),
            path = %script.display(),
            "lifecycle hook script not found, skipping"
        );
        return Err(ExtensionError::HookNotFound(script.display().to_string()));
    }

    let timeout_secs = kind.timeout_secs();
    let label = kind.label();

    info!(
        extension = extension_name,
        hook = label,
        path = %script.display(),
        timeout_secs,
        "executing lifecycle hook"
    );

    // Select an interpreter by the script's file extension and pass the script
    // as an argument. Spawning the script path directly as the program fails on
    // Windows (`CreateProcess` cannot exec a `.sh`/shebang file). The interpreter
    // is a bare name so `CmdBuilder` resolves it through PATH (+ Windows shims).
    let (program, args) = hook_command(&script);
    let mut builder = CmdBuilder::clean_cli(&program);
    builder.args(&args);
    builder.current_dir(ext_dir);
    let child_future = builder.output();

    let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child_future).await;

    match result {
        Err(_elapsed) => {
            warn!(
                extension = extension_name,
                hook = label,
                timeout_secs,
                "lifecycle hook timed out"
            );
            Err(ExtensionError::HookTimeout {
                extension_name: extension_name.to_owned(),
                hook: label.to_owned(),
                timeout_secs,
            })
        }
        Ok(Err(io_err)) => {
            warn!(
                extension = extension_name,
                hook = label,
                error = %io_err,
                "lifecycle hook I/O error"
            );
            Err(ExtensionError::HookFailed {
                extension_name: extension_name.to_owned(),
                hook: label.to_owned(),
                reason: io_err.to_string(),
            })
        }
        Ok(Ok(output)) => {
            if output.status.success() {
                info!(
                    extension = extension_name,
                    hook = label,
                    "lifecycle hook completed successfully"
                );
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_owned(), |c| c.to_string());
                warn!(
                    extension = extension_name,
                    hook = label,
                    exit_code = %code,
                    stderr = %stderr,
                    "lifecycle hook exited with error"
                );
                Err(ExtensionError::HookFailed {
                    extension_name: extension_name.to_owned(),
                    hook: label.to_owned(),
                    reason: format!("exit code {code}: {}", stderr.trim()),
                })
            }
        }
    }
}

/// Determine whether the `onInstall` hook should run.
///
/// Returns `true` when:
/// - There is no persisted version (first-time install).
/// - The persisted version differs from the current manifest version.
pub fn needs_install_hook(current_version: &str, persisted_version: Option<&str>) -> bool {
    match persisted_version {
        None => true,
        Some(prev) => prev != current_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a lifecycle hook script with platform-appropriate syntax and
    /// return the file name (relative to `dir`) to hand to `execute_hook`.
    ///
    /// On Windows a `.cmd` batch file is written; elsewhere a `#!/bin/sh`
    /// script (made executable). `stem` is the file name without extension.
    /// `unix_body` / `windows_body` are the script bodies for each platform.
    fn write_hook(dir: &Path, stem: &str, unix_body: &str, windows_body: &str) -> String {
        #[cfg(windows)]
        {
            let name = format!("{stem}.cmd");
            // `@echo off` keeps the interpreter from echoing each command into
            // stdout, and CRLF line endings keep cmd.exe happy.
            let content = format!("@echo off\r\n{}\r\n", windows_body.replace('\n', "\r\n"));
            std::fs::write(dir.join(&name), content).unwrap();
            let _ = unix_body;
            name
        }
        #[cfg(not(windows))]
        {
            let name = format!("{stem}.sh");
            let full = dir.join(&name);
            std::fs::write(&full, format!("#!/bin/sh\n{unix_body}\n")).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let _ = windows_body;
            name
        }
    }

    // -----------------------------------------------------------------------
    // needs_install_hook
    // -----------------------------------------------------------------------

    #[test]
    fn test_needs_install_first_time() {
        assert!(needs_install_hook("1.0.0", None));
    }

    #[test]
    fn test_needs_install_version_changed() {
        assert!(needs_install_hook("2.0.0", Some("1.0.0")));
    }

    #[test]
    fn test_no_install_same_version() {
        assert!(!needs_install_hook("1.0.0", Some("1.0.0")));
    }

    #[test]
    fn test_needs_install_downgrade() {
        assert!(needs_install_hook("0.9.0", Some("1.0.0")));
    }

    // -----------------------------------------------------------------------
    // HookKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_hook_kind_timeout_values() {
        assert_eq!(HookKind::OnInstall.timeout_secs(), 120);
        assert_eq!(HookKind::OnUninstall.timeout_secs(), 60);
        assert_eq!(HookKind::OnActivate.timeout_secs(), 30);
        assert_eq!(HookKind::OnDeactivate.timeout_secs(), 30);
    }

    #[test]
    fn test_hook_kind_labels() {
        assert_eq!(HookKind::OnInstall.label(), "onInstall");
        assert_eq!(HookKind::OnUninstall.label(), "onUninstall");
        assert_eq!(HookKind::OnActivate.label(), "onActivate");
        assert_eq!(HookKind::OnDeactivate.label(), "onDeactivate");
    }

    // -----------------------------------------------------------------------
    // resolve_hook_path
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_hook_path_present() {
        let hooks = LifecycleHooks {
            on_install: Some("scripts/install.sh".into()),
            on_activate: Some("scripts/activate.sh".into()),
            on_deactivate: None,
            on_uninstall: None,
        };
        assert_eq!(
            resolve_hook_path(&hooks, HookKind::OnInstall),
            Some("scripts/install.sh")
        );
        assert_eq!(
            resolve_hook_path(&hooks, HookKind::OnActivate),
            Some("scripts/activate.sh")
        );
        assert_eq!(resolve_hook_path(&hooks, HookKind::OnDeactivate), None);
        assert_eq!(resolve_hook_path(&hooks, HookKind::OnUninstall), None);
    }

    #[test]
    fn test_resolve_hook_path_empty_string() {
        let hooks = LifecycleHooks {
            on_install: Some(String::new()),
            on_activate: None,
            on_deactivate: None,
            on_uninstall: None,
        };
        assert_eq!(resolve_hook_path(&hooks, HookKind::OnInstall), None);
    }

    // -----------------------------------------------------------------------
    // hook_command — interpreter dispatch (single source of truth)
    // -----------------------------------------------------------------------

    /// Collect `(program, args)` as plain `String`s for assertion convenience.
    fn dispatch(name: &str) -> (String, Vec<String>) {
        let (program, args) = hook_command(Path::new(name));
        (
            program.to_string_lossy().into_owned(),
            args.iter().map(|a| a.to_string_lossy().into_owned()).collect(),
        )
    }

    #[test]
    fn hook_command_sh_runs_via_sh() {
        let (program, args) = dispatch("/ext/scripts/install.sh");
        assert_eq!(program, "sh");
        assert_eq!(args, vec!["/ext/scripts/install.sh".to_owned()]);
    }

    #[test]
    fn hook_command_ps1_runs_via_powershell_with_file_flag() {
        let (program, args) = dispatch("/ext/scripts/setup.ps1");
        let expected_program = if cfg!(windows) { "powershell" } else { "pwsh" };
        assert_eq!(program, expected_program);
        // …-NoProfile -ExecutionPolicy Bypass -File <script>
        assert_eq!(args.first().map(String::as_str), Some("-NoProfile"));
        assert_eq!(args.get(1).map(String::as_str), Some("-ExecutionPolicy"));
        assert_eq!(args.get(2).map(String::as_str), Some("Bypass"));
        assert_eq!(args.get(3).map(String::as_str), Some("-File"));
        assert_eq!(args.get(4).map(String::as_str), Some("/ext/scripts/setup.ps1"));
    }

    #[test]
    fn hook_command_extension_is_case_insensitive() {
        // `.SH` must dispatch like `.sh`.
        let (program, _args) = dispatch("/ext/Install.SH");
        assert_eq!(program, "sh");
    }

    #[cfg(windows)]
    #[test]
    fn hook_command_cmd_and_bare_run_via_cmd_on_windows() {
        for name in ["C:/ext/install.cmd", "C:/ext/install.bat", "C:/ext/install"] {
            let (program, args) = dispatch(name);
            assert_eq!(program, "cmd", "name={name}");
            assert_eq!(args.first().map(String::as_str), Some("/C"), "name={name}");
            assert_eq!(args.get(1).map(String::as_str), Some(name));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn hook_command_bare_runs_directly_on_unix() {
        // No extension → execute directly (rely on shebang), no interpreter arg.
        let (program, args) = dispatch("/ext/scripts/install");
        assert_eq!(program, "/ext/scripts/install");
        assert!(args.is_empty());
    }

    // -----------------------------------------------------------------------
    // execute_hook (async unit tests)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_execute_hook_script_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = execute_hook(dir.path(), "nonexistent.sh", HookKind::OnActivate, "test-ext").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ExtensionError::HookNotFound(_)));
    }

    #[tokio::test]
    async fn test_execute_hook_success() {
        let dir = tempfile::tempdir().unwrap();
        let name = write_hook(dir.path(), "hook", "exit 0", "exit /b 0");

        let result = execute_hook(dir.path(), &name, HookKind::OnActivate, "test-ext").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_hook_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let name = write_hook(
            dir.path(),
            "fail",
            "echo 'something broke' >&2\nexit 1",
            "echo something broke 1>&2 & exit /b 1",
        );

        let result = execute_hook(dir.path(), &name, HookKind::OnInstall, "test-ext").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ExtensionError::HookFailed {
                extension_name,
                hook,
                reason,
            } => {
                assert_eq!(extension_name, "test-ext");
                assert_eq!(hook, "onInstall");
                assert!(reason.contains("something broke"));
            }
            other => panic!("expected HookFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_execute_hook_timeout() {
        let dir = tempfile::tempdir().unwrap();
        // A long-enough-to-outlive-the-deadline hook on each platform. The 200ms
        // deadline below must elapse before the process completes (Err == timeout),
        // but `CmdBuilder::output()` blocks the executor (not cooperatively
        // cancellable), so `timeout` only reports `Elapsed` AFTER the child exits —
        // keep the sleep short so the test stays ~1s while still comfortably
        // exceeding 200ms (so a fast spawn-failure regression still surfaces as Ok).
        let name = write_hook(
            dir.path(),
            "slow",
            "sleep 1",
            "ping -n 2 127.0.0.1 >NUL",
        );
        let script = dir.path().join(&name);
        assert!(script.exists());

        let (program, args) = hook_command(&script);
        let mut builder = CmdBuilder::clean_cli(&program);
        builder.args(&args);
        builder.current_dir(dir.path());

        let result = tokio::time::timeout(std::time::Duration::from_millis(200), builder.output()).await;

        // The deadline must elapse before the long-running process completes
        // (Err == timeout). On a fast Windows `CreateProcess` failure this
        // would instead resolve immediately and `result` would be Ok — which
        // is exactly the regression this guards against.
        assert!(result.is_err(), "should have timed out");
    }

    #[tokio::test]
    async fn test_execute_hook_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("cwd_marker.txt");
        // print cwd to a file: unix `pwd`, cmd `cd` with no args prints cwd.
        let name = write_hook(
            dir.path(),
            "check_cwd",
            "pwd > cwd_marker.txt",
            "cd > cwd_marker.txt",
        );

        let result = execute_hook(dir.path(), &name, HookKind::OnActivate, "test-ext").await;

        assert!(result.is_ok());
        assert!(marker.exists());
        let cwd_content = std::fs::read_to_string(&marker).unwrap();
        // The cwd written by the script should match the extension dir
        // (may have symlink resolution differences, compare canonical)
        let expected = dir.path().canonicalize().unwrap();
        let actual_trimmed = cwd_content.trim();
        let actual = Path::new(actual_trimmed).canonicalize().unwrap();
        assert_eq!(actual, expected);
    }
}
