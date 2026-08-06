//! RemoteShell sentinel protocol against a real sshd: cwd/env persistence,
//! exit codes, cwd in the marker, recoverable timeout, and the disconnect /
//! close-with-evidence contracts the connection pool relies on.
mod support;

use std::time::Duration;

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

/// A shell that is gone is not a shell that is slow. Reporting the conventional
/// timeout code for a dead channel makes liveness detection and teardown
/// forensics impossible: the pool cannot tell "still running, be patient" from
/// "link is gone, redial".
#[tokio::test(flavor = "multi_thread")]
async fn run_reports_disconnect_when_the_shell_exits() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();

    // `exit` ends the remote shell, so this submission's sentinel can never
    // arrive — the channel closes instead.
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
    // The remote is this machine, so this crate's own checkout is a git repo
    // with plenty of history to page.
    let repo = env!("CARGO_MANIFEST_DIR");
    let sh = support::connect(&sshd).await.open_shell(repo).await.unwrap();
    let probe = sh
        .run("git rev-parse --is-inside-work-tree", Duration::from_secs(5))
        .await
        .expect("probe");
    if !probe.output.contains("true") {
        eprintln!("SKIP: no git / not a work tree here (honest skip): {probe:?}");
        return;
    }

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

/// The honest half of the contract: no evidence, no `reaped`.
#[tokio::test(flavor = "multi_thread")]
async fn close_after_the_shell_died_is_not_reaped() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    // Kill the shell out from under us, then drain it so the close path has
    // nothing left to learn from.
    let _ = sh.run("exit", T).await;
    let _ = sh.run("echo drained", T).await;

    let proof = sh.close(T).await;
    assert!(
        !proof.is_reaped(),
        "a shell that vanished before close must not be reported reaped: {proof:?}"
    );
    assert!(
        !proof.errors.is_empty() || proof.exit_status.is_none(),
        "an unproven close must say why it is unproven: {proof:?}"
    );
}
