//! Explicit native focus acceptance against an isolated audit fixture only.
//! Run with the fixture's PID and exact title. Missing TCC grants fail closed;
//! this probe never requests permission, clicks, types, or captures user pixels.

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = probe() {
        eprintln!("macOS fixture probe failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This native acceptance probe requires macOS.");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn probe() -> Result<(), String> {
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    use std::{sync::mpsc, time::Duration};

    let args: Vec<_> = std::env::args().skip(1).collect();
    let [pid, title] = args.as_slice() else {
        return Err("usage: macos_fixture_probe <fixture PID> <exact fixture title>".to_owned());
    };
    let pid: u32 = pid.parse().map_err(|_| "fixture PID must be a positive integer")?;
    if pid == 0 || !title.starts_with("NomiFun macOS audit fixture ") {
        return Err("only an explicit isolated 'NomiFun macOS audit fixture ' window is accepted".to_owned());
    }
    let status = nomi_computer::permissions::permission_status();
    if status.accessibility != Some(true) || status.screen_recording != Some(true) {
        return Err(format!("native focus acceptance is unavailable for this process identity: {status:?}; no permission request or input was attempted"));
    }
    let windows = xcap::Window::all().map_err(|error| error.to_string())?;
    let matches: Vec<_> = windows.into_iter().filter(|window| {
        window.pid().ok() == Some(pid) && window.title().ok().as_ref() == Some(title)
    }).collect();
    let [window] = matches.as_slice() else {
        return Err("the fixture PID and title must identify exactly one visible window".to_owned());
    };
    let id = window.id().map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = tokio::runtime::Runtime::new()
            .map_err(|error| error.to_string())
            .and_then(|runtime| runtime.block_on(async {
                let focus = nomi_computer::fallback_backend::focus_window(id).await?;
                let cursor = nomi_computer::input::cursor_position().await?;
                Ok(serde_json::json!({"passed":true, "fixture_pid":pid, "window_id":id, "focus":focus, "cursor":cursor}))
            }));
        let _ = sender.send(result);
    });
    // Exercise the production main-queue dispatch path while the operation
    // runs on its normal worker. Keep joining the admitted worker, so this
    // acceptance helper cannot abandon a native activation after a timeout.
    let result = loop {
        match receiver.try_recv() {
            Ok(result) => break result,
            Err(mpsc::TryRecvError::Disconnected) => break Err("native probe worker closed without a result".to_owned()),
            Err(mpsc::TryRecvError::Empty) => {
                CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::from_millis(50), true);
            }
        }
    };
    worker.join().map_err(|_| "native probe worker panicked")?;
    println!("{}", result?);
    Ok(())
}
