//! Integration tests for lifecycle hooks (test-plan LH-1..LH-6).
//!
//! These tests exercise `execute_hook`, `needs_install_hook`, and
//! `resolve_hook_path` as black-box functions, verifying first install,
//! version change, activate/deactivate execution, timeout behaviour,
//! and graceful handling of missing scripts.

use std::fs;
use std::path::Path;

use nomifun_extension::{HookKind, LifecycleHooks, execute_hook, needs_install_hook, resolve_hook_path};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a platform-appropriate lifecycle hook script and return its path
/// relative to `dir` (including the platform-correct extension) to hand to
/// `execute_hook`.
///
/// `rel_stem` is the relative path WITHOUT a file extension (e.g.
/// `"scripts/install"`). On Windows a `.cmd` batch file is written using
/// `windows_body`; elsewhere a `#!/bin/sh` script (made executable) using
/// `unix_body`. `execute_hook`'s extension→interpreter dispatch then picks the
/// matching interpreter (`cmd /C` vs `sh`).
fn write_script(dir: &Path, rel_stem: &str, unix_body: &str, windows_body: &str) -> String {
    #[cfg(windows)]
    let rel_path = format!("{rel_stem}.cmd");
    #[cfg(not(windows))]
    let rel_path = format!("{rel_stem}.sh");

    let full = dir.join(&rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }

    #[cfg(windows)]
    {
        // `@echo off` stops the interpreter from echoing commands into stdout;
        // CRLF endings keep cmd.exe happy.
        let content = format!("@echo off\r\n{}\r\n", windows_body.replace('\n', "\r\n"));
        fs::write(&full, content).unwrap();
        let _ = unix_body;
    }
    #[cfg(not(windows))]
    {
        fs::write(&full, format!("#!/bin/sh\n{unix_body}\n")).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&full, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = windows_body;
    }

    rel_path
}

fn setup_ext_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

// ---------------------------------------------------------------------------
// LH-1: First install executes onInstall
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh1_first_install_executes_on_install() {
    let dir = setup_ext_dir();
    let marker = dir.path().join("installed.marker");
    let rel = write_script(
        dir.path(),
        "scripts/install",
        &format!("touch '{}'", marker.display()),
        &format!("type nul > \"{}\"", marker.display()),
    );

    let hooks = LifecycleHooks {
        on_install: Some(rel),
        ..Default::default()
    };

    // First install: no persisted version
    assert!(needs_install_hook("1.0.0", None));

    let hook_path = resolve_hook_path(&hooks, HookKind::OnInstall).unwrap();
    let result = execute_hook(dir.path(), hook_path, HookKind::OnInstall, "test-ext").await;
    assert!(result.is_ok());
    assert!(marker.exists(), "onInstall marker file should be created");
}

// ---------------------------------------------------------------------------
// LH-2: Version change executes onInstall
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh2_version_change_executes_on_install() {
    let dir = setup_ext_dir();
    let marker = dir.path().join("upgraded.marker");
    let rel = write_script(
        dir.path(),
        "scripts/install",
        &format!("touch '{}'", marker.display()),
        &format!("type nul > \"{}\"", marker.display()),
    );

    let hooks = LifecycleHooks {
        on_install: Some(rel),
        ..Default::default()
    };

    // Version changed from 1.0.0 to 2.0.0
    assert!(needs_install_hook("2.0.0", Some("1.0.0")));

    let hook_path = resolve_hook_path(&hooks, HookKind::OnInstall).unwrap();
    let result = execute_hook(dir.path(), hook_path, HookKind::OnInstall, "test-ext").await;
    assert!(result.is_ok());
    assert!(marker.exists(), "onInstall marker should be created on upgrade");
}

// ---------------------------------------------------------------------------
// LH-2 (negative): Same version does NOT trigger onInstall
// ---------------------------------------------------------------------------

#[test]
fn lh2_same_version_skips_install() {
    assert!(!needs_install_hook("1.0.0", Some("1.0.0")));
}

// ---------------------------------------------------------------------------
// LH-3: Each activation executes onActivate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh3_activate_executes_on_activate() {
    let dir = setup_ext_dir();
    let counter_file = dir.path().join("activate_count.txt");
    // Append a line on each activation to count calls
    let rel = write_script(
        dir.path(),
        "scripts/activate",
        &format!("echo 'activated' >> '{}'", counter_file.display()),
        &format!("echo activated >> \"{}\"", counter_file.display()),
    );

    let hooks = LifecycleHooks {
        on_activate: Some(rel),
        ..Default::default()
    };

    let hook_path = resolve_hook_path(&hooks, HookKind::OnActivate).unwrap();

    // Activate twice
    execute_hook(dir.path(), hook_path, HookKind::OnActivate, "test-ext")
        .await
        .unwrap();
    execute_hook(dir.path(), hook_path, HookKind::OnActivate, "test-ext")
        .await
        .unwrap();

    let content = fs::read_to_string(&counter_file).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "onActivate should run on each activation");
}

// ---------------------------------------------------------------------------
// LH-4: Deactivation executes onDeactivate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh4_deactivate_executes_on_deactivate() {
    let dir = setup_ext_dir();
    let marker = dir.path().join("deactivated.marker");
    let rel = write_script(
        dir.path(),
        "scripts/deactivate",
        &format!("touch '{}'", marker.display()),
        &format!("type nul > \"{}\"", marker.display()),
    );

    let hooks = LifecycleHooks {
        on_deactivate: Some(rel),
        ..Default::default()
    };

    let hook_path = resolve_hook_path(&hooks, HookKind::OnDeactivate).unwrap();
    let result = execute_hook(dir.path(), hook_path, HookKind::OnDeactivate, "test-ext").await;
    assert!(result.is_ok());
    assert!(marker.exists(), "onDeactivate marker should be created");
}

// ---------------------------------------------------------------------------
// LH-5: Hook timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh5_hook_timeout() {
    let dir = setup_ext_dir();
    // A hook that outlives the short deadline below on each platform: unix
    // `sleep`, Windows `ping` to localhost (a portable busy-wait — Windows has
    // no `sleep`). `CmdBuilder::output()` blocks the executor (not cooperatively
    // cancellable), so the wrapping `timeout` only reports `Elapsed` after the
    // child exits — keep it short (~1s) so the test is fast while still
    // comfortably exceeding the 200ms deadline.
    let rel = write_script(
        dir.path(),
        "scripts/slow",
        "sleep 1",
        "ping -n 2 127.0.0.1 >NUL",
    );

    let hooks = LifecycleHooks {
        on_activate: Some(rel.clone()),
        ..Default::default()
    };
    let hook_path = resolve_hook_path(&hooks, HookKind::OnActivate).unwrap();
    assert_eq!(hook_path, rel);
    assert!(dir.path().join(&rel).exists());

    // `execute_hook`'s built-in timeout (30s+) is far too long for a unit
    // test, so we wrap our own short deadline around it. The hook never
    // finishes within 200ms, so the deadline elapses (Err == timed out).
    // Critically, this routes through `execute_hook` → the real interpreter
    // dispatch + spawn — on the old code a Windows `.sh` spawn failed
    // instantly and this would resolve before the deadline (Ok), the exact
    // regression this guards.
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        execute_hook(dir.path(), hook_path, HookKind::OnActivate, "test-ext"),
    )
    .await;
    assert!(result.is_err(), "should time out before script completes");
}

// ---------------------------------------------------------------------------
// LH-6: Hook script does not exist — graceful handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lh6_missing_script_graceful() {
    let dir = setup_ext_dir();

    let hooks = LifecycleHooks {
        on_activate: Some("nonexistent.sh".into()),
        ..Default::default()
    };

    let hook_path = resolve_hook_path(&hooks, HookKind::OnActivate).unwrap();
    let result = execute_hook(dir.path(), hook_path, HookKind::OnActivate, "test-ext").await;

    assert!(result.is_err());
    match result.unwrap_err() {
        nomifun_extension::ExtensionError::HookNotFound(path) => {
            assert!(
                path.contains("nonexistent.sh"),
                "error should mention the missing script path"
            );
        }
        other => panic!("expected HookNotFound, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Additional: resolve_hook_path returns None when hook is not declared
// ---------------------------------------------------------------------------

#[test]
fn resolve_hook_path_none_when_not_declared() {
    let hooks = LifecycleHooks::default();
    assert!(resolve_hook_path(&hooks, HookKind::OnInstall).is_none());
    assert!(resolve_hook_path(&hooks, HookKind::OnUninstall).is_none());
    assert!(resolve_hook_path(&hooks, HookKind::OnActivate).is_none());
    assert!(resolve_hook_path(&hooks, HookKind::OnDeactivate).is_none());
}

// ---------------------------------------------------------------------------
// Additional: Hook script exits with non-zero status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_nonzero_exit_returns_hook_failed() {
    let dir = setup_ext_dir();
    let rel = write_script(
        dir.path(),
        "scripts/fail",
        "echo 'setup failed' >&2; exit 42",
        "echo setup failed 1>&2 & exit /b 42",
    );

    let result = execute_hook(dir.path(), &rel, HookKind::OnInstall, "failing-ext").await;

    assert!(result.is_err());
    match result.unwrap_err() {
        nomifun_extension::ExtensionError::HookFailed {
            extension_name,
            hook,
            reason,
        } => {
            assert_eq!(extension_name, "failing-ext");
            assert_eq!(hook, "onInstall");
            assert!(reason.contains("42"), "should include exit code");
            assert!(reason.contains("setup failed"), "should include stderr");
        }
        other => panic!("expected HookFailed, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Additional: Hook uses working directory correctly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hook_working_directory_is_ext_dir() {
    let dir = setup_ext_dir();
    // print cwd to a file: unix `pwd`, cmd `cd` with no args prints cwd.
    let rel = write_script(dir.path(), "check_dir", "pwd > cwd_out.txt", "cd > cwd_out.txt");

    let result = execute_hook(dir.path(), &rel, HookKind::OnActivate, "cwd-ext").await;
    assert!(result.is_ok());

    let cwd_file = dir.path().join("cwd_out.txt");
    assert!(cwd_file.exists());
    let cwd = fs::read_to_string(&cwd_file).unwrap();
    let expected = dir.path().canonicalize().unwrap();
    let actual = std::path::Path::new(cwd.trim()).canonicalize().unwrap();
    assert_eq!(actual, expected);
}
