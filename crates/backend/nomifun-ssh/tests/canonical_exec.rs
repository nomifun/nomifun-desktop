//! Exercise the product's exact ssh/exec authority and dispatch path, rather
//! than the raw SshBackend path (which never exercised its privilege fence).
mod support;

use std::{sync::Arc, time::Duration};

use nomifun_common::{ConversationId, SshHostId};
use nomifun_ssh::{
    AgentSshAuthority, AgentSshHostResource, SshActionContext, SshActionError, SshActionOwner,
    SshCommandOutput, SshExecInput, SshExternalActionStatus, SshFsReadInput, SshFsReadOutput,
    SshFsWriteInput, SshLinkKey, SshLinkPhase, SshResourceOperation, SshSudoInput,
};

const COMMAND_BUDGET: u64 = 8_000;
const RECOVERY_BUDGET: Duration = Duration::from_secs(15);

fn fixture() -> Option<support::sshd::TestSshd> {
    if !cfg!(target_os = "linux") {
        eprintln!("SKIP: canonical ssh/exec requires a Linux sshd fixture");
        return None;
    }
    let sshd = support::sshd::start_pubkey_sshd();
    if sshd.is_none() {
        eprintln!("SKIP: no usable sshd");
    }
    sshd
}

fn authority(harness: &support::PoolHarness, host: &SshHostId) -> AgentSshAuthority {
    AgentSshAuthority::bound(
        &harness.user_id,
        AgentSshHostResource::new(
            "canonical-ssh-fixture",
            &harness.user_id,
            host.clone(),
            "/",
            [
                SshResourceOperation::Execute,
                SshResourceOperation::Read,
                SshResourceOperation::Write,
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn context(harness: &support::PoolHarness) -> SshActionContext {
    SshActionContext {
        principal_id: harness.user_id.clone(),
        agent_session_id: ConversationId::new().as_str().to_owned(),
        operation_id: "canonical-ssh-test-turn".into(),
    }
}

async fn exec(
    owner: &SshActionOwner,
    authority: &AgentSshAuthority,
    context: &SshActionContext,
    command: &str,
) -> SshCommandOutput {
    owner
        .exec(
            authority,
            context,
            SshExecInput {
                command: command.into(),
                timeout_ms: COMMAND_BUDGET,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("canonical command {command:?}: {error}"))
}

async fn require_nonroot(
    sshd: &support::sshd::TestSshd,
    harness: &support::PoolHarness,
    host: &SshHostId,
    authority: &AgentSshAuthority,
    context: &SshActionContext,
) -> bool {
    if sshd.username != "root" {
        return true;
    }
    let result = SshActionOwner::new(harness.pool.clone())
        .exec(
            authority,
            context,
            SshExecInput {
                command: "echo MUST_NOT_EXECUTE".into(),
                timeout_ms: COMMAND_BUDGET,
            },
        )
        .await;
    assert!(
        matches!(result, Err(SshActionError::External(ref message)) if message.contains("non-root Linux")),
        "root login must be rejected before dispatch: {result:?}"
    );
    assert_eq!(
        harness.pool.active_link_count(),
        1,
        "rejected exec must not disrupt the filesystem connection for {host}"
    );
    harness.pool.shutdown_all().await;
    eprintln!(
        "SKIP persistence assertions: run the Linux fixture as a non-root user; root denial was verified"
    );
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_persists_shell_state_and_isolates_sessions() {
    let Some(sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let first = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &first).await {
        return;
    }
    let owner = SshActionOwner::new(harness.pool.clone());
    let changed = exec(&owner, &authority, &first, "cd /tmp && export NOMI_SSH_STATE='persisted 中文 value' && NOMI_LOCAL_STATE=local && nomi_shell_function() { printf 'function-state'; }").await;
    assert_eq!(
        changed.status,
        SshExternalActionStatus::Succeeded,
        "{changed:?}"
    );
    let state = exec(&owner, &authority, &first, "printf '%s|%s|%s|' \"$PWD\" \"$NOMI_SSH_STATE\" \"$NOMI_LOCAL_STATE\"; nomi_shell_function; /bin/sh -c 'printf \"|%s\" \"$NOMI_SSH_STATE\"'").await;
    assert_eq!(
        state.stdout.trim(),
        "/tmp|persisted 中文 value|local|function-state|persisted 中文 value"
    );
    let link = harness
        .pool
        .acquire(&harness.user_id, &first.agent_session_id, &host, "/")
        .await
        .unwrap();
    assert_eq!(
        link.last_cwd(),
        "/tmp",
        "canonical receipt must update reconnect cwd"
    );
    // Rebuilding an owner (as on a model switch) must keep the pooled process.
    let rebuilt = SshActionOwner::new(harness.pool.clone());
    assert_eq!(
        exec(
            &rebuilt,
            &authority,
            &first,
            "printf '%s' \"$NOMI_SSH_STATE\""
        )
        .await
        .stdout
        .trim(),
        "persisted 中文 value"
    );
    let second = context(&harness);
    let isolated = exec(
        &owner,
        &authority,
        &second,
        "printf '%s|%s' \"$PWD\" \"${NOMI_SSH_STATE-unset}\"",
    )
    .await;
    assert_eq!(
        isolated.stdout.trim(),
        "/|unset",
        "same host must not share session shell state"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_keeps_kernel_fence_and_requires_sudo_authority() {
    let Some(sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let context = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &context).await {
        return;
    }
    let owner = SshActionOwner::new(harness.pool.clone());
    for _ in 0..2 {
        let status = exec(&owner, &authority, &context, "grep '^NoNewPrivs:' /proc/$$/status; /bin/sh -c 'grep \"^NoNewPrivs:\" /proc/$$/status'; /usr/bin/id -u").await;
        let lines: Vec<_> = status.stdout.lines().collect();
        assert_eq!(lines.len(), 3, "{status:?}");
        assert!(
            lines[..2]
                .iter()
                .all(|line| line.split_whitespace().last() == Some("1")),
            "parent and child must be kernel fenced: {status:?}"
        );
        assert_ne!(
            lines[2].trim(),
            "0",
            "ordinary execution must remain non-root"
        );
    }
    let sudo = exec(
        &owner,
        &authority,
        &context,
        "if [ -x /usr/bin/sudo ]; then /usr/bin/sudo -n /usr/bin/id -u; else (exit 77); fi",
    )
    .await;
    assert_ne!(
        sudo.exit_code, 0,
        "setuid sudo must not elevate, even when passwordless: {sudo:?}"
    );
    assert!(!sudo.stdout.lines().any(|line| line.trim() == "0"));
    let denied = owner
        .sudo(
            &authority,
            &context,
            SshSudoInput {
                command: "id -u".into(),
                timeout_ms: COMMAND_BUDGET,
            },
        )
        .await;
    assert!(
        matches!(denied, Err(SshActionError::ResourceOperationDenied { .. })),
        "exec grant must not authorize sudo: {denied:?}"
    );
    let old_marker = exec(
        &owner,
        &authority,
        &context,
        "printf '__NOMIFUN_UNPRIVILEGED_EXEC_UNAVAILABLE__\\n'",
    )
    .await;
    assert_eq!(
        old_marker.exit_code, 0,
        "user output must not be interpreted as guard rejection"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_recovers_timeout_and_cancel_without_replaying_commands() {
    let Some(sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let context = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &context).await {
        return;
    }
    let owner = SshActionOwner::new(harness.pool.clone());
    exec(
        &owner,
        &authority,
        &context,
        "cd /tmp && export NOMI_RECOVERY=retained",
    )
    .await;
    let timeout = owner
        .exec(
            &authority,
            &context,
            SshExecInput {
                command: "sleep 30".into(),
                timeout_ms: 150,
            },
        )
        .await
        .unwrap();
    assert_eq!(timeout.status, SshExternalActionStatus::TimedOut);
    assert_eq!(
        exec(
            &owner,
            &authority,
            &context,
            "printf '%s|%s' \"$PWD\" \"$NOMI_RECOVERY\""
        )
        .await
        .stdout
        .trim(),
        "/tmp|retained",
        "successful resync must retain shell state"
    );
    let cancelled = tokio::time::timeout(
        Duration::from_millis(150),
        owner.exec(
            &authority,
            &context,
            SshExecInput {
                command: "cd / && sleep 30".into(),
                timeout_ms: 30_000,
            },
        ),
    )
    .await;
    assert!(cancelled.is_err());
    let recovered = exec(
        &owner,
        &authority,
        &context,
        "printf '%s|%s|' \"$PWD\" \"${NOMI_RECOVERY-unset}\"; grep '^NoNewPrivs:' /proc/$$/status",
    )
    .await;
    assert!(
        recovered.stdout.starts_with("/tmp|unset|"),
        "replaced shell restores only the last proven cwd, without replaying an unknown command: {recovered:?}"
    );
    assert_eq!(
        recovered.stdout.split_whitespace().last(),
        Some("1"),
        "replacement must also be fenced"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_concurrent_submissions_do_not_recycle_an_active_shell() {
    let Some(sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let context = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &context).await {
        return;
    }
    let owner = Arc::new(SshActionOwner::new(harness.pool.clone()));
    exec(
        &owner,
        &authority,
        &context,
        "export NOMI_CONCURRENT=before",
    )
    .await;
    let slow = {
        let owner = owner.clone();
        let authority = authority.clone();
        let context = context.clone();
        tokio::spawn(async move {
            exec(
                &owner,
                &authority,
                &context,
                "sleep 1; cd /tmp; export NOMI_CONCURRENT=after; printf done",
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queue_timeout = owner
        .exec(
            &authority,
            &context,
            SshExecInput {
                command: "export NOMI_CONCURRENT=must_not_execute".into(),
                timeout_ms: 50,
            },
        )
        .await;
    assert!(
        matches!(queue_timeout, Err(SshActionError::External(ref message)) if message.contains("command slot")),
        "queue timeout is a proven pre-dispatch rejection: {queue_timeout:?}"
    );
    let next = exec(
        &owner,
        &authority,
        &context,
        "printf '%s' \"$NOMI_CONCURRENT\"",
    )
    .await;
    assert_eq!(slow.await.unwrap().stdout.trim(), "done");
    assert_eq!(next.stdout.trim(), "after");
    assert!(
        !harness
            .events
            .status_phases()
            .iter()
            .any(|phase| phase == "degraded")
    );
    let link = harness
        .pool
        .acquire(&harness.user_id, &context.agent_session_id, &host, "/")
        .await
        .unwrap();
    assert_eq!(link.last_cwd(), "/tmp");
    let exiting = {
        let owner = owner.clone();
        let authority = authority.clone();
        let context = context.clone();
        tokio::spawn(async move { exec(&owner, &authority, &context, "sleep 1; exit 7").await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after_exit = exec(&owner, &authority, &context, "pwd").await;
    assert_eq!(exiting.await.unwrap().exit_code, 7);
    assert_eq!(
        after_exit.stdout.trim(),
        "/tmp",
        "queued calls must resolve the replacement handle after terminal settlement"
    );
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_reconnect_restores_confirmed_cwd_and_reapplies_fence() {
    let Some(mut sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let context = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &context).await {
        return;
    }
    let owner = SshActionOwner::new(harness.pool.clone());
    exec(
        &owner,
        &authority,
        &context,
        "cd /tmp && export NOMI_BEFORE_DISCONNECT=lost",
    )
    .await;
    let mut state = harness
        .pool
        .subscribe(&SshLinkKey::new(&context.agent_session_id, host))
        .unwrap();
    state.borrow_and_update();
    sshd.stop();
    support::collect_phases_until(&mut state, SshLinkPhase::Reconnecting, RECOVERY_BUDGET).await;
    sshd.restart().expect("restart fixture");
    support::collect_phases_until(&mut state, SshLinkPhase::Connected, RECOVERY_BUDGET).await;
    let recovered = exec(&owner, &authority, &context, "printf '%s|%s|' \"$PWD\" \"${NOMI_BEFORE_DISCONNECT-unset}\"; grep '^NoNewPrivs:' /proc/$$/status").await;
    assert!(recovered.stdout.starts_with("/tmp|unset|"), "{recovered:?}");
    assert_eq!(recovered.stdout.split_whitespace().last(), Some("1"));
    harness.pool.shutdown_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_exec_keeps_sftp_and_nonzero_receipts_working() {
    let Some(sshd) = fixture() else { return };
    let harness = support::harness(sshd.known_hosts_path(), support::brisk_tuning()).await;
    let host = harness.add_fixture_host(&sshd).await;
    let authority = authority(&harness, &host);
    let context = context(&harness);
    if !require_nonroot(&sshd, &harness, &host, &authority, &context).await {
        return;
    }
    let owner = SshActionOwner::new(harness.pool.clone());
    exec(&owner, &authority, &context, "cd /tmp").await;
    let path = format!(
        "/tmp/nomifun-canonical-ssh-{}.txt",
        context.agent_session_id
    );
    let content = "SFTP 中文内容\n";
    owner
        .fs_write(
            &authority,
            &context,
            SshFsWriteInput {
                path: path.clone(),
                content: content.into(),
            },
        )
        .await
        .unwrap();
    let read = owner
        .fs_read(
            &authority,
            &context,
            SshFsReadInput::Read { path: path.clone() },
        )
        .await
        .unwrap();
    assert_eq!(
        read,
        SshFsReadOutput::Text {
            content: content.into()
        }
    );
    assert_eq!(
        exec(&owner, &authority, &context, &format!("cat '{path}'"))
            .await
            .stdout,
        content.trim_end_matches('\n')
    );
    let nonzero = exec(&owner, &authority, &context, "(exit 7)").await;
    assert_eq!(nonzero.status, SshExternalActionStatus::ExitedNonZero);
    assert_eq!(nonzero.exit_code, 7);
    let exit = exec(
        &owner,
        &authority,
        &context,
        "printf terminal_exit_marker; exit 7",
    )
    .await;
    assert_eq!(exit.status, SshExternalActionStatus::ExitedNonZero);
    assert_eq!(exit.exit_code, 7);
    assert_eq!(
        exit.stdout, "terminal_exit_marker",
        "server exit-status is a terminal receipt even without a sentinel"
    );
    let after_exit = exec(
        &owner,
        &authority,
        &context,
        "pwd; grep '^NoNewPrivs:' /proc/$$/status",
    )
    .await;
    assert!(after_exit.stdout.starts_with("/tmp\n"));
    assert_eq!(after_exit.stdout.split_whitespace().last(), Some("1"));
    exec(&owner, &authority, &context, &format!("rm -- '{path}'")).await;
    harness.pool.shutdown_all().await;
}
