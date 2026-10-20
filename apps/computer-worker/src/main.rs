// Copyright (c) 2025 Cua AI, Inc. MIT License; see ../LICENSE.
// The private worker startup is adapted from trycua/cua's main.rs at
// 4d959bbfa57f0c89f310330e849500b222084194.

mod private_worker;
mod probe;

enum Command {
    Worker(String),
    SelfCheck,
}

fn command(args: &[String]) -> anyhow::Result<Command> {
    match args {
        [flag] if flag == "--self-check" => Ok(Command::SelfCheck),
        [worker, flag, generation]
            if worker == "__private-worker"
                && flag == "--generation"
                && !generation.is_empty()
                && generation.len() <= 128
                && generation
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-') =>
        {
            Ok(Command::Worker(generation.clone()))
        }
        _ => anyhow::bail!("expected __private-worker --generation <id> or --self-check"),
    }
}

fn main() {
    #[cfg(target_os = "windows")]
    if let Some(code) = platform_windows::user_app_launch::run_helper_if_requested() {
        std::process::exit(code);
    }
    cua_driver_core::build_info::init(env!("CUA_DRIVER_GIT_SHA"));
    let result = match command(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(Command::SelfCheck) => probe::self_check(),
        Ok(Command::Worker(generation)) => run_worker(generation),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("nomifun-computer-worker: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn run_worker(generation: String) -> anyhow::Result<()> {
    private_worker::run(generation, None)
}

#[cfg(target_os = "linux")]
fn run_worker(generation: String) -> anyhow::Result<()> {
    // Preserve the SDK's X11 authentication preparation before the native
    // cleanup owner opens its first display. With no DISPLAY (protocol probes),
    // this is a no-op; an explicitly supplied private Xauthority stays intact.
    platform_linux::xauth::ensure_xauthority_discovered();
    // SAFETY: this is the fresh worker's initial thread. No SDK, Tokio or
    // platform worker thread has been started. The native cleanup owner must
    // be prepared here, before private_worker creates its runtime.
    let input_owner = unsafe { platform_linux::input::prepare_process_xtest_owner()? };
    let execution = private_worker::run(generation, None);
    let cleanup = match input_owner {
        Some(owner) => owner.finish(),
        None => Ok(()),
    };
    match (execution, cleanup) {
        (Err(execution), Err(cleanup)) => {
            Err(execution.context(format!("native input cleanup also failed: {cleanup:#}")))
        }
        (execution, Ok(())) => execution,
        (Ok(()), Err(cleanup)) => Err(cleanup),
    }
}

#[cfg(target_os = "macos")]
fn run_worker(generation: String) -> anyhow::Result<()> {
    let (initialized_tx, initialized_rx) = std::sync::mpsc::sync_channel(1);
    let protocol = std::thread::spawn(move || {
        let code = match private_worker::run(generation, Some(initialized_tx)) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("nomifun-computer-worker: {error:#}");
                1
            }
        };
        // AppKit's loop is process-long. Closing this directly owned protocol
        // channel ends the worker; it has no reconnectable endpoint.
        std::process::exit(code);
    });
    if initialized_rx.recv().unwrap_or(false) {
        platform_macos::cursor::overlay::run_on_main_thread();
    } else {
        // Preserve the protocol thread's exit status when initialization fails.
        // Returning from main here could otherwise race its process::exit(1).
        protocol
            .join()
            .map_err(|_| anyhow::anyhow!("private worker initialization thread panicked"))?;
    }
    Ok(())
}
