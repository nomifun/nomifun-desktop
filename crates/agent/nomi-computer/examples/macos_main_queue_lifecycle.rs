//! Native lifecycle acceptance for the production macOS main-queue dispatcher.
//! Uses only owned closures and read-only TCC probes: no input, capture, focus,
//! permission request, application installation, or user-data access.

#[cfg(target_os = "macos")]
#[path = "../src/macos_main.rs"]
#[allow(dead_code)]
mod macos_main;
#[cfg(target_os = "macos")]
#[path = "../src/native_control.rs"]
#[allow(dead_code)]
mod native_control;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macos_main_queue_lifecycle requires macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = acceptance() {
        eprintln!("macOS main-queue lifecycle acceptance failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
fn acceptance() -> Result<(), String> {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}, mpsc};
    use std::time::Duration;

    if objc2::MainThreadMarker::new().is_none() {
        return Err("native acceptance must begin on the process main thread".into());
    }
    let permissions = nomi_computer::permissions::permission_status();
    // When the real process is denied, exercise the original AX actor's
    // typed denial as well. Restrict even a concurrent grant change to this
    // windowless fixture's PID; never inspect another application's content.
    let ax_denial_probe = if permissions.accessibility == Some(false) {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result = nomi_a11y::create_engine().and_then(|actor| actor.observe(&nomi_a11y::ObserveOpts {
                pid: Some(std::process::id() as i32),
                node_budget: 1,
                max_depth: 1,
            }));
            let _ = reply_tx.send(result);
        });
        let result = reply_rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "denied AX actor did not supply its original reply")?;
        worker.join().map_err(|_| "AX denial worker panicked")?;
        if !matches!(result, Err(nomi_a11y::A11yError::Permission(_))) {
            return Err(format!("denied AX process did not retain its typed permission error: {result:?}"));
        }
        "confirmed_denied_before_observe"
    } else {
        "not_run_permission_already_granted"
    };
    let open_fds_before = std::fs::read_dir("/dev/fd")
        .map_err(|e| e.to_string())?
        .count();
    let mut rounds = Vec::new();
    for round in 1..=3 {
        // Keep the real main queue unserviced until the exact queued closure
        // has been withdrawn. The enqueue acknowledgement is a barrier, not a
        // timing guess about whether the worker reached the queue.
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (queued_tx, queued_rx) = mpsc::sync_channel(1);
        let (withdrawn_tx, withdrawn_rx) = mpsc::sync_channel(1);
        let effects = Arc::new(AtomicUsize::new(0));
        let seen = effects.clone();
        let worker = std::thread::spawn({
            let cancellation = cancellation.clone();
            move || {
                let result = macos_main::run_queued_controlled(&cancellation, |task| {
                    dispatch2::DispatchQueue::main().exec_async(task);
                    queued_tx.send(()).unwrap();
                }, move || {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
                let _ = withdrawn_tx.send(result);
            }
        });
        queued_rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
        cancellation.cancel();
        let queued_result = withdrawn_rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "queued cancellation did not settle while the main queue was unavailable")?;
        worker.join().map_err(|_| "queued worker panicked")?;
        if queued_result.as_ref().map_err(String::as_str) != Err(native_control::CANCELLED_BEFORE_DISPATCH) {
            return Err(format!("queued cancellation returned {queued_result:?}"));
        }
        // The marker follows the withdrawn closure in the actual main queue;
        // observing it proves recovery did not execute late native work.
        let (drained_tx, drained_rx) = mpsc::sync_channel(1);
        dispatch2::DispatchQueue::main().exec_async(move || { let _ = drained_tx.send(()); });
        pump_until(drained_rx)?;
        if effects.load(Ordering::SeqCst) != 0 {
            return Err("withdrawn main-queue task executed late".into());
        }

        // Once the original main-queue closure starts, Stop must keep its
        // caller joined until that same closure supplies the real result.
        let cancellation = tokio_util::sync::CancellationToken::new();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let native = std::thread::spawn({
            let cancellation = cancellation.clone();
            move || {
                let result = macos_main::run_blocking_controlled(&cancellation, move || {
                    entered_tx.send(objc2::MainThreadMarker::new().is_some()).unwrap();
                    release_rx.recv().map_err(|e| e.to_string())?;
                    Ok("original native result")
                });
                result_tx.send(result).unwrap();
            }
        });
        let observer = std::thread::spawn(move || -> Result<(), String> {
            if !entered_rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())? {
                return Err("native operation escaped the process main thread".into());
            }
            cancellation.cancel();
            let early = result_rx.recv_timeout(Duration::from_millis(100));
            // Release even after a failure so the fixture cannot strand its
            // own native closure on the process main thread.
            let _ = release_tx.send(());
            if !matches!(early, Err(mpsc::RecvTimeoutError::Timeout)) {
                return Err(format!("Stop abandoned an admitted native result: {early:?}"));
            }
            let result = result_rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
            if result.as_deref() != Ok("original native result") {
                return Err(format!("original native result was replaced: {result:?}"));
            }
            Ok(())
        });
        while !observer.is_finished() {
            pump_once();
        }
        observer.join().map_err(|_| "cancellation observer panicked")??;
        native.join().map_err(|_| "native worker panicked")?;

        // A cancelled client cannot close the shared process main queue.
        let (healthy_tx, healthy_rx) = mpsc::sync_channel(1);
        let healthy = std::thread::spawn(move || {
            let result = macos_main::run_blocking(|| Ok(objc2::MainThreadMarker::new().is_some()));
            let _ = healthy_tx.send(result);
        });
        if !pump_until(healthy_rx)?? {
            return Err("healthy follow-up did not run on the main thread".into());
        }
        healthy.join().map_err(|_| "healthy follow-up panicked")?;
        rounds.push(serde_json::json!({"round":round, "withdrawn_effects":0,
            "admitted_result_joined":true, "healthy_followup_on_main":true,
            "open_fds":std::fs::read_dir("/dev/fd").map_err(|e| e.to_string())?.count()}));
    }
    let after = nomi_computer::permissions::permission_status();
    println!("{}", serde_json::json!({"schema":"nomifun.macos-main-queue-lifecycle.v1",
        "passed":true, "pid":std::process::id(), "open_fds_before":open_fds_before, "rounds":rounds,
        "tcc_before":{"accessibility":permissions.accessibility,"screen_recording":permissions.screen_recording},
        "tcc_after":{"accessibility":after.accessibility,"screen_recording":after.screen_recording},
        "ax_denial_probe":ax_denial_probe,
        "permission_requests":0,"device_effects":0}));
    Ok(())
}

#[cfg(target_os = "macos")]
fn pump_once() {
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, std::time::Duration::from_millis(20), true);
}

#[cfg(target_os = "macos")]
fn pump_until<T>(reply: std::sync::mpsc::Receiver<T>) -> Result<T, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match reply.try_recv() {
            Ok(result) => return Ok(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err("main-queue fixture lost its exact reply".into()),
            Err(std::sync::mpsc::TryRecvError::Empty) => {},
        }
        if std::time::Instant::now() >= deadline {
            return Err("native main queue did not deliver the fixture reply".into());
        }
        pump_once();
    }
}
