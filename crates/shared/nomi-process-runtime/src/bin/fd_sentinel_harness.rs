#[cfg(unix)]
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    process,
    time::{Duration, Instant},
};

#[cfg(unix)]
use nomi_process_runtime::{
    CapabilityPolicy, CommandSpec, ProcessOutcome, ProcessOwner, ProcessPolicy,
    NormalizedProcessRequest, ProcessState, ProcessSupervisor, SupervisorConfig, Transport,
};

#[cfg(unix)]
const REQUIRED_NOFILE_SOFT: libc::rlim_t = 8_192;
#[cfg(unix)]
const SENTINEL_MIN_FD: RawFd = 4_097;
#[cfg(unix)]
const SENTINEL_EOF_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(unix)]
fn main() {
    if let Err(error) = run() {
        eprintln!("high-FD sentinel harness failed: {error}");
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
    ensure_nofile_soft_limit()?;
    let (sentinel_reader, sentinel_writer) = high_fd_sentinel()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("create Tokio runtime after high-FD sentinel: {error}"))?;
    runtime.block_on(run_sentinel_contract(
        helper,
        sentinel_reader,
        sentinel_writer,
    ))
}

#[cfg(unix)]
fn ensure_nofile_soft_limit() -> Result<(), String> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one initialized rlimit to the supplied pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == -1 {
        return Err(format!(
            "getrlimit(RLIMIT_NOFILE): {}",
            io::Error::last_os_error()
        ));
    }
    if limit.rlim_max < REQUIRED_NOFILE_SOFT {
        return Err(format!(
            "hard RLIMIT_NOFILE {} is below required test limit {REQUIRED_NOFILE_SOFT}",
            limit.rlim_max
        ));
    }
    if limit.rlim_cur >= REQUIRED_NOFILE_SOFT {
        return Ok(());
    }
    limit.rlim_cur = REQUIRED_NOFILE_SOFT;
    // SAFETY: the requested soft limit does not exceed the unchanged hard limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } == -1 {
        return Err(format!(
            "setrlimit(RLIMIT_NOFILE={REQUIRED_NOFILE_SOFT}): {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn high_fd_sentinel() -> Result<(OwnedFd, OwnedFd), String> {
    let mut descriptors = [-1; 2];
    create_cloexec_pipe(&mut descriptors)?;
    // SAFETY: create_cloexec_pipe initialized both owned descriptors on success.
    let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    // SAFETY: create_cloexec_pipe initialized both owned descriptors on success.
    let original_writer = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };

    let high_writer = loop {
        // SAFETY: original_writer remains open and SENTINEL_MIN_FD is a valid lower bound.
        let descriptor = unsafe {
            libc::fcntl(
                original_writer.as_raw_fd(),
                libc::F_DUPFD_CLOEXEC,
                SENTINEL_MIN_FD,
            )
        };
        if descriptor >= SENTINEL_MIN_FD {
            // SAFETY: a successful F_DUPFD_CLOEXEC returns a newly owned descriptor.
            break unsafe { OwnedFd::from_raw_fd(descriptor) };
        }
        if descriptor >= 0 {
            // SAFETY: fcntl returned an unexpected but newly owned descriptor.
            unsafe { libc::close(descriptor) };
            return Err(format!(
                "F_DUPFD_CLOEXEC returned descriptor {descriptor} below {SENTINEL_MIN_FD}"
            ));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(format!(
                "F_DUPFD_CLOEXEC(min={SENTINEL_MIN_FD}): {error}"
            ));
        }
    };
    drop(original_writer);
    Ok((reader, high_writer))
}

#[cfg(target_os = "linux")]
fn create_cloexec_pipe(descriptors: &mut [RawFd; 2]) -> Result<(), String> {
    // SAFETY: pipe2 initializes both array entries on success.
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "pipe2(O_CLOEXEC): {}",
            io::Error::last_os_error()
        ))
    }
}

#[cfg(target_os = "macos")]
fn create_cloexec_pipe(descriptors: &mut [RawFd; 2]) -> Result<(), String> {
    // Darwin's public libc surface has no pipe2 declaration. Apply CLOEXEC to
    // both fresh descriptors before any runtime threads exist.
    // SAFETY: pipe initializes both array entries on success.
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } == -1 {
        return Err(format!("pipe: {}", io::Error::last_os_error()));
    }
    for descriptor in *descriptors {
        // SAFETY: descriptor is one of the two fresh pipe endpoints.
        if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            let error = io::Error::last_os_error();
            // SAFETY: both descriptors are owned by this function until success.
            unsafe {
                libc::close(descriptors[0]);
                libc::close(descriptors[1]);
            }
            descriptors.fill(-1);
            return Err(format!("fcntl(FD_CLOEXEC): {error}"));
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn run_sentinel_contract(
    helper: OsString,
    sentinel_reader: OwnedFd,
    sentinel_writer: OwnedFd,
) -> Result<(), String> {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let handle = tokio::time::timeout(
        Duration::from_secs(5),
        supervisor.start(helper_request(helper)),
    )
    .await
    .map_err(|_| "start long-running helper exceeded 5 seconds".to_owned())?
    .map_err(|error| format!("start long-running helper: {error}"))?;

    let before_eof = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .map_err(|error| format!("status before sentinel EOF: {error}"))?;
    if before_eof.state != ProcessState::Running {
        return Err(format!(
            "leader was not running before sentinel EOF: {before_eof:?}"
        ));
    }

    drop(sentinel_writer);
    let eof_result = wait_for_pipe_eof(sentinel_reader.as_raw_fd());
    let running_result = supervisor
        .status(&handle.owner, &handle.session_id)
        .await
        .map_err(|error| format!("status after sentinel EOF: {error}"))
        .and_then(|snapshot| {
            if snapshot.state == ProcessState::Running {
                Ok(())
            } else {
                Err(format!(
                    "leader stopped before sentinel EOF was verified: {snapshot:?}"
                ))
            }
        });

    let cancellation = tokio::time::timeout(
        Duration::from_secs(6),
        supervisor.cancel(&handle.owner, &handle.session_id),
    )
    .await
    .map_err(|_| "cancel long-running helper exceeded 6 seconds".to_owned())?
    .map_err(|error| format!("cancel long-running helper: {error}"));

    eof_result?;
    running_result?;
    let outcome = cancellation?;
    let ProcessOutcome::Cancelled { cleanup, .. } = outcome else {
        return Err(format!(
            "long-running helper did not reach Cancelled: {outcome:?}"
        ));
    };
    if !cleanup.reaped {
        return Err(format!(
            "cancelled helper was not terminally reaped: {cleanup:?}"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn wait_for_pipe_eof(reader: RawFd) -> Result<(), String> {
    let started = Instant::now();
    let deadline = started + SENTINEL_EOF_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "high-FD sentinel did not reach EOF within {SENTINEL_EOF_TIMEOUT:?}"
            ));
        }
        let timeout_ms = remaining.as_millis().clamp(1, libc::c_int::MAX as u128) as libc::c_int;
        let mut descriptor = libc::pollfd {
            fd: reader,
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        // SAFETY: descriptor points to one initialized pollfd for the call duration.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result > 0 {
            break;
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(format!("poll high-FD sentinel: {error}"));
        }
    }

    let mut byte = [0_u8];
    // SAFETY: reader remains open and byte provides one writable byte.
    let count = unsafe { libc::read(reader, byte.as_mut_ptr().cast(), byte.len()) };
    match count {
        0 => Ok(()),
        1 => Err("high-FD sentinel unexpectedly carried data before EOF".to_owned()),
        _ => Err(format!(
            "read high-FD sentinel after readiness: {}",
            io::Error::last_os_error()
        )),
    }
}

#[cfg(unix)]
fn helper_request(helper: OsString) -> NormalizedProcessRequest {
    let cwd = env::current_dir().expect("high-FD sentinel harness current directory should exist");
    NormalizedProcessRequest {
        owner: ProcessOwner::new(uuid::Uuid::now_v7(), uuid::Uuid::now_v7()),
        command: CommandSpec::Program {
            program: helper,
            args: vec![OsString::from("sleep"), OsString::from("60000")],
        },
        cwd: cwd.clone(),
        env: BTreeMap::new(),
        transport: Transport::Pipe,
        policy: ProcessPolicy::default(),
        capability: CapabilityPolicy::local_owner(cwd),
    }
}
