//! RemoteShell sentinel protocol against a real sshd: cwd/env persistence,
//! exit codes, cwd in the marker, and recoverable timeout.
mod support;

use std::time::Duration;

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
