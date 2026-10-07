#![cfg(any(target_os = "linux", windows))]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    sync::Arc,
    time::{Duration, Instant},
};

use nomi_process_runtime::{
    CapabilityPolicy, CommandSpec, NormalizedProcessRequest, OutputCursor,
    PollResult, ProcessHandle, ProcessOutcome, ProcessOwner, ProcessPolicy,
    ProcessSupervisor, SupervisorConfig, Transport,
};

const PROCESS_COUNT: usize = 1_000;
const EXPECTED_OUTPUT: &[u8] = b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

fn helper_binary() -> &'static str {
    env!("CARGO_BIN_EXE_process_test_helper")
}

fn helper_request() -> NormalizedProcessRequest {
    let cwd = std::env::current_dir().expect("current directory should exist");
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: helper_binary().into(),
            args: [OsString::from("flood"), OsString::from("32")].into(),
        },
        cwd: cwd.clone(),
        env: BTreeMap::new(),
        transport: Transport::Pipe,
        policy: ProcessPolicy::default(),
        capability: CapabilityPolicy::local_owner(cwd),
    }
}

async fn run_one(supervisor: &Arc<ProcessSupervisor>) {
    let handle = supervisor
        .start(helper_request())
        .await
        .expect("short helper should start");
    let outcome = wait_for_terminal(supervisor, &handle).await;
    let ProcessOutcome::Exited {
        code,
        signal,
        output,
        cleanup,
        ..
    } = outcome
    else {
        panic!("short helper should exit normally, got {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
    assert!(cleanup.reaped, "short helper must be exactly reaped");
    assert_eq!(output.raw_bytes(), EXPECTED_OUTPUT);
    assert_eq!(output.dropped_bytes, 0);
}

async fn wait_for_terminal(
    supervisor: &ProcessSupervisor,
    handle: &ProcessHandle,
) -> ProcessOutcome {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        supervisor.poll(
            &handle.owner,
            &handle.session_id,
            OutputCursor::START,
            Instant::now() + Duration::from_secs(5),
        ),
    )
    .await
    .expect("short helper poll must stay bounded")
    .expect("short helper poll should succeed");
    match result {
        PollResult::Finished(outcome) => outcome,
        PollResult::Running { .. } => panic!("short helper remained active after bounded poll"),
    }
}

async fn warm_runtime() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig {
        max_sessions: 2,
        ..SupervisorConfig::default()
    });
    run_one(&supervisor).await;
    let report = supervisor.shutdown().await;
    assert!(report.sessions.is_empty());
    drop(supervisor);
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[derive(Clone, Copy, Debug)]
struct ResourceSnapshot {
    handles_or_fds: usize,
    threads: usize,
}

#[tokio::test]
#[ignore = "manual 1,000-process reliability soak"]
async fn one_thousand_short_processes_preserve_output_and_release_resources() {
    warm_runtime().await;
    let before = resource_snapshot();
    let supervisor = ProcessSupervisor::new(SupervisorConfig {
        max_sessions: 8,
        ..SupervisorConfig::default()
    });

    for completed in 1..=PROCESS_COUNT {
        run_one(&supervisor).await;
        if completed % 100 == 0 {
            eprintln!("completed {completed}/{PROCESS_COUNT} short processes");
        }
    }

    let report = supervisor.shutdown().await;
    assert!(
        report.sessions.is_empty(),
        "naturally completed helpers must not become shutdown cancellations: {:?}",
        report.sessions
    );
    drop(supervisor);
    tokio::time::sleep(Duration::from_millis(250)).await;
    let after = resource_snapshot();
    eprintln!("resource snapshots: before={before:?}, after={after:?}");

    assert!(
        after.handles_or_fds <= before.handles_or_fds.saturating_add(4),
        "host handle/fd count grew beyond tolerance: before={before:?}, after={after:?}"
    );
    assert!(
        after.threads <= before.threads.saturating_add(2),
        "host thread count grew beyond tolerance: before={before:?}, after={after:?}"
    );
}

#[cfg(target_os = "linux")]
fn resource_snapshot() -> ResourceSnapshot {
    ResourceSnapshot {
        handles_or_fds: std::fs::read_dir("/proc/self/fd")
            .expect("process fd directory should be readable")
            .count(),
        threads: std::fs::read_dir("/proc/self/task")
            .expect("process task directory should be readable")
            .count(),
    }
}

#[cfg(windows)]
fn resource_snapshot() -> ResourceSnapshot {
    use std::{io, mem::size_of};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32,
                Thread32First, Thread32Next,
            },
            Threading::{
                GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount,
            },
        },
    };

    let process_id = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    assert_ne!(
        snapshot,
        INVALID_HANDLE_VALUE,
        "thread snapshot failed: {}",
        io::Error::last_os_error()
    );
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut threads = 0_usize;
    assert_ne!(
        unsafe { Thread32First(snapshot, &mut entry) },
        0,
        "thread enumeration failed: {}",
        io::Error::last_os_error()
    );
    loop {
        if entry.th32OwnerProcessID == process_id {
            threads += 1;
        }
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            break;
        }
    }
    assert_ne!(unsafe { CloseHandle(snapshot) }, 0);

    let mut handles = 0_u32;
    assert_ne!(
        unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut handles) },
        0,
        "process handle count failed: {}",
        io::Error::last_os_error()
    );
    ResourceSnapshot {
        handles_or_fds: handles as usize,
        threads,
    }
}
