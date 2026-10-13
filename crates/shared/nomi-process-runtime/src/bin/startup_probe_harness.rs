#[cfg(unix)]
use std::{io, path::PathBuf, process::Stdio, time::{Duration, Instant}};

#[cfg(unix)]
use nomi_process_runtime::{ChildProcessBuilder, capture_child_identity,
    platform_lifecycle_metrics, probe_process_identity, run_isolated_startup_probe,
    same_recorded_process};

#[cfg(unix)]
fn main() {
    if let Err(error) = run() {
        eprintln!("startup probe harness failed: {error}");
        std::process::exit(2);
    }
}

#[cfg(not(unix))]
fn main() {}

#[cfg(unix)]
fn run() -> io::Result<()> {
    let marker = std::env::args_os().nth(1).map(PathBuf::from)
        .ok_or_else(|| io::Error::other("expected a private child identity marker"))?;
    assert_eq!(platform_lifecycle_metrics().poller_threads, 0);
    // This is the standalone harness main thread. No worker/runtime has been
    // created in this host, and none of the callbacks initialize GUI APIs.
    let output = unsafe {
        run_isolated_startup_probe(Duration::from_secs(3), 256 * 1024, || {
            let runtime = runtime()?;
            runtime.block_on(async {
                let mut builder = ChildProcessBuilder::new("/bin/sh");
                builder.args(["-c", "printf '%s' 'managed-startup-result'"]);
                let output = builder.output().await?;
                if !output.status.success() { return Err(io::Error::other("probe command failed")); }
                Ok(output.stdout)
            })
        })
    }?;
    assert_eq!(output, b"managed-startup-result");
    assert_eq!(platform_lifecycle_metrics().poller_threads, 0);
    let large = unsafe {
        run_isolated_startup_probe(Duration::from_secs(3), 256 * 1024,
            || Ok(vec![b'x'; 128 * 1024]))
    }?;
    assert_eq!(large.len(), 128 * 1024, "parent must drain before waiting");
    assert!(unsafe {
        run_isolated_startup_probe(Duration::from_secs(1), 32,
            || Ok(vec![b'x'; 33]))
    }.is_err(), "oversized discovery must be rejected");
    assert_eq!(unsafe {
        run_isolated_startup_probe(Duration::from_secs(1), 32, || {
            // Exit success without completing the callback must not be
            // confused with an admitted empty result.
            libc::_exit(0)
        })
    }.unwrap_err().kind(), io::ErrorKind::InvalidData);

    let start = Instant::now();
    let result = unsafe {
        run_isolated_startup_probe(Duration::from_millis(400), 1024, || {
            let runtime = runtime()?;
            let mut builder = ChildProcessBuilder::new("/bin/sleep");
            builder.arg("60").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let mut process = {
                let _entered = runtime.enter();
                builder.spawn_managed()?
            };
            let identity = capture_child_identity(process.child_mut())?;
            std::fs::write(&marker,
                format!("{} {}", identity.pid, identity.platform_start_key))?;
            // Deliberately prevent callback cleanup. The parent timeout kills
            // the helper, leaving its registered watchdog to terminate sleep.
            std::thread::sleep(Duration::from_secs(60));
            runtime.block_on(process.shutdown())?;
            Ok(Vec::new())
        })
    };
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(2));
    let text = std::fs::read_to_string(&marker)?;
    let mut fields = text.split_whitespace();
    let identity = nomi_process_runtime::ExactProcessIdentity {
        pid: fields.next().unwrap().parse().unwrap(),
        platform_start_key: fields.next().unwrap().parse().unwrap(),
        start_time_epoch_seconds: 0,
        executable: None,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if probe_process_identity(identity.pid)?.is_none_or(|live| !same_recorded_process(&identity, &live)) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("managed probe survived isolated helper timeout"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(platform_lifecycle_metrics().poller_threads, 0);
    // SAFETY: all managed lifecycle workers were confined to helpers; this
    // host is still the single main thread at the actual mutation point.
    unsafe { std::env::set_var("NOMIFUN_STARTUP_PROBE_HARNESS", "safe"); }
    assert_eq!(std::env::var("NOMIFUN_STARTUP_PROBE_HARNESS").as_deref(), Ok("safe"));
    println!("startup isolation, bounds, timeout cleanup, and single-thread environment mutation passed");
    Ok(())
}

#[cfg(unix)]
fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()
}
