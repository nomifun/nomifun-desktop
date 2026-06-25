//! Test-only helpers shared by the PTY/process unit tests.
//!
//! These tests must spawn a cross-platform child process (the `pty_test_helper`
//! binary built alongside this crate) instead of unix-only programs. Because the
//! tests live in `src/` (unit tests), `CARGO_BIN_EXE_pty_test_helper` is NOT
//! available — that env var is only injected for integration tests under
//! `tests/`. So we probe for the bin (see [`pty_test_helper_bin`]).

use std::path::PathBuf;

/// Absolute path to the `pty_test_helper` binary built with this crate.
///
/// Discovery must survive this repo's split build layout: `.cargo/config.toml`
/// sets `build-dir = {workspace-root}/build.noindex`, so the unit-test RUNNER
/// exe lives under `build.noindex/<profile>/deps/`, while the `[[bin]]` artifact
/// is hard-linked into the default target dir `target/<profile>/`. `CARGO_BIN_EXE_*`
/// is unavailable to `src/` unit tests, so we probe candidate locations and take
/// the first that exists (standard layout, split build-dir, and a manifest-root
/// derivation as a backstop).
pub(crate) fn pty_test_helper_bin() -> PathBuf {
    let bin_name = if cfg!(windows) { "pty_test_helper.exe" } else { "pty_test_helper" };
    let exe = std::env::current_exe().expect("current_exe");
    // current_exe = .../<profile>/deps/<test-runner>.exe  →  profile_dir = .../<profile>
    let profile_dir = exe
        .parent()
        .and_then(|deps| if deps.ends_with("deps") { deps.parent() } else { Some(deps) })
        .expect("profile dir")
        .to_path_buf();
    let profile = profile_dir.file_name().and_then(|s| s.to_str()).unwrap_or("debug").to_string();

    let mut candidates: Vec<PathBuf> = Vec::new();
    // 1) Standard cargo: bin sits alongside the profile dir (target/<profile>/bin).
    candidates.push(profile_dir.join(bin_name));
    // 2) Split build-dir: the runner is under build.noindex/<profile> but the bin
    //    artifact lands in the sibling target/<profile>. Map the path across.
    if let Some(s) = profile_dir.to_str() {
        if s.contains("build.noindex") {
            candidates.push(PathBuf::from(s.replacen("build.noindex", "target", 1)).join(bin_name));
        }
    }
    // 3) Backstop: derive from the workspace root via CARGO_MANIFEST_DIR
    //    (crates/agent/nomi-tools → agent → crates → <root>).
    if let Some(root) = PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3) {
        candidates.push(root.join("target").join(&profile).join(bin_name));
    }
    candidates
        .iter()
        .find(|c| c.exists())
        .cloned()
        .unwrap_or_else(|| panic!("pty_test_helper binary not found; tried: {candidates:?}"))
}

/// The helper path as a `String`, for use as a `PtyParams.program`.
pub(crate) fn pty_test_helper_program() -> String {
    pty_test_helper_bin().to_string_lossy().into_owned()
}

/// A shell command line that runs the helper with `subcommand` through the
/// platform shell (`cmd /C` / `sh -c`), for the `exec_command` / `write_stdin`
/// tools, which always wrap their `cmd` in the login shell.
///
/// Quoting differs by platform because of how `portable-pty` builds the child
/// command line on each OS:
///
/// - **Unix (`sh -c`)**: the helper path is double-quoted so spaces survive the
///   shell word-split. `sh` parses quotes correctly.
///
/// - **Windows (`cmd /C`)**: the path is emitted **unquoted**. `portable-pty`'s
///   `CommandBuilder` argv-quotes each arg using MSVCRT rules — any arg
///   containing a quote gets its `"` rewritten to `\"`. But `cmd /C` does NOT
///   understand argv `\"` escaping; it would treat `\"C:\path\helper.exe\"` as a
///   literal (quotes-in-name) program and fail with "is not recognized as an
///   internal or external command". With no embedded quotes, `portable-pty`
///   wraps the whole single arg in one outer quote pair (no inner escapes) and
///   `cmd /C` strips that pair cleanly, yielding a parseable command line.
///   This relies on the helper path containing **no spaces** — true for this
///   repo's controlled build layout (`target` / `build.noindex` under the
///   workspace root). A spaced path cannot be expressed correctly through the
///   `cmd /C` + `portable-pty` argv-escaping combination from a single command
///   string, so we accept that constraint here rather than mangle the quotes.
pub(crate) fn pty_test_helper_shell_cmd(subcommand: &str) -> String {
    let prog = pty_test_helper_program();
    if cfg!(windows) {
        debug_assert!(
            !prog.contains(' '),
            "Windows shell-wrapped helper path must be space-free (cmd /C + \
             portable-pty cannot carry a quoted path); got: {prog}"
        );
        format!("{prog} {subcommand}")
    } else {
        format!("\"{prog}\" {subcommand}")
    }
}
