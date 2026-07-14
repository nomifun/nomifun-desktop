#[cfg(unix)]
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    io,
    process,
    time::{Duration, Instant},
};

#[cfg(unix)]
use nomi_process_runtime::{
    CapabilityPolicy, CommandSpec, ProcessOutcome, ProcessOwner, ProcessPolicy,
    NormalizedProcessRequest, OutputCursor, PollResult, ProcessSupervisor, SupervisorConfig,
    Transport,
};

#[cfg(unix)]
const LOW_FD_LIMIT: libc::rlim_t = 128;

#[cfg(unix)]
fn main() {
    if let Err(error) = run() {
        eprintln!("low-FD supervisor harness failed: {error}");
        process::exit(2);
    }
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn run() -> Result<(), String> {
    let helper = env::args_os()
        .nth(1)
        .ok_or_else(|| "expected the process_test_helper path".to_owned())?;
    lower_nofile_soft_limit()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("create Tokio runtime after lowering RLIMIT_NOFILE: {error}"))?;
    runtime.block_on(run_exit_contracts(helper))
}

#[cfg(unix)]
fn lower_nofile_soft_limit() -> Result<(), String> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one initialized rlimit to the supplied pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == -1 {
        return Err(format!("getrlimit(RLIMIT_NOFILE): {}", io::Error::last_os_error()));
    }
    if limit.rlim_max < LOW_FD_LIMIT {
        return Err(format!(
            "hard RLIMIT_NOFILE {} is below required test limit {LOW_FD_LIMIT}",
            limit.rlim_max
        ));
    }
    limit.rlim_cur = LOW_FD_LIMIT;
    // SAFETY: the new soft limit does not exceed the unchanged hard limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } == -1 {
        return Err(format!("setrlimit(RLIMIT_NOFILE=128): {}", io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
async fn run_exit_contracts(helper: OsString) -> Result<(), String> {
    for expected in [0, 7] {
        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let handle = supervisor
            .start(helper_request(helper.clone(), expected))
            .await
            .map_err(|error| format!("start helper exit {expected}: {error}"))?;
        let polled = tokio::time::timeout(
            Duration::from_secs(3),
            supervisor.poll(
                &handle.owner,
                &handle.session_id,
                OutputCursor::START,
                Instant::now() + Duration::from_secs(30),
            ),
        )
        .await
        .map_err(|_| format!("poll helper exit {expected} exceeded 3 seconds"))?
        .map_err(|error| format!("poll helper exit {expected}: {error}"))?;

        let PollResult::Finished(ProcessOutcome::Exited {
            code,
            signal,
            cleanup,
            ..
        }) = polled
        else {
            return Err(format!(
                "helper exit {expected} did not produce an exact Exited terminal state: {polled:?}"
            ));
        };
        if code != Some(expected) || signal.is_some() || !cleanup.reaped {
            return Err(format!(
                "helper exit {expected} terminal mismatch: code={code:?}, signal={signal:?}, cleanup={cleanup:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn helper_request(helper: OsString, expected: i32) -> NormalizedProcessRequest {
    let cwd = env::current_dir().expect("low-FD harness current directory should exist");
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: helper,
            args: vec![OsString::from("exit"), OsString::from(expected.to_string())],
        },
        cwd: cwd.clone(),
        env: BTreeMap::new(),
        transport: Transport::Pipe,
        policy: ProcessPolicy::default(),
        capability: CapabilityPolicy::local_owner(cwd),
    }
}
