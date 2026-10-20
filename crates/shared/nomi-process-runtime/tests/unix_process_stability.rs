#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(target_os = "macos")]
use std::mem::{size_of, size_of_val};

use nomi_process_runtime::{
    CommandSpec, NormalizedProcessRequest, OutputCursor, PlatformLifecycleMetrics, PollResult,
    ProcessHandle, ProcessOutcome, ProcessOwner, ProcessState, ProcessSupervisor,
    ProcessTerminalWitness, SupervisorConfig, Transport, platform_lifecycle_metrics,
    probe_process_identity,
};

const SAMPLE: &[u8] = b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

fn request(transport: Transport, arguments: &[&str]) -> NormalizedProcessRequest {
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: env!("CARGO_BIN_EXE_process_test_helper").into(),
            args: arguments.iter().map(OsString::from).collect(),
        },
        cwd: std::env::current_dir().expect("fixture working directory should exist"),
        env: BTreeMap::new(),
        transport,
        policy: Default::default(),
    }
}

async fn terminal(supervisor: &ProcessSupervisor, handle: &ProcessHandle) -> ProcessOutcome {
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
    .expect("original native owner should settle within the observation window")
    .expect("original native outcome should be readable");
    match result {
        PollResult::Finished(outcome) => outcome,
        PollResult::Running { .. } => panic!("short fixture remained active"),
    }
}

async fn short_process(
    supervisor: Arc<ProcessSupervisor>,
    transport: Transport,
) -> ProcessTerminalWitness {
    let handle = supervisor
        .start(request(transport, &["flood", "32"]))
        .await
        .expect("short fixture should start");
    let witness = handle.terminal_witness();
    let outcome = terminal(&supervisor, &handle).await;
    let ProcessOutcome::Exited {
        code,
        signal,
        output,
        cleanup,
    } = outcome
    else {
        panic!("short fixture must exit naturally with its exact owner: {outcome:?}");
    };
    assert_eq!(code, Some(0));
    assert_eq!(signal, None);
    assert!(
        cleanup.reaped,
        "short fixture requires original native reap proof"
    );
    assert!(
        cleanup.errors.is_empty(),
        "native cleanup errors: {:?}",
        cleanup.errors
    );
    assert_eq!(output.raw_bytes(), SAMPLE);
    assert_eq!(output.dropped_bytes, 0);
    assert!(
        supervisor
            .release_terminal(&handle.owner, &handle.session_id)
            .unwrap(),
        "only the exact reaped terminal owner should leave the registry"
    );
    assert_eq!(
        witness
            .frozen_output()
            .expect("history witness should remain readable")
            .snapshot_from(OutputCursor::START)
            .raw_bytes(),
        SAMPLE
    );
    witness
}

async fn short_round(supervisor: &Arc<ProcessSupervisor>) -> Vec<ProcessTerminalWitness> {
    let mut work = tokio::task::JoinSet::new();
    for _ in 0..2 {
        for transport in [Transport::Pipe, Transport::Pty { cols: 80, rows: 24 }] {
            work.spawn(short_process(Arc::clone(supervisor), transport));
        }
    }
    let mut history = Vec::new();
    while let Some(result) = work.join_next().await {
        history.push(result.expect("short native fixture must not panic"));
    }
    history
}

#[derive(Clone, Debug)]
struct Resources {
    fds: usize,
    threads: i32,
    rss: u64,
    cpu_ns: u64,
    children: BTreeSet<libc::pid_t>,
    lifecycle: PlatformLifecycleMetrics,
}

#[cfg(target_os = "macos")]
fn resources() -> Resources {
    let pid = std::process::id() as libc::pid_t;
    let mut descriptors: Vec<libc::proc_fdinfo> = (0..256)
        .map(|_| libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        })
        .collect();
    // This only observes descriptors of the current test process. A full buffer
    // is rejected instead of treating a truncated list as a stable baseline.
    let descriptor_bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            descriptors.as_mut_ptr().cast(),
            (descriptors.len() * size_of::<libc::proc_fdinfo>()) as i32,
        )
    };
    assert!(
        descriptor_bytes > 0,
        "native FD inventory failed: {}",
        std::io::Error::last_os_error()
    );
    assert!(
        (descriptor_bytes as usize) < descriptors.len() * size_of::<libc::proc_fdinfo>(),
        "FD inventory was truncated"
    );
    assert_eq!(
        descriptor_bytes as usize % size_of::<libc::proc_fdinfo>(),
        0
    );
    let mut task: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let task_bytes = size_of::<libc::proc_taskinfo>() as i32;
    assert_eq!(
        unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                (&raw mut task).cast(),
                task_bytes,
            )
        },
        task_bytes,
        "native task inventory failed"
    );
    // getrusage exposes timeval CPU units, unlike proc_taskinfo's Mach ticks.
    // Keep this diagnostic in ns without adding a Mach ABI dependency.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) }, 0);
    let cpu_ns = [usage.ru_utime, usage.ru_stime].into_iter().map(|time| {
        u64::try_from(time.tv_sec).expect("CPU seconds must be nonnegative") * 1_000_000_000
            + u64::try_from(time.tv_usec).expect("CPU microseconds must be nonnegative") * 1_000
    }).sum();
    let mut children = [0 as libc::pid_t; 128];
    let child_count = unsafe {
        libc::proc_listchildpids(
            pid,
            children.as_mut_ptr().cast(),
            size_of_val(&children) as i32,
        )
    };
    assert!(child_count >= 0, "native direct-child inventory failed");
    assert!(
        (child_count as usize) < children.len(),
        "direct-child inventory was truncated"
    );
    Resources {
        fds: descriptor_bytes as usize / size_of::<libc::proc_fdinfo>(),
        threads: task.pti_threadnum,
        rss: task.pti_resident_size,
        cpu_ns,
        lifecycle: platform_lifecycle_metrics(),
        children: children[..child_count as usize]
            .iter()
            .copied()
            .filter(|pid| *pid > 1)
            .collect(),
    }
}

#[cfg(target_os = "linux")]
fn resources() -> Resources {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) }, 0);
    let cpu_ns = [usage.ru_utime, usage.ru_stime].into_iter().map(|time| {
        u64::try_from(time.tv_sec).expect("CPU seconds must be nonnegative") * 1_000_000_000
            + u64::try_from(time.tv_usec).expect("CPU microseconds must be nonnegative") * 1_000
    }).sum();
    let fds = std::fs::read_dir("/proc/self/fd")
        .expect("native FD inventory failed").count();
    let tasks: Vec<std::path::PathBuf> = std::fs::read_dir("/proc/self/task")
        .expect("native task inventory failed")
        .map(|entry| entry.expect("native task entry readable").path())
        .collect();
    let status = std::fs::read_to_string("/proc/self/status")
        .expect("native status inventory failed");
    let rss = status.lines().find_map(|line| line.strip_prefix("VmRSS:")
        .and_then(|value| value.trim().strip_suffix(" kB"))
        .and_then(|value| value.parse::<u64>().ok())
        .map(|kib| kib * 1024))
        .expect("native RSS inventory missing");
    let mut children = BTreeSet::new();
    for task in &tasks {
        match std::fs::read_to_string(task.join("children")) {
            Ok(list) => {
                for token in list.split_whitespace() {
                    let child: libc::pid_t = token.parse().expect("native child inventory is numeric");
                    if child > 1 { children.insert(child); }
                }
            }
            Err(error) => {
                assert!(!task.exists() && error.kind() == std::io::ErrorKind::NotFound,
                    "native child inventory failed for a still-present task: {error}");
            }
        }
    }
    Resources {
        fds,
        threads: i32::try_from(tasks.len()).expect("thread count fits"),
        rss,
        cpu_ns,
        lifecycle: platform_lifecycle_metrics(),
        children,
    }
}

async fn settled_initial_resources() -> Resources {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed = resources();
        if observed.children.is_empty()
            && observed.lifecycle.admitted == 0
            && observed.lifecycle.retained_jobs == 0
            && observed.lifecycle.quarantined == 0
        {
            return observed;
        }
        assert!(
            Instant::now() < deadline,
            "warmup native owners did not retire: {observed:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn settled_resources(baseline: &Resources) -> Resources {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed = resources();
        if observed.fds <= baseline.fds + 2
            && observed.children == baseline.children
            && observed.lifecycle.admitted == baseline.lifecycle.admitted
            && observed.lifecycle.retained_jobs == baseline.lifecycle.retained_jobs
            && observed.lifecycle.quarantined == 0
        {
            return observed;
        }
        assert!(
            Instant::now() < deadline,
            "reaped pipe/PTY/watchdog resources did not settle: baseline={baseline:?}, actual={observed:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn repeated_pipe_and_pty_retirement_preserves_the_original_long_process_and_stable_native_resources()
 {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    drop(short_round(&supervisor).await);
    let before_long = settled_initial_resources().await;
    let long = supervisor
        .start(request(Transport::Pipe, &["echo-stdin"]))
        .await
        .expect("legitimate long-lived sibling should start once");
    let long_identity = probe_process_identity(long.pid)
        .unwrap()
        .expect("original live generation should be observable");
    supervisor
        .write(&long.owner, &long.session_id, b"long-live\n")
        .await
        .unwrap();
    let baseline = resources();
    let mut retained_history = Vec::new();
    eprintln!(
        "{} resource baseline: before_long={before_long:?}, with_long={baseline:?}, long_identity={long_identity:?}",
        std::env::consts::OS
    );
    for round in 1..=3 {
        retained_history.extend(short_round(&supervisor).await);
        let status = supervisor
            .status(&long.owner, &long.session_id)
            .await
            .unwrap();
        assert_eq!(status.pid, long.pid);
        assert_eq!(status.started_at, long.started_at);
        assert_eq!(status.state, ProcessState::Running);
        assert_eq!(
            probe_process_identity(long.pid).unwrap().as_ref(),
            Some(&long_identity),
            "short-task retirement must preserve the original long process generation"
        );
        let snapshot = settled_resources(&baseline).await;
        assert!(
            snapshot.threads <= baseline.threads + 2,
            "retired native workers accumulated: baseline={baseline:?}, actual={snapshot:?}"
        );
        eprintln!(
            "{} resource round={round}, short_reaped={}, retained_history={}, resources={snapshot:?}, rss_delta={}, cpu_delta_ns={}",
            std::env::consts::OS,
            round * 4,
            retained_history.len(),
            snapshot.rss.saturating_sub(baseline.rss),
            snapshot.cpu_ns.saturating_sub(baseline.cpu_ns)
        );
    }
    assert_eq!(retained_history.len(), 12);
    for witness in &retained_history {
        let ProcessOutcome::Exited {
            cleanup, output, ..
        } = witness.terminal(OutputCursor::START).unwrap()
        else {
            panic!("retained history must not recreate native execution");
        };
        assert!(cleanup.reaped);
        assert_eq!(output.raw_bytes(), SAMPLE);
    }
    supervisor
        .close_stdin(&long.owner, &long.session_id)
        .await
        .unwrap();
    let ProcessOutcome::Exited {
        code,
        output,
        cleanup,
        ..
    } = terminal(&supervisor, &long).await
    else {
        panic!("original long sibling should end naturally only after its own stdin EOF");
    };
    assert_eq!(code, Some(0));
    assert!(cleanup.reaped);
    assert_eq!(output.raw_bytes(), b"long-live\n");
    assert!(
        supervisor
            .release_terminal(&long.owner, &long.session_id)
            .unwrap()
    );
    let report = supervisor.shutdown().await;
    assert!(report.is_exact());
    assert!(
        report.sessions.is_empty(),
        "all original processes ended naturally before shutdown"
    );
    assert!(report.startups.is_empty());
    drop(supervisor);
    let after = settled_resources(&before_long).await;
    assert!(after.threads <= before_long.threads + 2);
    eprintln!(
        "{} resource final: resources={after:?}, original_long_reaped=true, retained_history={}",
        std::env::consts::OS,
        retained_history.len()
    );
}
