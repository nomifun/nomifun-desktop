//! RemoteShell sentinel protocol against a real sshd: cwd/env persistence,
//! exit codes, cwd in the marker, recoverable timeout, and the disconnect /
//! close-with-evidence contracts the connection pool relies on.
mod support;

use std::{io::Write, process::{Command, Stdio}, time::Duration};

use nomi_ssh::connection::SshError;

const T: Duration = Duration::from_secs(8);

#[tokio::test(flavor = "multi_thread")]
async fn cwd_and_env_persist_across_commands() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let conn = support::connect(&sshd).await;
    let sh = conn.open_shell("/tmp").await.expect("open shell");

    let out = sh.run("echo hello_remote", T).await.expect("echo");
    assert_eq!(out.exit_code, 0, "output: {:?}", out.output);
    assert!(out.output.contains("hello_remote"), "got: {:?}", out.output);
    assert!(!out.timed_out);

    let uniq = format!("/tmp/nomi_shell_{}", std::process::id());
    sh.run(&format!("mkdir -p {uniq} && cd {uniq}"), T)
        .await
        .expect("cd");
    let pwd = sh.run("pwd", T).await.expect("pwd");
    assert!(
        pwd.output.contains(&uniq),
        "cwd must persist, got: {:?}",
        pwd.output
    );
    assert!(
        pwd.cwd.contains(&uniq),
        "marker must carry cwd, got: {:?}",
        pwd.cwd
    );

    sh.run("export NOMI_V=persisted_val", T).await.expect("export");
    let v = sh.run("echo $NOMI_V", T).await.expect("echo var");
    assert!(
        v.output.contains("persisted_val"),
        "env must persist, got: {:?}",
        v.output
    );

    // clean up
    let _ = sh.run(&format!("rm -rf {uniq}"), T).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_nonzero_exit_code() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    let out = sh.run("(exit 7)", T).await.expect("run");
    assert_eq!(out.exit_code, 7, "got: {:?}", out);
    assert!(!out.timed_out);
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_is_recoverable() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    let out = sh
        .run("sleep 30", Duration::from_millis(700))
        .await
        .expect("run");
    assert!(out.timed_out, "sleep 30 with 700ms budget must time out");
    // The shell must remain usable after a timeout.
    let after = sh.run("echo recovered", T).await.expect("post-timeout run");
    assert_eq!(after.exit_code, 0, "got: {:?}", after);
    assert!(
        after.output.contains("recovered"),
        "got: {:?}",
        after.output
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_run_retires_the_channel_instead_of_reusing_it() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();

    let cancelled = tokio::time::timeout(
        Duration::from_millis(100),
        sh.run("sleep 30", Duration::from_secs(30)),
    )
    .await;
    assert!(cancelled.is_err(), "outer cancellation budget must fire");
    assert!(
        !sh.is_reusable().await,
        "a cancelled command leaves an unknown remote outcome and its channel must be retired"
    );

    let next = sh.run("echo must_not_run", T).await;
    assert!(
        matches!(next, Err(SshError::Protocol(_))),
        "the next command must fail before submission so a caller can replace the channel: {next:?}"
    );
}

/// A shell that is gone is not a shell that is slow. Reporting the conventional
/// timeout code for a dead channel makes liveness detection and teardown
/// forensics impossible: the pool cannot tell "still running, be patient" from
/// "link is gone, redial".
#[tokio::test(flavor = "multi_thread")]
async fn shell_exit_reports_terminal_status_and_retires_the_channel() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();

    // `exit` prevents a sentinel, but the server still supplies its terminal
    // process status. That receipt must not make the channel reusable.
    let first = sh.run("exit", T).await;
    let second = sh.run("echo after_exit", T).await;

    for (label, result) in [("first", &first), ("second", &second)] {
        if let Ok(outcome) = result {
            assert!(
                !(outcome.timed_out && outcome.exit_code == 124),
                "{label} run dressed a dead shell up as a timeout: {outcome:?}"
            );
        }
    }
    assert!(matches!(first, Ok(ref outcome) if outcome.exit_code == 0 && outcome.cwd.is_empty() && !outcome.timed_out),
        "server exit-status must remain an exact terminal receipt: {first:?}");
    assert!(
        matches!(second, Err(SshError::Disconnected(_))),
        "a run against a dead shell must report Disconnected, got: {second:?}"
    );
}

/// The shell runs on a real PTY (sudo needs one), so every "page my output if
/// stdout is a tty" tool would start `less` and block forever on a keypress that
/// is never coming — turning `git log`, `systemctl status`, `journalctl` and
/// `man` into timeouts. Init must neutralise the pagers.
#[tokio::test(flavor = "multi_thread")]
async fn init_neutralises_the_remote_pagers() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    let out = sh
        .run(
            r#"printf 'PAGER=%s GIT_PAGER=%s SYSTEMD_PAGER=%s TERM=%s\n' "$PAGER" "$GIT_PAGER" "$SYSTEMD_PAGER" "$TERM""#,
            T,
        )
        .await
        .expect("run");
    assert_eq!(
        out.output.trim(),
        "PAGER=cat GIT_PAGER=cat SYSTEMD_PAGER=cat TERM=dumb",
        "init must export pager-neutralising values, got: {:?}",
        out.output
    );
    // And they must be exported, not just set, or a child process (git) would
    // never see them.
    let child = sh
        .run(r#"sh -c 'printf "%s,%s\n" "$GIT_PAGER" "$TERM"'"#, T)
        .await
        .expect("run child");
    assert!(
        child.output.contains("cat,dumb"),
        "the values must be exported to children, got: {:?}",
        child.output
    );
    // `TERM=dumb` must not cost us the PTY: sudo asks `isatty`, not `TERM`, and
    // the responder can only answer a prompt read from the terminal.
    let tty = sh
        .run(r#"if [ -t 0 ] && [ -t 1 ]; then echo STILL_A_TTY; fi"#, T)
        .await
        .expect("run tty check");
    assert!(
        tty.output.contains("STILL_A_TTY"),
        "the shell must keep its tty under TERM=dumb, got: {:?}",
        tty.output
    );
}

/// End-to-end proof on a real PTY: `git log` output longer than the pty is tall
/// starts `less` unless the pager is neutralised, and `less` then waits for a
/// keypress until the command budget runs out.
#[tokio::test(flavor = "multi_thread")]
async fn a_paging_command_completes_instead_of_hanging() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let available = match Command::new("git").arg("--version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP: git is unavailable");
            return;
        }
        Err(error) => panic!("probe git: {error}"),
    };
    assert!(available.status.success(), "git --version failed: {available:?}");

    // This sshd connects back to the same machine. Use its native temporary
    // filesystem, independent of contributor history and WSL /mnt/c latency.
    #[cfg(target_os = "linux")]
    let repo = tempfile::tempdir_in("/tmp").unwrap();
    #[cfg(not(target_os = "linux"))]
    let repo = tempfile::tempdir().unwrap();
    let initialized = fixture_git(repo.path())
        .args(["init", "--bare", "--quiet", "--template=", "."])
        .output()
        .unwrap();
    assert!(initialized.status.success(), "git init: {}", String::from_utf8_lossy(&initialized.stderr));
    let head = fixture_git(repo.path())
        .args(["symbolic-ref", "HEAD", "refs/heads/nomi-pager-test"])
        .output()
        .unwrap();
    assert!(head.status.success(), "git symbolic-ref: {}", String::from_utf8_lossy(&head.stderr));

    let mut history = String::new();
    for index in 1..=100 {
        let message = format!("pager fixture {index:03}\n");
        history.push_str(&format!(
            "commit refs/heads/nomi-pager-test\nmark :{index}\n\
             committer SSH Fixture <ssh-fixture@example.invalid> {} +0000\n\
             data {}\n{message}",
            1_700_000_000 + index,
            message.len(),
        ));
        if index > 1 {
            history.push_str(&format!("from :{}\n", index - 1));
        }
        history.push('\n');
    }
    history.push_str("done\n");
    let mut importer = fixture_git(repo.path())
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    importer.stdin.take().unwrap().write_all(history.as_bytes()).unwrap();
    let imported = importer.wait_with_output().unwrap();
    assert!(imported.status.success(), "git fast-import: {}", String::from_utf8_lossy(&imported.stderr));

    let sh = support::connect(&sshd).await.open_shell(repo.path().to_str().unwrap()).await.unwrap();

    let out = sh
        .run("git log --oneline -100", Duration::from_secs(5))
        .await
        .expect("git log");
    assert!(
        !out.timed_out,
        "git log must not be swallowed by a pager, got: {out:?}"
    );
    assert_eq!(out.exit_code, 0, "got: {out:?}");
    assert_eq!(
        out.output.lines().count(),
        100,
        "the full log must reach the caller, got: {:?}",
        out.output
    );
}

fn fixture_git(directory: &std::path::Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(directory)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0");
    for variable in [
        "GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_PARAMETERS", "GIT_CONFIG", "GIT_TEMPLATE_DIR",
    ] {
        command.env_remove(variable);
    }
    command
}

/// `is_reaped()` is the teardown verdict, so it must be backed by evidence from
/// the server: the channel closed AND the remote said how the shell ended.
#[tokio::test(flavor = "multi_thread")]
async fn close_proves_the_shell_was_reaped() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    sh.run("echo alive", T).await.expect("run before close");

    let proof = sh.close(T).await;
    assert!(proof.eof_sent, "close must send EOF, got: {proof:?}");
    assert!(
        proof.channel_closed,
        "close must observe the channel closing, got: {proof:?}"
    );
    assert!(
        proof.exit_status.is_some() || proof.exit_signal.is_some(),
        "close must capture how the shell ended, got: {proof:?}"
    );
    assert!(proof.is_reaped(), "got: {proof:?}");
}

/// Terminal evidence consumed by run remains owned by the channel lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn close_preserves_the_terminal_proof_already_observed_by_run() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    let terminal = sh.run("printf terminal_marker; exit 7", T).await.unwrap();
    assert_eq!(terminal.exit_code, 7);
    assert_eq!(terminal.output, "terminal_marker");
    let _ = sh.run("echo drained", T).await;

    let proof = sh.close(T).await;
    assert!(proof.is_reaped(), "run's exact terminal messages must survive until teardown: {proof:?}");
    assert_eq!(proof.exit_status, Some(7));
    assert_eq!(sh.close(T).await, proof, "repeated close must return the retained proof");
}
