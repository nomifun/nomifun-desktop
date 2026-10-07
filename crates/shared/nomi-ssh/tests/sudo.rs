//! Prompt-driven auto-answer (sudo) against a real sshd. Uses a pure-sh prompt
//! that reads from the tty, so no fake sudo binary is required.
mod support;

use std::time::Duration;

use nomi_ssh::responder::AnswerRule;
use zeroize::Zeroizing;

const T: Duration = Duration::from_secs(8);
const PROMPT: &str = "__NOMIFUN_SUDO_TEST_PROMPT__:";

#[tokio::test(flavor = "multi_thread")]
async fn answer_rule_injects_password_and_it_never_appears_in_output() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let conn = support::connect(&sshd).await;
    let rules = vec![AnswerRule::exact_once(
        PROMPT,
        Zeroizing::new("test_sudo_pw".to_string()),
    )
    .unwrap()];
    let sh = conn
        .open_shell_with_rules("/tmp", rules)
        .await
        .expect("open shell with rules");

    // A pure-sh stand-in for sudo: print the exact sudo prompt, read a line from
    // the tty (which the responder must supply), and echo OK iff it matches.
    let script = format!(
        "printf '{}'; read -r pw; if [ \"$pw\" = \"test_sudo_pw\" ]; then echo INJECT_OK; else echo INJECT_BAD; fi",
        PROMPT
    );
    let out = sh.run(&script, T).await.expect("run");

    assert_eq!(out.exit_code, 0, "got: {:?}", out);
    assert!(
        out.output.contains("INJECT_OK"),
        "responder should have supplied the password, got: {:?}",
        out.output
    );
    assert!(
        !out.output.contains("test_sudo_pw"),
        "the injected password must never appear in captured output, got: {:?}",
        out.output
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_rules_prompt_is_not_answered() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let sh = support::connect(&sshd)
        .await
        .open_shell("/tmp")
        .await
        .unwrap();
    // No rules: the read blocks, so a short timeout must fire (proving nothing
    // auto-answered) and the shell must recover.
    let script = r#"printf '[sudo] password for tester: '; read -r pw; echo done"#;
    let out = sh.run(script, Duration::from_millis(800)).await.expect("run");
    assert!(out.timed_out, "no rule → read blocks → must time out, got: {:?}", out);
    let after = sh.run("echo recovered", T).await.expect("post");
    assert!(after.output.contains("recovered"), "got: {:?}", after.output);
}

#[tokio::test(flavor = "multi_thread")]
async fn unprivileged_execution_cannot_use_installed_credential_responders() {
    if !cfg!(target_os = "linux") {
        eprintln!("SKIP: unprivileged execution requires Linux");
        return;
    }
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    if sshd.username == "root" {
        eprintln!("SKIP: run the unprivileged responder fixture as a non-root user");
        return;
    }
    let shell = support::connect(&sshd)
        .await
        .open_shell_with_rules("/tmp", vec![AnswerRule::exact_once(PROMPT, Zeroizing::new("must_not_be_injected".into())).unwrap()])
        .await
        .unwrap();
    let command = format!("printf '{PROMPT}'; read -r password; printf 'ANSWERED:%s' \"$password\"");
    let output = shell.run_unprivileged(&command, Duration::from_millis(800)).await.unwrap();
    assert!(output.timed_out, "ordinary execution must ignore even installed rules: {output:?}");
    assert!(!output.output.contains("must_not_be_injected"));
    assert_eq!(shell.run_unprivileged("printf recovered", T).await.unwrap().output, "recovered");
}
