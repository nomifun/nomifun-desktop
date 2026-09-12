//! Execute the real command builder in an isolated directory, not a second
//! hand-written quoting implementation. No SSH host or real user file is used.
use super::*;
use std::{path::Path, process::Command};

fn shell() -> Option<std::path::PathBuf> {
    #[cfg(unix)]
    {
        Some("sh".into())
    }
    #[cfg(windows)]
    {
        let path = std::path::PathBuf::from("C:/Program Files/Git/usr/bin/sh.exe");
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("SKIP POSIX command execution: Git for Windows sh is unavailable");
            None
        }
    }
}

fn listing(shell: &Path, dir: &Path, glob: &str) -> Vec<String> {
    validate_backend_path(glob).unwrap();
    list_lines(&execute(shell, dir, &list_command(glob)))
}

fn execute(shell: &Path, dir: &Path, script: &str) -> String {
    let mut command = Command::new(shell);
    command
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("QUOTING_STYLE", "shell-always")
        .args(["-c", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn glob_redirection_cannot_overwrite_an_unrelated_file() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input"), b"fixture").unwrap();
    std::fs::write(dir.path().join("victim"), b"keep this content").unwrap();
    listing(&shell, dir.path(), "input > victim");
    assert_eq!(
        std::fs::read(dir.path().join("victim")).unwrap(),
        b"keep this content"
    );
}

#[test]
fn glob_literal_spaces_do_not_split_into_extra_patterns() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for name in ["has space.rs", "unrelated.rs"] {
        std::fs::write(dir.path().join(name), b"").unwrap();
    }
    assert_eq!(listing(&shell, dir.path(), "has *.rs"), ["has space.rs"]);
}

#[test]
fn glob_leading_dash_cannot_become_an_ls_option() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("--help"), b"").unwrap();
    assert_eq!(listing(&shell, dir.path(), "--help"), ["--help"]);
}

#[test]
fn escaped_shell_metacharacters_keep_glob_matching() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for (name, glob) in [
        ("dollar$one.rs", "dollar$*.rs"),
        ("semi;one.rs", "semi;*.rs"),
        ("a'b-one.rs", "a'b-*.rs"),
        ("paren(one).rs", "paren(*).rs"),
        ("#one.rs", "#*.rs"),
        ("中文-one.rs", "中文-*.rs"),
    ] {
        std::fs::write(dir.path().join(name), b"").unwrap();
        assert_eq!(listing(&shell, dir.path(), glob), [name]);
    }
}

#[test]
fn ordinary_wildcards_ranges_and_no_match_keep_their_meaning() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "d.txt"] {
        std::fs::write(dir.path().join(name), b"").unwrap();
    }
    assert_eq!(listing(&shell, dir.path(), "[a-c].rs"), ["a.rs", "b.rs"]);
    assert_eq!(listing(&shell, dir.path(), "[!a].rs"), ["b.rs"]);
    assert_eq!(listing(&shell, dir.path(), "?.rs"), ["a.rs", "b.rs"]);
    assert!(listing(&shell, dir.path(), "missing-*.rs").is_empty());
}

#[test]
fn listing_does_not_change_the_persistent_shell_loop_variable() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.rs"), b"").unwrap();
    let script = format!(
        "_nomi_glob_path=preserved; {}; printf '%s\\n' \"$_nomi_glob_path\"",
        list_command("*.rs")
    );
    assert_eq!(execute(&shell, dir.path(), &script), "one.rs\npreserved\n");
}

#[cfg(unix)]
#[test]
fn broken_symlinks_are_still_glob_entries() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("missing-target", dir.path().join("broken-link")).unwrap();
    assert_eq!(listing(&shell, dir.path(), "broken-*"), ["broken-link"]);
}
#[test]
fn command_result_translation_rejects_partial_timeout_output() {
    let output = RemoteCommandOutput {
        stdout: "partial match".into(),
        exit_code: 0,
        timed_out: true,
    };
    assert!(command_stdout(output, "search").is_err());
}

#[test]
fn command_result_translation_rejects_nonzero_exit_status() {
    let output = RemoteCommandOutput {
        stdout: "permission denied".into(),
        exit_code: 2,
        timed_out: false,
    };
    assert!(command_stdout(output, "listing").is_err());
}

#[test]
fn successful_empty_and_nonempty_results_are_preserved() {
    for stdout in ["", "a.rs:2:found\n"] {
        let out = RemoteCommandOutput {
            stdout: stdout.into(),
            exit_code: 0,
            timed_out: false,
        };
        assert_eq!(command_stdout(out, "search").unwrap(), stdout);
    }
}

#[test]
fn no_match_does_not_retry_with_a_different_search_engine() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "rg() {{ return 1; }}; grep() {{ printf 'unexpected-fallback'; return 0; }}; {}",
        grep_command("fixture", ".")
    );
    assert_eq!(execute(&shell, dir.path(), &script), "");
}

#[test]
fn search_engine_errors_are_not_hidden_or_retried() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "rg() {{ printf 'synthetic invalid regex\\n' >&2; return 2; }}; grep() {{ printf 'unexpected-fallback\\n'; return 0; }}; {}; _fixture_status=$?; printf 'status=%s' \"$_fixture_status\"",
        grep_command("[", ".")
    );
    assert_eq!(execute(&shell, dir.path(), &script), "status=2");
}
#[test]
fn unavailable_ripgrep_uses_extended_grep_and_literal_dash_file() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("-"), b"first\nsecond\nthird\n").unwrap();
    let script = format!(
        "PATH=/fixture-no-executables; grep() {{ /usr/bin/grep \"$@\"; }}; {}",
        grep_command("first|second", "-")
    );
    assert_eq!(execute(&shell, dir.path(), &script), "1:first\n2:second\n");
}

#[test]
fn missing_search_tools_preserve_failure_and_diagnostics() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "PATH=/fixture-no-executables; {} 2>&1; _fixture_status=$?; printf 'status=%s' \"$_fixture_status\"",
        grep_command("fixture", ".")
    );
    let output = execute(&shell, dir.path(), &script);
    assert!(output.ends_with("status=127"), "{output}");
    assert!(
        output.contains("grep"),
        "missing-tool diagnostic was lost: {output}"
    );
}

#[test]
fn search_subshell_preserves_positional_parameters_status_and_errexit() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "set -e; set -- kept; _nomi_search_status=preserved; rg() {{ return 1; }}; {}; printf '%s:%s' \"$1\" \"$_nomi_search_status\"",
        grep_command("fixture", ".")
    );
    assert_eq!(execute(&shell, dir.path(), &script), "kept:preserved");
}

#[test]
fn grep_fallback_preserves_no_match_invalid_regex_and_quoted_path() {
    let Some(shell) = shell() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a'b"), "it's here\n").unwrap();
    for (pattern, status, expected) in [
        ("missing", 0, ""),
        ("[", 2, ""),
        ("it's", 0, "1:it's here\n"),
    ] {
        let script = format!(
            "PATH=/fixture-no-executables; grep() {{ /usr/bin/grep \"$@\"; }}; {}; _fixture_status=$?; printf 'status=%s' \"$_fixture_status\"",
            grep_command(pattern, "a'b")
        );
        assert_eq!(
            execute(&shell, dir.path(), &script),
            format!("{expected}status={status}")
        );
    }
}
