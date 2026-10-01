#![cfg(any(unix, windows))]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::io;

use nomi_process_runtime::{
    CapabilityPolicy, CommandSpec, ProcessError, ProcessOutcome, ProcessOwner,
    ProcessPolicy, NormalizedProcessRequest, OutputCursor, PollResult, ProcessSupervisor,
    SupervisorConfig, Transport,
};
#[cfg(target_os = "macos")]
use nomi_process_runtime::SandboxPolicy;
#[cfg(any(windows, target_os = "macos"))]
use nomi_process_runtime::ShellKind;

fn helper_binary() -> &'static str {
    env!("CARGO_BIN_EXE_process_test_helper")
}

#[cfg(unix)]
fn low_fd_harness_binary() -> &'static str {
    env!("CARGO_BIN_EXE_low_fd_harness")
}

#[cfg(unix)]
fn fd_sentinel_harness_binary() -> &'static str {
    env!("CARGO_BIN_EXE_fd_sentinel_harness")
}

fn request(program: impl Into<OsString>, args: impl IntoIterator<Item = OsString>) -> NormalizedProcessRequest {
    let cwd = std::env::current_dir().expect("current directory should exist");
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: program.into(),
            args: args.into_iter().collect(),
        },
        cwd: cwd.clone(),
        env: BTreeMap::new(),
        transport: Transport::Pipe,
        policy: ProcessPolicy::default(),
        capability: CapabilityPolicy::local_owner(cwd),
    }
}

fn helper_request(args: &[&str]) -> NormalizedProcessRequest {
    request(
        helper_binary(),
        args.iter().map(OsString::from).collect::<Vec<_>>(),
    )
}

async fn wait_for_terminal(
    supervisor: &ProcessSupervisor,
    handle: &nomi_process_runtime::ProcessHandle,
) -> ProcessOutcome {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        supervisor.poll(
            &handle.owner,
            &handle.session_id,
            OutputCursor::START,
            Instant::now() + Duration::from_secs(30),
        ),
    )
    .await
    .expect("terminal poll must stay bounded")
    .expect("terminal poll should succeed");
    match result {
        PollResult::Finished(outcome) => outcome,
        PollResult::Running { .. } => panic!("helper should have exited before the bounded poll"),
    }
}

#[tokio::test]
#[cfg_attr(unix, serial_test::serial(unix_process_contract))]
async fn output_arrival_wakes_a_running_poll_before_the_yield_deadline() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    #[cfg(unix)]
    let process = request("/bin/cat", Vec::<OsString>::new());
    #[cfg(windows)]
    let process = helper_request(&["echo-stdin"]);
    let handle = supervisor
        .start(process)
        .await
        .expect("echo process should start");
    let began = Instant::now();
    let poll = supervisor.poll_until_activity(
        &handle.owner,
        &handle.session_id,
        OutputCursor::START,
        Instant::now() + Duration::from_secs(5),
    );
    tokio::pin!(poll);

    tokio::time::sleep(Duration::from_millis(25)).await;
    supervisor
        .write(&handle.owner, &handle.session_id, b"wake-on-output\n")
        .await
        .expect("stdin write should succeed");
    let result = tokio::time::timeout(Duration::from_secs(1), &mut poll)
        .await
        .expect("output should wake the poll")
        .expect("poll should succeed");
    let PollResult::Running { output, .. } = result else {
        panic!("echo helper should still be running");
    };

    assert!(began.elapsed() < Duration::from_secs(1));
    assert_eq!(output.raw_bytes(), b"wake-on-output\n");
    supervisor
        .close_stdin(&handle.owner, &handle.session_id)
        .await
        .expect("closing stdin should succeed");
    let _ = wait_for_terminal(&supervisor, &handle).await;
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn unix_pipe_preserves_zero_and_nonzero_exit_codes() {
    for expected in [0, 7] {
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let process = request(
            "/bin/sh",
            [
                OsString::from("-c"),
                OsString::from(format!("exit {expected}")),
            ],
        );
        let handle = supervisor
            .start(process)
            .await
            .expect("Unix quick-exit shell should start");

        let quick_exit_bound = if cfg!(target_os = "macos") {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(250)
        };
        let poll_started = Instant::now();
        let outcome = tokio::time::timeout(
            quick_exit_bound,
            wait_for_terminal(&supervisor, &handle),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("quick natural exit must wake a far-yield poll within {quick_exit_bound:?}")
        });
        assert!(poll_started.elapsed() < quick_exit_bound);
        let ProcessOutcome::Exited { code, signal, .. } = outcome else {
            panic!("helper exit should produce Exited, got {outcome:?}");
        };
        assert_eq!(code, Some(expected));
        assert_eq!(signal, None);
    }
}

#[tokio::test]
#[cfg(target_os = "macos")]
#[serial_test::serial(unix_process_contract)]
async fn macos_concurrent_quick_shells_each_commit_and_report_their_exit() {
    let long_supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let long_handle = long_supervisor
        .start(request("/bin/cat", Vec::<OsString>::new()))
        .await
        .expect("long-running peer should start");
    let mut starts = tokio::task::JoinSet::new();
    for index in 0..16 {
        starts.spawn(async move {
            let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
            let mut process = request("unused", []);
            process.command = CommandSpec::Shell {
                shell: ShellKind::Posix,
                script: "printf '%s' \"$NOMIFUN_MACOS_SHELL_VALUE\"; exit 7".to_owned(),
            };
            let value = format!("quick-shell-{index}");
            process.env.insert(
                OsString::from("NOMIFUN_MACOS_SHELL_VALUE"),
                OsString::from(&value),
            );
            let handle = supervisor
                .start(process)
                .await
                .unwrap_or_else(|error| panic!("quick shell {index} failed to start: {error:?}"));
            let outcome = wait_for_terminal(&supervisor, &handle).await;
            let ProcessOutcome::Exited {
                code,
                output,
                cleanup,
                ..
            } = outcome
            else {
                panic!("quick shell {index} did not exit truthfully: {outcome:?}");
            };
            assert_eq!(code, Some(7));
            assert_eq!(output.text(), value);
            assert!(cleanup.reaped);
        });
    }
    while let Some(result) = starts.join_next().await {
        result.expect("quick-shell task must not panic");
    }
    long_supervisor
        .close_stdin(&long_handle.owner, &long_handle.session_id)
        .await
        .expect("long-running peer stdin should close");
    let outcome = wait_for_terminal(&long_supervisor, &long_handle).await;
    assert!(matches!(
        outcome,
        ProcessOutcome::Exited {
            code: Some(0),
            ..
        }
    ));
}

#[tokio::test]
#[cfg_attr(unix, serial_test::serial(unix_process_contract))]
async fn elapsed_process_deadline_rejects_start_before_user_code_runs() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let marker = directory.path().join("must-not-run.marker");
    let mut process = request(
        helper_binary(),
        [
            OsString::from("write-file"),
            marker.as_os_str().to_owned(),
        ],
    );
    process.cwd = directory.path().canonicalize().expect("canonical cwd");
    process.capability = CapabilityPolicy::local_owner(process.cwd.clone());
    process.policy.deadline = Some(Instant::now());
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());

    let error = supervisor
        .start(process)
        .await
        .expect_err("elapsed deadline must reject start");

    assert_eq!(error.code(), "spawn_failed");
    assert!(!marker.exists());
}

#[tokio::test]
#[cfg_attr(unix, serial_test::serial(unix_process_contract))]
async fn running_deadline_preserves_partial_file_effect_and_reports_reaped_timeout() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let marker = directory.path().join("partial-effect.marker");
    let mut process = helper_request(&[
        "write-file-then-sleep",
        marker
            .to_str()
            .expect("temporary marker path should be UTF-8"),
        "60000",
    ]);
    process.cwd = directory.path().canonicalize().expect("canonical cwd");
    process.capability = CapabilityPolicy::local_owner(process.cwd.clone());
    process.policy.deadline = Some(Instant::now() + Duration::from_secs(1));
    process.policy.interrupt_grace = Duration::from_millis(50);
    process.policy.terminate_grace = Duration::from_millis(50);
    process.policy.reap_grace = Duration::from_millis(500);
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(process)
        .await
        .expect("partial-effect helper should start before its deadline");

    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        wait_for_terminal(&supervisor, &handle),
    )
    .await
    .expect("deadline cleanup must remain bounded");
    let ProcessOutcome::TimedOut { cleanup, .. } = outcome else {
        panic!("running deadline must produce TimedOut, got {outcome:?}");
    };
    assert!(cleanup.reaped);
    assert_eq!(
        fs::read(&marker).expect("the pre-timeout effect should remain observable"),
        b"partial effect before timeout\n"
    );
}

#[tokio::test]
#[cfg_attr(unix, serial_test::serial(unix_process_contract))]
async fn concurrent_starts_reserve_capacity_before_spawn_and_release_after_cleanup() {
    const MAX_SESSIONS: usize = 2;
    const ATTEMPTS: usize = 32;
    let directory = Arc::new(tempfile::tempdir().expect("temporary directory"));
    let supervisor = ProcessSupervisor::new(SupervisorConfig {
        max_sessions: MAX_SESSIONS,
        ..SupervisorConfig::default()
    });
    let barrier = Arc::new(tokio::sync::Barrier::new(ATTEMPTS + 1));
    let mut starts = tokio::task::JoinSet::new();
    for index in 0..ATTEMPTS {
        let directory = directory.clone();
        let supervisor = supervisor.clone();
        let barrier = barrier.clone();
        starts.spawn(async move {
            let marker = directory.path().join(format!("attempt-{index:02}.pid"));
            let mut process = helper_request(&[
                "write-pid-then-sleep",
                marker.to_str().expect("marker path should be UTF-8"),
                "60000",
            ]);
            process.cwd = directory.path().canonicalize().expect("canonical cwd");
            process.capability = CapabilityPolicy::local_owner(process.cwd.clone());
            process.policy.interrupt_grace = Duration::from_millis(10);
            process.policy.terminate_grace = Duration::from_millis(20);
            process.policy.reap_grace = Duration::from_millis(500);
            barrier.wait().await;
            (index, supervisor.start(process).await)
        });
    }
    barrier.wait().await;

    let mut handles = Vec::new();
    let mut capacity_rejections = 0;
    while let Some(joined) = starts.join_next().await {
        let (index, result) = joined.expect("start task should join");
        match result {
            Ok(handle) => handles.push((index, handle)),
            Err(error) if error.code() == "capacity_exhausted" => capacity_rejections += 1,
            Err(error) => panic!("unexpected concurrent start error: {error:?}"),
        }
    }
    assert_eq!(handles.len(), MAX_SESSIONS);
    assert_eq!(capacity_rejections, ATTEMPTS - MAX_SESSIONS);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count = fs::read_dir(directory.path())
                .expect("marker directory should be readable")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "pid"))
                .count();
            if count == MAX_SESSIONS {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("admitted helpers should publish readiness");
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("marker directory should be readable")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "pid"))
            .count(),
        MAX_SESSIONS,
        "capacity-rejected starts must not execute user code"
    );

    for (_, handle) in handles {
        let outcome = supervisor
            .cancel(&handle.owner, &handle.session_id)
            .await
            .expect("admitted helper cleanup should succeed");
        let ProcessOutcome::Cancelled { cleanup, .. } = outcome else {
            panic!("admitted helper should settle as Cancelled");
        };
        assert!(cleanup.reaped);
    }

    let reuse_marker = directory.path().join("reuse.pid");
    let mut reuse = helper_request(&[
        "write-pid-then-sleep",
        reuse_marker
            .to_str()
            .expect("reuse marker path should be UTF-8"),
        "60000",
    ]);
    reuse.cwd = directory.path().canonicalize().expect("canonical cwd");
    reuse.capability = CapabilityPolicy::local_owner(reuse.cwd.clone());
    reuse.policy.interrupt_grace = Duration::from_millis(10);
    reuse.policy.terminate_grace = Duration::from_millis(20);
    reuse.policy.reap_grace = Duration::from_millis(500);
    let handle = supervisor
        .start(reuse)
        .await
        .expect("cleanup should release one capacity slot");
    let outcome = supervisor
        .cancel(&handle.owner, &handle.session_id)
        .await
        .expect("reused capacity helper cleanup should succeed");
    assert!(matches!(
        outcome,
        ProcessOutcome::Cancelled {
            cleanup: nomi_process_runtime::CleanupReport { reaped: true, .. },
            ..
        }
    ));
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn public_supervisor_preserves_exit_codes_with_nofile_soft_limit_128() {
    let mut command = tokio::process::Command::new(low_fd_harness_binary());
    command.arg(helper_binary()).kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(8), command.output())
        .await
        .expect("low-FD harness must stay within its bounded runtime")
        .expect("low-FD harness process should launch");

    assert!(
        output.status.success(),
        "public supervisor failed under RLIMIT_NOFILE=128: status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn public_supervisor_closes_inherited_high_fd_sentinel() {
    let mut command = tokio::process::Command::new(fd_sentinel_harness_binary());
    command.arg(helper_binary()).kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(12), command.output())
        .await
        .expect("high-FD sentinel harness must stay within its bounded runtime")
        .expect("high-FD sentinel harness process should launch");

    assert!(
        output.status.success(),
        "public supervisor retained an inherited FD >=4097: status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn unix_pipe_round_trips_stdin_and_close_stdin_delivers_eof() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&["echo-stdin"]))
        .await
        .expect("Unix pipe helper should start");

    supervisor
        .write(&handle.owner, &handle.session_id, b"hello\0world\n")
        .await
        .expect("stdin write should succeed");
    supervisor
        .close_stdin(&handle.owner, &handle.session_id)
        .await
        .expect("closing stdin should succeed");

    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, .. } = outcome else {
        panic!("echo helper should produce Exited, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert_eq!(output.raw_bytes(), b"hello\0world\n");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[serial_test::serial(unix_process_contract)]
async fn macos_preserves_unicode_executable_path_argv_environment_and_cwd() {
    let directory = tempfile::tempdir().expect("temporary working directory");
    let cwd = directory.path().join("中文 workspace 'quoted'");
    fs::create_dir(&cwd).expect("complex working directory");
    let cwd = cwd.canonicalize().expect("canonical working directory");
    let executable = cwd.join("工具 helper 'quoted'");
    fs::copy(helper_binary(), &executable).expect("copy helper with executable permissions");
    let first = OsString::from("中文 spaced \\");
    let second = OsString::from(r#"quote " and literal $(exit 99)"#);
    let env_key = OsString::from("NOMIFUN_MACOS_ENV_CASE");
    let env_value = OsString::from("值 'quoted' $HOME");
    let mut process = request(
        executable,
        [
            OsString::from("print-args-env-cwd"),
            first.clone(),
            second.clone(),
            env_key.clone(),
            cwd.as_os_str().to_owned(),
        ],
    );
    process.cwd = cwd.clone();
    process.capability = CapabilityPolicy::local_owner(cwd.clone());
    process.env.insert(env_key, env_value.clone());

    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor.start(process).await.expect("complex macOS path should spawn");
    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, cleanup, .. } = outcome else {
        panic!("complex macOS path helper must exit, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert!(cleanup.reaped);
    let expected = [first, second, env_value, cwd.into_os_string()]
        .into_iter()
        .map(|field| {
            let field = field.to_string_lossy();
            format!("{}:{field}\n", field.len())
        })
        .collect::<String>();
    assert_eq!(output.text(), expected);
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[serial_test::serial(unix_process_contract)]
async fn macos_posix_shell_preserves_literal_environment_and_exit_status() {
    let mut process = request("unused", []);
    process.command = CommandSpec::Shell {
        shell: ShellKind::Posix,
        script: "printf '%s' \"$NOMIFUN_MACOS_SHELL_VALUE\"; exit 7".to_owned(),
    };
    let value = "中文 'quoted' $(exit 99) $HOME \\";
    process.env.insert("NOMIFUN_MACOS_SHELL_VALUE".into(), value.into());
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor.start(process).await.expect("POSIX shell should start");
    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, cleanup, .. } = outcome else {
        panic!("POSIX shell must exit, got {outcome:?}");
    };
    assert_eq!(code, Some(7));
    assert_eq!(output.text(), value);
    assert!(cleanup.reaped);
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[serial_test::serial(unix_process_contract)]
async fn macos_seatbelt_bare_program_keeps_requested_path_and_literal_spaces() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().expect("isolated executable fixture");
    let cwd = fixture.path().canonicalize().expect("canonical workspace");
    let blocked = cwd.join("blocked");
    let runnable = cwd.join("bin");
    fs::create_dir(&blocked).unwrap();
    fs::create_dir(&runnable).unwrap();
    let name = "literal helper with spaces";
    fs::copy(helper_binary(), blocked.join(name)).unwrap();
    fs::set_permissions(blocked.join(name), fs::Permissions::from_mode(0o644)).unwrap();
    fs::copy(helper_binary(), runnable.join(name)).unwrap();
    fs::copy(helper_binary(), cwd.join(name)).unwrap();
    // A non-executable earlier PATH entry must not shadow a later executable.
    // Relative entries resolve under the requested cwd, not the host checkout.
    for path in [std::env::join_paths([&blocked, &runnable]).unwrap(), OsString::from("blocked:bin"), OsString::new()] {
        let mut process = request(name, [OsString::from("exit"), OsString::from("7")]);
        process.cwd = cwd.clone();
        process.env.insert("PATH".into(), path);
        process.capability = CapabilityPolicy { cwd_roots: vec![cwd.clone()],
            sandbox: SandboxPolicy::MacSeatbelt { write_roots: vec![cwd.clone()] } };
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor.start(process).await.expect("literal spaced name must retain execvp semantics");
        let ProcessOutcome::Exited { code, cleanup, .. } = wait_for_terminal(&supervisor, &handle).await else {
            panic!("literal helper must exit with its actual status");
        };
        assert_eq!(code, Some(7));
        assert!(cleanup.reaped);
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[serial_test::serial(unix_process_contract)]
async fn macos_seatbelt_program_pipe_allows_only_declared_write_roots() {
    // Darwin's trusted temporary directories are intentionally writable in
    // the profile. Keep both fixtures beside the checkout so `outside` really
    // exercises the declared write-root boundary.
    let fixture_root = std::env::current_dir().expect("current directory");
    let workspace = tempfile::tempdir_in(&fixture_root).expect("workspace");
    let outside = tempfile::tempdir_in(&fixture_root).expect("outside");
    let workspace = workspace.path().canonicalize().expect("canonical workspace");
    let outside = outside.path().canonicalize().expect("canonical outside");
    let inside_marker = workspace.join("inside.marker");
    let outside_marker = outside.join("outside.marker");

    let mut allowed = request(
        helper_binary(),
        [
            OsString::from("write-file"),
            inside_marker.as_os_str().to_owned(),
        ],
    );
    allowed.cwd = workspace.clone();
    allowed.capability = CapabilityPolicy {
        cwd_roots: vec![workspace.clone()],
        sandbox: SandboxPolicy::MacSeatbelt {
            write_roots: vec![workspace.clone()],
        },
    };
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(allowed)
        .await
        .expect("in-root sandboxed program should start");
    let ProcessOutcome::Exited { code, output, .. } = wait_for_terminal(&supervisor, &handle).await else {
        panic!("in-root sandboxed program must exit");
    };
    assert_eq!(code, Some(0), "sandbox output: {}", output.text());
    assert!(inside_marker.exists());

    let mut denied = request(
        helper_binary(),
        [
            OsString::from("write-file"),
            outside_marker.as_os_str().to_owned(),
        ],
    );
    denied.cwd = workspace.clone();
    denied.capability = CapabilityPolicy {
        cwd_roots: vec![workspace.clone()],
        sandbox: SandboxPolicy::MacSeatbelt {
            write_roots: vec![workspace],
        },
    };
    let handle = supervisor
        .start(denied)
        .await
        .expect("Seatbelt denial is reported by the sandboxed program");
    let ProcessOutcome::Exited { code, .. } = wait_for_terminal(&supervisor, &handle).await else {
        panic!("denied sandboxed program must exit");
    };
    assert_ne!(code, Some(0));
    assert!(!outside_marker.exists());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[serial_test::serial(unix_process_contract)]
async fn macos_seatbelt_rejects_tmpdir_override_before_user_code_runs() {
    let workspace = tempfile::tempdir().expect("workspace");
    let workspace = workspace.path().canonicalize().expect("canonical workspace");
    let marker = workspace.join("must-not-run.marker");
    let mut process = request(
        helper_binary(),
        [
            OsString::from("write-file"),
            marker.as_os_str().to_owned(),
        ],
    );
    process.cwd = workspace.clone();
    process.env.insert(
        OsString::from("TMPDIR"),
        OsString::from("/tmp/untrusted-override"),
    );
    process.capability = CapabilityPolicy {
        cwd_roots: vec![workspace.clone()],
        sandbox: SandboxPolicy::MacSeatbelt {
            write_roots: vec![workspace],
        },
    };

    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let error = supervisor
        .start(process)
        .await
        .expect_err("untrusted TMPDIR override must fail before process");

    assert_eq!(error.code(), "invalid_command");
    assert!(!marker.exists());
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn invalid_executable_is_a_stable_spawn_failure_without_a_session() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let missing = Path::new("/definitely/not/a/nomifun-executable");

    let started = tokio::time::timeout(
        Duration::from_secs(6),
        supervisor.start(request(missing.as_os_str(), Vec::<OsString>::new())),
    )
    .await
    .expect("invalid executable spawn must finish within the shared setup deadline");
    let error = match started {
        Ok(_) => panic!("invalid executable must fail before a session is returned"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "spawn_failed");
    assert!(!matches!(error, ProcessError::Transport { .. }));
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn cancel_removes_the_leader_and_same_group_grandchild() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("grandchild.pid");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&[
            "spawn-grandchild",
            marker.to_str().expect("temporary path should be UTF-8"),
        ]))
        .await
        .expect("grandchild helper should start");
    let leader = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .expect("started leader should have status")
        .pid as libc::pid_t;
    let grandchild = wait_for_pid_marker(&marker).await as libc::pid_t;
    let mut cleanup = PidCleanup::new([leader, grandchild]);

    let outcome = tokio::time::timeout(
        Duration::from_secs(6),
        supervisor.cancel(&handle.owner, &handle.session_id),
    )
    .await
    .expect("group cancellation must stay within its frozen budget")
    .expect("group cancellation should resolve");

    let ProcessOutcome::Cancelled { cleanup: report, .. } = outcome else {
        panic!("group cancellation should be terminal Cancelled, got {outcome:?}");
    };
    assert!(report.interrupt_attempted);
    wait_for_processes_gone([leader, grandchild]).await;
    cleanup.disarm();
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn ignored_sigint_escalates_to_sigterm_and_removes_the_group() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("interrupt-ignoring-grandchild.pid");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&[
            "spawn-ignore-group",
            marker.to_str().expect("temporary path should be UTF-8"),
        ]))
        .await
        .expect("interrupt-ignoring group should start");
    let leader = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .expect("started leader should have status")
        .pid as libc::pid_t;
    let grandchild = wait_for_pid_marker(&marker).await as libc::pid_t;
    let mut cleanup = PidCleanup::new([leader, grandchild]);
    let cancellation_started = Instant::now();

    let outcome = tokio::time::timeout(
        Duration::from_secs(4),
        supervisor.cancel(&handle.owner, &handle.session_id),
    )
    .await
    .expect("SIGINT-to-SIGTERM escalation must stay bounded")
    .expect("group cancellation should resolve");
    let elapsed = cancellation_started.elapsed();

    let ProcessOutcome::Cancelled { cleanup: report, .. } = outcome else {
        panic!("escalated group cancellation should be Cancelled, got {outcome:?}");
    };
    assert!(report.interrupt_attempted);
    assert!(report.terminate_attempted);
    assert!(!report.force_kill_attempted);
    assert!(
        elapsed >= Duration::from_millis(900),
        "SIGTERM was sent before the one-second SIGINT grace: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3));
    wait_for_processes_gone([leader, grandchild]).await;
    cleanup.disarm();
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn leader_exit_does_not_publish_success_while_same_group_descendant_survives() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("leader-first-grandchild.pid");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&[
            "leader-first",
            marker.to_str().expect("temporary path should be UTF-8"),
        ]))
        .await
        .expect("leader-first helper should start");
    let leader = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .expect("started leader should have status")
        .pid as libc::pid_t;
    let grandchild = wait_for_pid_marker(&marker).await as libc::pid_t;
    let mut cleanup = PidCleanup::new([leader, grandchild]);

    let outcome = tokio::time::timeout(
        Duration::from_millis(250),
        wait_for_terminal(&supervisor, &handle),
    )
    .await
    .expect("leader-first cleanup should finish inside the quick-exit boundary");

    let ProcessOutcome::Exited { code, cleanup: report, .. } = outcome else {
        panic!("clean leader-first exit should remain Exited, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert!(report.reaped);
    wait_for_processes_gone([leader, grandchild]).await;
    cleanup.disarm();
}

#[tokio::test]
#[cfg(unix)]
#[serial_test::serial(unix_process_contract)]
async fn observable_setsid_escape_is_lost_instead_of_waiting_for_fake_pipe_eof() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("escaped-descendant.pid");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&[
            "setsid-escape",
            marker.to_str().expect("temporary path should be UTF-8"),
        ]))
        .await
        .expect("setsid escape helper should start");
    let escaped = wait_for_pid_marker(&marker).await as libc::pid_t;
    let mut cleanup = PidCleanup::new([escaped]);

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        supervisor.poll(
            &handle.owner,
            &handle.session_id,
            OutputCursor::START,
            Instant::now() + Duration::from_secs(30),
        ),
    )
    .await
    .expect("an inherited pipe held by an escaped descendant must not stall the waiter")
    .expect("poll should resolve the escaped session");

    let PollResult::Finished(ProcessOutcome::Lost { cleanup: report, .. }) = result else {
        panic!("detectable setsid escape must be Lost, got {result:?}");
    };
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("output reader timed out")),
        "Lost cleanup should identify the missing pipe EOF: {:?}",
        report.errors
    );
    cleanup.kill_all();
    wait_for_processes_gone([escaped]).await;
    cleanup.disarm();
}

#[cfg(unix)]
async fn wait_for_pid_marker(path: &Path) -> u32 {
    // A freshly linked macOS debug helper may pay one-time dyld and validation
    // cost before its user code runs. Poll immediately, but keep that loader
    // cost outside the process cleanup and wakeup SLAs asserted elsewhere.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse::<u32>()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("PID marker was not published: {}", path.display()))
}

#[cfg(unix)]
async fn wait_for_processes_gone(pids: impl IntoIterator<Item = libc::pid_t>) {
    let pids = pids.into_iter().collect::<Vec<_>>();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if pids.iter().all(|pid| !process_exists(*pid)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("processes still existed after cleanup: {pids:?}"));
}

#[cfg(unix)]
fn process_exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal zero probes liveness without delivering a signal.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
struct PidCleanup {
    pids: Vec<libc::pid_t>,
    armed: bool,
}

#[cfg(unix)]
impl PidCleanup {
    fn new(pids: impl IntoIterator<Item = libc::pid_t>) -> Self {
        Self {
            pids: pids.into_iter().collect(),
            armed: true,
        }
    }

    fn kill_all(&self) {
        for pid in &self.pids {
            // SAFETY: the guard stores only PIDs published by this test's helpers.
            let _ = unsafe { libc::kill(*pid, libc::SIGKILL) };
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for PidCleanup {
    fn drop(&mut self) {
        if self.armed {
            self.kill_all();
        }
    }
}

#[cfg(windows)]
#[tokio::test]
async fn natural_exit_returns_promptly() {
    for expected in [0, 7] {
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor
            .start(helper_request(&["exit", &expected.to_string()]))
            .await
            .expect("Windows pipe helper should start");

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_terminal(&supervisor, &handle),
        )
        .await
        .expect("quick natural exit must wake a far-yield poll within 2 seconds");
        let ProcessOutcome::Exited {
            code,
            signal,
            cleanup,
            ..
        } = outcome
        else {
            panic!("helper exit should produce Exited, got {outcome:?}");
        };
        assert_eq!(code, Some(expected));
        assert_eq!(signal, None);
        assert!(cleanup.reaped);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_pipe_round_trips_stdin_and_close_stdin_delivers_eof() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(helper_request(&["echo-stdin"]))
        .await
        .expect("Windows pipe helper should start");

    supervisor
        .write(&handle.owner, &handle.session_id, b"hello\0world\n")
        .await
        .expect("stdin write should succeed");
    supervisor
        .close_stdin(&handle.owner, &handle.session_id)
        .await
        .expect("closing stdin should succeed");

    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited {
        code,
        signal,
        output,
        cleanup,
    } = outcome
    else {
        panic!("echo helper should produce Exited, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
    assert_eq!(output.raw_bytes(), b"hello\0world\n");
    assert!(cleanup.reaped);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_invalid_executable_is_a_stable_spawn_failure_without_a_session() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let missing = directory.path().join("definitely-missing-nomifun-executable.exe");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());

    let started = tokio::time::timeout(
        Duration::from_secs(6),
        supervisor.start(request(missing.into_os_string(), Vec::<OsString>::new())),
    )
    .await
    .expect("invalid executable spawn must finish within the shared setup deadline");
    let error = match started {
        Ok(_) => panic!("invalid executable must fail before a session is returned"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "spawn_failed");
    assert!(!matches!(error, ProcessError::Transport { .. }));
}

#[cfg(windows)]
#[tokio::test]
async fn windows_cancel_reaps_the_leader_and_grandchild_within_five_seconds() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("grandchild.pid");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(request(
            helper_binary(),
            [
                OsString::from("spawn-grandchild"),
                marker.as_os_str().to_owned(),
            ],
        ))
        .await
        .expect("Windows grandchild helper should start");
    let leader_pid = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .expect("started leader should have status")
        .pid;
    let leader =
        ExactWindowsProcess::open(leader_pid).expect("leader exact process handle should open");
    let grandchild_pid = wait_for_windows_pid_marker(&marker).await;
    let grandchild = ExactWindowsProcess::open(grandchild_pid)
        .expect("grandchild exact process handle should open");

    let cancellation_started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        supervisor.cancel(&handle.owner, &handle.session_id),
    )
    .await
    .expect("Windows Job cancellation must finish within five seconds")
    .expect("Windows Job cancellation should resolve");
    let elapsed = cancellation_started.elapsed();

    let ProcessOutcome::Cancelled { cleanup, .. } = outcome else {
        panic!("Windows Job cancellation should be terminal Cancelled, got {outcome:?}");
    };
    assert!(cleanup.interrupt_attempted);
    assert!(cleanup.terminate_attempted || cleanup.force_kill_attempted);
    assert!(cleanup.reaped);
    assert!(
        elapsed < Duration::from_secs(5),
        "Windows cancellation exceeded its frozen budget: {elapsed:?}"
    );

    leader
        .wait_terminated(Duration::from_secs(2), "leader")
        .await;
    grandchild
        .wait_terminated(Duration::from_secs(2), "grandchild")
        .await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_leader_exit_waits_for_job_descendant_cleanup_before_success() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let marker = directory.path().join("leader-first-grandchild.pid");
    let exit_gate = directory.path().join("leader-exit.gate");
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(request(
            helper_binary(),
            [
                OsString::from("leader-first-gated"),
                marker.as_os_str().to_owned(),
                exit_gate.as_os_str().to_owned(),
            ],
        ))
        .await
        .expect("Windows leader-first helper should start");
    let grandchild_pid = wait_for_windows_pid_marker(&marker).await;
    let grandchild = ExactWindowsProcess::open(grandchild_pid)
        .expect("grandchild exact process handle should open");
    fs::write(&exit_gate, b"go").expect("leader exit gate should be published");

    let outcome = tokio::time::timeout(
        Duration::from_millis(250),
        wait_for_terminal(&supervisor, &handle),
    )
    .await
    .expect("leader-first Job cleanup should stay inside the quick-exit boundary");
    let ProcessOutcome::Exited {
        code,
        signal,
        cleanup,
        ..
    } = outcome
    else {
        panic!("leader-first helper should remain a truthful Exited outcome, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
    assert!(cleanup.reaped);
    grandchild
        .wait_terminated(Duration::from_secs(2), "leader-first grandchild")
        .await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_preserves_complex_unicode_argv_environment_and_cwd() {
    let directory = tempfile::tempdir().expect("temporary working directory should be created");
    let cwd = directory
        .path()
        .canonicalize()
        .expect("temporary working directory should canonicalize");
    let first = OsString::from("涓枃 spaced \\");
    let second = OsString::from(r#"quote " and trailing \\"#);
    let env_key = OsString::from("NOMIFUN_WINDOWS_ENV_CASE");
    let env_value = OsString::from("鍊?value");
    let mut process = request(
        helper_binary(),
        [
            OsString::from("print-args-env-cwd"),
            first.clone(),
            second.clone(),
            env_key.clone(),
            cwd.as_os_str().to_owned(),
        ],
    );
    process.cwd = cwd.clone();
    process.capability = CapabilityPolicy::local_owner(cwd.clone());
    process
        .env
        .insert(OsString::from("nomifun_windows_env_case"), env_value.clone());

    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor
        .start(process)
        .await
        .expect("complex Windows argv/env/cwd helper should start");
    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, .. } = outcome else {
        panic!("complex Windows argv/env/cwd helper should exit, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    let expected = [first, second, env_value, cwd.into_os_string()]
        .into_iter()
        .map(|field| {
            let field = field.to_string_lossy();
            format!("{}:{field}\n", field.len())
        })
        .collect::<String>();
    assert_eq!(output.text(), expected);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_powershell_preserves_final_native_and_pipeline_status() {
    for (script, expected) in [
        ("cmd /c exit 7", 7),
        ("cmd /c exit 7; Write-Output recovered", 0),
        ("Write-Output before; cmd /c exit 7", 7),
        ("Get-DefinitelyMissingNomifunCommand", 1),
        ("Write-Error bad -ErrorAction Continue", 1),
    ] {
        let mut process = request(helper_binary(), Vec::<OsString>::new());
        process.command = CommandSpec::Shell {
            shell: ShellKind::PowerShell,
            script: script.into(),
        };
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor
            .start(process)
            .await
            .unwrap_or_else(|error| panic!("PowerShell script failed to start: {script}: {error}"));
        let outcome = wait_for_terminal(&supervisor, &handle).await;
        let ProcessOutcome::Exited { code, .. } = outcome else {
            panic!("PowerShell script should exit: {script}: {outcome:?}");
        };
        assert_eq!(code, Some(expected), "PowerShell script: {script}");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_program_powershell_initializes_under_managed_owner() {
    let workspace = tempfile::tempdir().expect("isolated PowerShell workspace");
    fs::write(workspace.path().join("normal.txt"), b"fixture")
        .expect("create a bounded workspace entry");
    let powershell = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows")),
    )
    .join("System32")
    .join("WindowsPowerShell")
    .join("v1.0")
    .join("powershell.exe");
    let resolved = std::env::var_os("PATH").into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("powershell.exe"))
        .find(|candidate| candidate.is_file())
        .expect("PowerShell should be present on PATH");
    assert_eq!(fs::canonicalize(resolved).unwrap(), fs::canonicalize(&powershell).unwrap());
    for program in [OsString::from("powershell.exe"), powershell.into_os_string()] {
        let mut process = request(
            program,
            [
                "-NoProfile",
                "-Command",
                "Get-ChildItem -Force | Select-Object Mode, Name, Attributes, LinkType | Format-Table -AutoSize",
            ]
            .into_iter()
            .map(OsString::from),
        );
        process.cwd = workspace.path().to_path_buf();
        process.capability = CapabilityPolicy::local_owner(workspace.path().to_path_buf());
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor
            .start(process)
            .await
            .expect("managed PowerShell program should start");
        let outcome = wait_for_terminal(&supervisor, &handle).await;
        let ProcessOutcome::Exited { code, output, .. } = outcome else {
            panic!("managed PowerShell program should exit, got {outcome:?}");
        };
        assert_eq!(code, Some(0), "PowerShell startup/output: {}", output.text());
        assert!(output.text().contains("normal.txt"));
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_shell_reports_full_cwd_and_explicit_hidden_flags() {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let prefix = format!("{} 中文 ", "long-path-".repeat(14));
    let workspace = tempfile::Builder::new().prefix(&prefix).tempdir().unwrap();
    fs::write(workspace.path().join(".dot-note"), b"dot").unwrap();
    fs::OpenOptions::new().write(true).create_new(true).attributes(2)
        .open(workspace.path().join("hidden.txt")).unwrap();
    let scripts = [
        "(Get-Location).Path",
        "Get-ChildItem -LiteralPath . -Force | ForEach-Object { [pscustomobject]@{Name=$_.Name;Attributes=$_.Attributes.ToString();Hidden=[bool]($_.Attributes -band [IO.FileAttributes]::Hidden);System=[bool]($_.Attributes -band [IO.FileAttributes]::System);LinkType=$_.LinkType} } | ConvertTo-Json -Compress",
    ];
    for (index, script) in scripts.into_iter().enumerate() {
        let mut process = request(helper_binary(), Vec::<OsString>::new());
        process.command = CommandSpec::Shell { shell:ShellKind::PowerShell, script:script.into() };
        process.cwd = workspace.path().to_path_buf();
        process.capability = CapabilityPolicy::local_owner(workspace.path().to_path_buf());
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor.start(process).await.unwrap();
        let ProcessOutcome::Exited { code, output, .. } = wait_for_terminal(&supervisor, &handle).await else {
            panic!("observation command must exit");
        };
        assert_eq!(code, Some(0), "{}", output.text());
        if index == 0 {
            assert_eq!(output.text().trim(), workspace.path().to_str().unwrap());
        } else {
            let entries: serde_json::Value = serde_json::from_str(&output.text()).unwrap();
            for name in [".dot-note", "hidden.txt"] {
                let entry = entries.as_array().unwrap().iter().find(|entry| entry["Name"] == name).unwrap();
                let attributes = fs::metadata(workspace.path().join(name)).unwrap().file_attributes();
                assert_eq!(entry["Hidden"], attributes & 2 != 0);
            }
        }
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_utf8_file_read_and_literal_search_preserve_text_and_errors() {
    let workspace = tempfile::Builder::new().prefix("UTF8 资料 ").tempdir().unwrap();
    fs::create_dir(workspace.path().join("资料 空格")).unwrap();
    let file = workspace.path().join("资料 空格/样本.txt");
    let content = "alpha\r\nneedle-验收-42\r\n第三行\r\nomega\r\n";
    fs::write(&file,content.as_bytes()).unwrap();
    let scripts = [
        "$ErrorActionPreference='Stop'; [Console]::Write((Get-Content -LiteralPath '资料 空格/样本.txt' -Encoding UTF8 -Raw))",
        "$ErrorActionPreference='Stop'; Select-String -LiteralPath '资料 空格/样本.txt' -Pattern 'needle-验收-42' -SimpleMatch -Encoding UTF8 -ErrorAction Stop | ForEach-Object { [Console]::Write($_.Line) }",
        "$ErrorActionPreference='Stop'; Select-String -LiteralPath '资料 空格/样本.txt' -Pattern 'MISSING_NEEDLE' -SimpleMatch -Encoding UTF8 -ErrorAction Stop",
        "$ErrorActionPreference='Stop'; Select-String -LiteralPath '资料 空格/missing.txt' -Pattern 'MISSING_NEEDLE' -SimpleMatch -Encoding UTF8 -ErrorAction Stop",
    ];
    for (index,script) in scripts.into_iter().enumerate() {
        let mut process = request(helper_binary(),Vec::<OsString>::new());
        process.command=CommandSpec::Shell { shell:ShellKind::PowerShell,script:script.into() };
        process.cwd=workspace.path().to_path_buf();
        process.capability=CapabilityPolicy::local_owner(workspace.path().to_path_buf());
        let supervisor=ProcessSupervisor::new(SupervisorConfig::default());
        let handle=supervisor.start(process).await.unwrap();
        let ProcessOutcome::Exited { code,output,.. } = wait_for_terminal(&supervisor,&handle).await else {
            panic!("the scoped read/search must settle");
        };
        match index {
            0 => { assert_eq!(code,Some(0)); assert_eq!(output.text().as_bytes(),content.as_bytes()); }
            1 => { assert_eq!(code,Some(0)); assert_eq!(output.text(),"needle-验收-42"); }
            2 => { assert_eq!(code,Some(0)); assert!(output.text().is_empty()); }
            _ => { assert_ne!(code,Some(0),"a missing file is not a successful empty search"); assert!(!output.text().is_empty(),"the error must remain visible"); }
        }
        assert_eq!(fs::read(&file).unwrap(),content.as_bytes());
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_cmd_c_preserves_a_quoted_workspace_path() {
    let workspace = tempfile::Builder::new().prefix("命令 repo ").tempdir().unwrap();
    fs::create_dir(workspace.path().join("资料 空格")).unwrap();
    let content = b"CMD_QUOTED_PATH_185\r\n";
    let file = workspace.path().join("资料 空格/样本.txt");
    fs::write(&file, content).unwrap();
    let mut process = request("cmd.exe", ["/d", "/c", "type \"资料 空格\\样本.txt\""].map(OsString::from));
    process.cwd = workspace.path().to_path_buf();
    process.capability = CapabilityPolicy::local_owner(workspace.path().to_path_buf());
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor.start(process).await.expect("cmd starts");
    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, .. } = outcome else {
        panic!("cmd should exit: {outcome:?}");
    };
    assert_eq!(code, Some(0), "cmd output: {}", output.text());
    assert_eq!(output.text().as_bytes(), content);
    assert_eq!(fs::read(file).unwrap(), content);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_powershell_autoloads_hash_cmdlet_under_managed_owner() {
    let workspace = tempfile::Builder::new().prefix("任务 hash ").tempdir().unwrap();
    fs::write(workspace.path().join("normal.txt"), b"fixture").unwrap();
    let script = "$ErrorActionPreference = 'Stop'; (Get-FileHash -LiteralPath 'normal.txt' -Algorithm SHA256).Hash";
    let mut process = request("powershell.exe", ["-NoProfile", "-Command", script].map(OsString::from));
    process.cwd = workspace.path().to_path_buf();
    process.capability = CapabilityPolicy::local_owner(workspace.path().to_path_buf());
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = supervisor.start(process).await.expect("hash command starts");
    let outcome = wait_for_terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited { code, output, .. } = outcome else {
        panic!("hash command should exit: {outcome:?}");
    };
    assert_eq!(code, Some(0), "hash cmdlet output: {}", output.text());
    let hash = output.text().trim().to_owned();
    assert_eq!(hash.len(), 64, "SHA-256 must actually be returned: {hash}");
    assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[cfg(windows)]
async fn wait_for_windows_pid_marker(path: &Path) -> u32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse::<u32>()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("PID marker was not published: {}", path.display()))
}

#[cfg(windows)]
struct OwnedWindowsHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl OwnedWindowsHandle {
    fn new(
        handle: windows_sys::Win32::Foundation::HANDLE,
        operation: &'static str,
    ) -> io::Result<Self> {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;

        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::new(
                io::Error::last_os_error().kind(),
                format!("{operation}: {}", io::Error::last_os_error()),
            ))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for OwnedWindowsHandle {
    fn drop(&mut self) {
        // SAFETY: the wrapper owns one valid kernel handle and closes it exactly once.
        let _ = unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
struct ExactWindowsProcess {
    pid: u32,
    handle: OwnedWindowsHandle,
}

#[cfg(windows)]
impl ExactWindowsProcess {
    fn open(pid: u32) -> io::Result<Self> {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };

        // SAFETY: OpenProcess returns a new non-inheritable handle for the exact process object.
        let handle = OwnedWindowsHandle::new(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                    0,
                    pid,
                )
            },
            "OpenProcess",
        )?;
        Ok(Self { pid, handle })
    }

    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle.raw()
    }

    async fn wait_terminated(&self, timeout: Duration, label: &str) {
        use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::WaitForSingleObject;

        let deadline = Instant::now() + timeout;
        loop {
            // SAFETY: the exact process handle remains live while it is inspected.
            match unsafe { WaitForSingleObject(self.raw(), 0) } {
                WAIT_OBJECT_0 => return,
                WAIT_TIMEOUT if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                WAIT_TIMEOUT => panic!("{label} pid={} was still alive after {timeout:?}", self.pid),
                WAIT_FAILED => panic!(
                    "waiting for {label} pid={} failed: {}",
                    self.pid,
                    io::Error::last_os_error()
                ),
                result => panic!(
                    "waiting for {label} pid={} returned unexpected status {result:#x}",
                    self.pid
                ),
            }
        }
    }
}
