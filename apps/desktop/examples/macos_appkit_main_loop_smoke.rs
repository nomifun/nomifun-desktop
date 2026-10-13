//! Native component acceptance for the production CF dispatcher and WK owner.
//!
//! Build: cargo build -p nomifun-desktop --example macos_appkit_main_loop_smoke
//! Run: target/debug/examples/macos_appkit_main_loop_smoke --report <new.json>
//!      [--case all|main-loop|deferred-engine]
//! The report's existing parent must be an owned, private 0700 directory. The
//! report is created once with 0600 permissions; existing evidence is rejected.
//! No backend, database, network, permissions, or desktop exit receipt is used.
//! A failed modal deadline remains a failure even when the same WK futures
//! subsequently settle after the original Tauri event callback returns.

#[cfg(target_os = "macos")]
#[path = "../src/macos_appkit_main_loop.rs"]
mod main_loop;
#[cfg(target_os = "macos")]
use main_loop as macos_appkit_main_loop;
#[cfg(target_os = "macos")]
#[path = "../src/browser_surface/macos/lifecycle.rs"]
mod lifecycle;

#[cfg(target_os = "macos")]
mod native {
    use std::{
        fs::{File, OpenOptions},
        future::Future,
        io::Write,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        task::{Context, Poll},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context as _, anyhow, bail, ensure};
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSModalPanelRunLoopMode, NSWindow};
    use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
    use serde::Serialize;
    use tauri::{AppHandle, Manager};

    use super::{lifecycle::DeferredEngine, main_loop::post_to_main};

    const MODAL_BOUND: Duration = Duration::from_secs(5);
    const HANDSHAKE_BOUND: Duration = Duration::from_secs(2);
    const SETTLEMENT_BOUND: Duration = Duration::from_secs(10);
    const MAX_REPORT_BYTES: usize = 64 * 1024;

    #[derive(Clone, Copy, Serialize)]
    #[serde(rename_all = "kebab-case")]
    enum Selection {
        All,
        MainLoop,
        DeferredEngine,
    }

    struct Options {
        selection: Selection,
        report_path: PathBuf,
        report: File,
    }

    impl Options {
        fn parse() -> anyhow::Result<Self> {
            let mut arguments = std::env::args_os().skip(1);
            let mut path = None;
            let mut selection = Selection::All;
            let mut selected = false;
            while let Some(argument) = arguments.next() {
                if argument == "--report" && path.is_none() {
                    path = arguments.next().map(PathBuf::from);
                } else if argument == "--case" && !selected {
                    selection = match arguments.next().and_then(|s| s.into_string().ok()).as_deref() {
                        Some("all") => Selection::All,
                        Some("main-loop") => Selection::MainLoop,
                        Some("deferred-engine") => Selection::DeferredEngine,
                        _ => bail!("--case must be all, main-loop, or deferred-engine"),
                    };
                    selected = true;
                } else {
                    bail!("use --report <new absolute JSON path> [--case all|main-loop|deferred-engine]");
                }
            }
            let report_path = path.context("--report is required")?;
            ensure!(report_path.is_absolute() && report_path.file_name().is_some(), "an absolute report file is required");
            let parent = report_path.parent().context("report parent is missing")?;
            let metadata = std::fs::symlink_metadata(parent)?;
            ensure!(metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o700, "report parent must be an owned 0700 directory");
            let report = OpenOptions::new().write(true).create_new(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&report_path)?;
            let metadata = report.metadata()?;
            ensure!(metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o600, "report must be an owned 0600 regular file");
            Ok(Self { selection, report_path, report })
        }
    }

    #[derive(Clone, Serialize)]
    struct NativeWitness {
        source: &'static str,
        main_thread: bool,
        current_mode: Option<String>,
        expected_mode: String,
        owned_modal_matches: bool,
        original_event_callback_active: bool,
        accepted: bool,
    }

    #[derive(Clone, Default, Serialize)]
    struct PollWitness {
        production_get_first_poll_pending: bool,
        production_shutdown_first_poll_pending: bool,
        polled_before_original_event_callback_return: bool,
    }

    #[derive(Clone, Default, Serialize)]
    struct WorkerWitness {
        actual_futures_settled: bool,
        get_rejected_closed_initialization: bool,
        shutdown_succeeded: bool,
        settled_before_original_event_callback_return: bool,
        completion_dispatch_acknowledged: bool,
    }

    #[derive(Default)]
    struct CaseState {
        poll: PollWitness,
        worker: WorkerWitness,
        completion: Option<NativeWitness>,
        deadline: Option<NativeWitness>,
        failures: Vec<String>,
    }

    struct CaseOwner {
        queued_engine: bool,
        active: AtomicBool,
        callback_returned: AtomicBool,
        modal_returned: AtomicBool,
        completion_calls: AtomicUsize,
        deadline_calls: AtomicUsize,
        state: Mutex<CaseState>,
    }

    impl CaseOwner {
        fn new(queued_engine: bool) -> Arc<Self> {
            Arc::new(Self {
                queued_engine, active: AtomicBool::new(false),
                callback_returned: AtomicBool::new(false), modal_returned: AtomicBool::new(false),
                completion_calls: AtomicUsize::new(0), deadline_calls: AtomicUsize::new(0),
                state: Mutex::new(CaseState::default()),
            })
        }

        fn fail(&self, message: impl Into<String>) {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.failures.len() < 16 { state.failures.push(message.into()); }
        }

        fn report(&self) -> CaseReport {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let completion_once = self.completion_calls.load(Ordering::Acquire) == 1;
            let poll_proven = !self.queued_engine || (state.poll.production_get_first_poll_pending
                && state.poll.production_shutdown_first_poll_pending
                && state.poll.polled_before_original_event_callback_return);
            let cleanup_proven = if self.queued_engine {
                state.worker.actual_futures_settled && state.worker.shutdown_succeeded
            } else {
                self.modal_returned.load(Ordering::Acquire)
                    && self.callback_returned.load(Ordering::Acquire)
            };
            let passed = state.failures.is_empty() && completion_once && poll_proven && cleanup_proven
                && state.completion.as_ref().is_some_and(|witness| witness.accepted)
                && state.worker.completion_dispatch_acknowledged
                && self.modal_returned.load(Ordering::Acquire)
                && self.callback_returned.load(Ordering::Acquire)
                && self.deadline_calls.load(Ordering::Acquire) == 0
                && (!self.queued_engine || (state.worker.settled_before_original_event_callback_return
                    && state.worker.get_rejected_closed_initialization));
            CaseReport {
                case: if self.queued_engine { "deferred-engine-queued-in-event-callback" } else { "worker-to-real-modal-mode" },
                passed, poll: state.poll.clone(), worker: state.worker.clone(),
                completion: state.completion.clone(), deadline: state.deadline.clone(),
                completion_calls: self.completion_calls.load(Ordering::Acquire),
                deadline_calls: self.deadline_calls.load(Ordering::Acquire),
                modal_returned: self.modal_returned.load(Ordering::Acquire),
                original_event_callback_returned: self.callback_returned.load(Ordering::Acquire),
                actual_cleanup_proven: cleanup_proven, failures: state.failures.clone(),
            }
        }
    }

    #[derive(Serialize)]
    struct CaseReport {
        case: &'static str,
        passed: bool,
        poll: PollWitness,
        worker: WorkerWitness,
        completion: Option<NativeWitness>,
        deadline: Option<NativeWitness>,
        completion_calls: usize,
        deadline_calls: usize,
        modal_returned: bool,
        original_event_callback_returned: bool,
        actual_cleanup_proven: bool,
        failures: Vec<String>,
    }

    #[derive(Serialize)]
    struct Report {
        scope: &'static str,
        selection: Selection,
        process_id: u32,
        modal_bound_ms: u128,
        desktop_server_started: bool,
        authenticated_exit_receipt_verified: bool,
        default_mode: Option<NativeWitness>,
        cases: Vec<CaseReport>,
        failures: Vec<String>,
        passed: bool,
    }

    fn current_mode() -> Option<String> {
        CFRunLoop::current()?.current_mode().map(|mode| mode.to_string())
    }

    fn modal_mode() -> String { unsafe { NSModalPanelRunLoopMode }.to_string() }

    fn default_mode() -> anyhow::Result<String> {
        Ok(unsafe { kCFRunLoopDefaultMode }.context("default CF run-loop mode is unavailable")?.to_string())
    }

    fn owned_window(app: &AppHandle) -> anyhow::Result<(tauri::Window, Retained<NSApplication>, usize)> {
        let mtm = MainThreadMarker::new().context("owned window must be inspected on the main thread")?;
        let window = app.get_window("main").context("owned fixture window is gone")?;
        let raw = window.ns_window().map_err(|_| anyhow!("owned native window is unavailable"))?;
        ensure!(!raw.is_null(), "owned native window is null");
        Ok((window, NSApplication::sharedApplication(mtm), raw as usize))
    }

    // The epoch's active bit is checked before inspecting or stopping a modal.
    // A late completion from this case can never stop the next case's window.
    fn finish_modal(owner: &Arc<CaseOwner>, expected_window: usize, source: &'static str) {
        if !owner.active.load(Ordering::Acquire) { return; }
        let main_thread = MainThreadMarker::new();
        let mode = current_mode();
        let expected_mode = modal_mode();
        let application = main_thread.map(NSApplication::sharedApplication);
        let matches = application.as_ref().and_then(|app| app.modalWindow())
            .is_some_and(|window| Retained::as_ptr(&window) as usize == expected_window);
        let event_active = !owner.callback_returned.load(Ordering::Acquire);
        let witness = NativeWitness {
            source, main_thread: main_thread.is_some(), current_mode: mode.clone(),
            expected_mode: expected_mode.clone(), owned_modal_matches: matches,
            original_event_callback_active: event_active,
            accepted: main_thread.is_some() && mode.as_deref() == Some(expected_mode.as_str())
                && matches && event_active,
        };
        let count = if source == "completion" {
            owner.completion_calls.fetch_add(1, Ordering::AcqRel)
        } else {
            owner.deadline_calls.fetch_add(1, Ordering::AcqRel)
        };
        if count != 0 { owner.fail(format!("{source} callback executed more than once")); }
        {
            let mut state = owner.state.lock().unwrap_or_else(|p| p.into_inner());
            if source == "completion" { state.completion = Some(witness.clone()); }
            else { state.deadline = Some(witness.clone()); }
        }
        if !witness.accepted { owner.fail(format!("{source} callback did not witness the owned modal mode")); }
        // Do not hold a Rust lock across AppKit. Even a failed mode assertion
        // may close this exact owned modal so its original event can return.
        if matches && let Some(application) = application {
            application.stopModal();
            // Stop only the innermost run invocation whose public mode was
            // just proven to belong to this fixture's owned modal. This lets
            // runModalForWindow observe stopModal without needing user input.
            if witness.accepted && let Some(run_loop) = CFRunLoop::current() {
                run_loop.stop();
            }
        }
    }

    fn post_completion(owner: Arc<CaseOwner>, expected_window: usize) -> bool {
        let (tx, rx) = mpsc::sync_channel(1);
        let callback_owner = owner.clone();
        if let Err(error) = post_to_main(move || {
            finish_modal(&callback_owner, expected_window, "completion");
            let _ = tx.send(());
        }) {
            owner.fail(format!("production completion dispatch failed: {error}"));
            return false;
        }
        if rx.recv_timeout(MODAL_BOUND).is_err() {
            owner.fail("production completion dispatch did not acknowledge within its bound");
            false
        } else { true }
    }

    fn case_worker(
        app: AppHandle, owner: Arc<CaseOwner>, expected_window: usize,
        polled: mpsc::SyncSender<PollWitness>, begin: mpsc::Receiver<()>, finished: mpsc::SyncSender<()>,
    ) {
        if !owner.queued_engine {
            let _ = polled.send(PollWitness::default());
            if begin.recv_timeout(HANDSHAKE_BOUND).is_err() {
                owner.fail("modal worker did not receive its owned start handshake");
            } else {
                let acknowledged = post_completion(owner.clone(), expected_window);
                owner.state.lock().unwrap_or_else(|p| p.into_inner()).worker.completion_dispatch_acknowledged = acknowledged;
            }
            let _ = finished.send(());
            return;
        }
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => { owner.fail(format!("owned worker runtime could not start: {error}")); let _ = finished.send(()); return; }
        };
        let deferred = Arc::new(DeferredEngine::default());
        let mut get = Box::pin(deferred.get(&app));
        let mut shutdown = Box::pin(deferred.shutdown());
        let waker = futures_util::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        // The main thread is still inside the real Tauri Task callback and is
        // blocked on `polled`. This poll executes get's gate and its real
        // main-thread submission before Pending can be sent to that thread.
        let get_ready = match get.as_mut().poll(&mut context) { Poll::Pending => None, Poll::Ready(value) => Some(value) };
        // Poll the actual shutdown next: it closes admission and waits on the
        // same initialization acknowledgement. No gate/watch value is injected.
        let shutdown_ready = match shutdown.as_mut().poll(&mut context) { Poll::Pending => None, Poll::Ready(value) => Some(value) };
        let witness = PollWitness {
            production_get_first_poll_pending: get_ready.is_none(),
            production_shutdown_first_poll_pending: shutdown_ready.is_none(),
            polled_before_original_event_callback_return: !owner.callback_returned.load(Ordering::Acquire),
        };
        let _ = polled.send(witness);
        let _ = begin.recv_timeout(HANDSHAKE_BOUND);
        // There is deliberately no timeout/drop around either production
        // future. The modal's deadline only releases the owned event callback;
        // this worker retains and awaits both original futures afterwards.
        let (get_result, shutdown_result) = runtime.block_on(async {
            tokio::join!(
                async { match get_ready { Some(result) => result, None => get.await } },
                async { match shutdown_ready { Some(result) => result, None => shutdown.await } },
            )
        });
        let closed = matches!(&get_result, Err(message) if message == "WK host closed before initialization" || message == "WK host is closing");
        let shutdown_succeeded = shutdown_result.is_ok();
        let before_return = !owner.callback_returned.load(Ordering::Acquire);
        {
            let mut state = owner.state.lock().unwrap_or_else(|p| p.into_inner());
            state.worker.actual_futures_settled = true;
            state.worker.get_rejected_closed_initialization = closed;
            state.worker.shutdown_succeeded = shutdown_succeeded;
            state.worker.settled_before_original_event_callback_return = before_return;
        }
        if !closed { owner.fail("the actual get future did not reject closing initialization"); }
        if !shutdown_succeeded { owner.fail("the actual shutdown future returned an error"); }
        if !before_return { owner.fail("initialization only settled after the original event callback returned"); }
        let acknowledged = post_completion(owner.clone(), expected_window);
        owner.state.lock().unwrap_or_else(|p| p.into_inner()).worker.completion_dispatch_acknowledged = acknowledged;
        drop(get_result);
        drop(deferred);
        let _ = finished.send(());
    }

    fn run_owned_modal(app: AppHandle, owner: Arc<CaseOwner>, finished: mpsc::SyncSender<()>) {
        let prepared = (|| -> anyhow::Result<_> {
            let (window, application, raw) = owned_window(&app)?;
            ensure!(application.modalWindow().is_none(), "another modal is already active in the fixture App");
            let (polled_tx, polled_rx) = mpsc::sync_channel(1);
            let (begin_tx, begin_rx) = mpsc::sync_channel(1);
            let worker_owner = owner.clone();
            thread::Builder::new().name("mac40-component-owned-worker".into()).spawn(move || {
                case_worker(app, worker_owner, raw, polled_tx, begin_rx, finished);
            })?;
            let polled = polled_rx.recv_timeout(HANDSHAKE_BOUND).context("actual first-poll handshake was not received")?;
            let valid = !owner.queued_engine || (polled.production_get_first_poll_pending
                && polled.production_shutdown_first_poll_pending && polled.polled_before_original_event_callback_return);
            owner.state.lock().unwrap_or_else(|p| p.into_inner()).poll = polled;
            ensure!(valid, "the actual first get/shutdown polls did not prove pending initialization inside the event callback");
            Ok((window, application, raw, begin_tx))
        })();
        match prepared {
            Err(error) => owner.fail(format!("owned modal preparation failed: {error}")),
            Ok((_window, application, raw, begin_tx)) => {
                let (cancel_tx, cancel_rx) = mpsc::sync_channel(1);
                let deadline_owner = owner.clone();
                let timer = thread::Builder::new().name("mac40-owned-modal-deadline".into()).spawn(move || {
                    if cancel_rx.recv_timeout(MODAL_BOUND).is_ok() || !deadline_owner.active.load(Ordering::Acquire) { return; }
                    deadline_owner.fail("owned modal deadline expired; the original production futures remain retained");
                    let callback_owner = deadline_owner.clone();
                    if let Err(error) = post_to_main(move || finish_modal(&callback_owner, raw, "deadline")) {
                        deadline_owner.fail(format!("production modal deadline dispatch failed: {error}"));
                    }
                });
                match timer {
                    Err(error) => owner.fail(format!("owned modal deadline could not start: {error}")),
                    Ok(timer) => {
                        owner.active.store(true, Ordering::Release);
                        let _ = begin_tx.send(());
                        // This pointer comes only from the retained Tauri
                        // window above, on its owning main thread.
                        let window = unsafe { &*(raw as *const NSWindow) };
                        window.makeKeyAndOrderFront(None);
                        let started = Instant::now();
                        let _response = application.runModalForWindow(window);
                        owner.active.store(false, Ordering::Release);
                        owner.modal_returned.store(true, Ordering::Release);
                        let _ = cancel_tx.send(());
                        if timer.join().is_err() { owner.fail("owned modal deadline worker panicked"); }
                        if started.elapsed() > SETTLEMENT_BOUND { owner.fail("native modal exceeded the outer case bound"); }
                    }
                }
            }
        }
        owner.callback_returned.store(true, Ordering::Release);
        // Return to Tao now. A queued initializer can then settle even when
        // it was excluded from the nested modal; its first failure is retained.
    }

    fn verify_default() -> anyhow::Result<NativeWitness> {
        let expected = default_mode()?;
        let calls = Arc::new(AtomicUsize::new(0));
        let callback_calls = calls.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        post_to_main(move || {
            let current_mode = current_mode();
            let main_thread = MainThreadMarker::new().is_some();
            let once = callback_calls.fetch_add(1, Ordering::AcqRel) == 0;
            let accepted = once && main_thread && current_mode.as_deref() == Some(expected.as_str());
            let _ = tx.send(NativeWitness {
                source: "default-completion", main_thread, current_mode,
                expected_mode: expected, owned_modal_matches: false,
                original_event_callback_active: false, accepted,
            });
        })?;
        let witness = rx.recv_timeout(MODAL_BOUND).context("default-mode production dispatch did not acknowledge")?;
        ensure!(calls.load(Ordering::Acquire) == 1, "default-mode callback was not exactly once");
        Ok(witness)
    }

    fn execute_case(app: &AppHandle, queued_engine: bool) -> Arc<CaseOwner> {
        let deadline = Instant::now() + SETTLEMENT_BOUND;
        let owner = CaseOwner::new(queued_engine);
        let callback_owner = owner.clone();
        let callback_app = app.clone();
        let (worker_tx, worker_rx) = mpsc::sync_channel(1);
        let (returned_tx, returned_rx) = mpsc::sync_channel(1);
        if let Err(error) = app.run_on_main_thread(move || {
            run_owned_modal(callback_app, callback_owner, worker_tx);
            let _ = returned_tx.send(());
        }) {
            owner.fail(format!("real Tauri event callback submission failed: {error}"));
            return owner;
        }
        if returned_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
            owner.fail("the original event callback did not return within the case bound");
        }
        if worker_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
            owner.fail("actual original worker settlement remains unconfirmed; no exit is authorized");
        }
        owner
    }

    fn run_driver(app: AppHandle, mut options: Options, completed: Arc<AtomicBool>, success: Arc<AtomicBool>, exit_allowed: Arc<AtomicBool>) {
        let mut report = Report {
            scope: "production CF main-loop dispatcher and DeferredEngine components",
            selection: options.selection, process_id: std::process::id(),
            modal_bound_ms: MODAL_BOUND.as_millis(), desktop_server_started: false,
            authenticated_exit_receipt_verified: false, default_mode: None,
            cases: Vec::new(), failures: Vec::new(), passed: false,
        };
        if matches!(options.selection, Selection::All | Selection::MainLoop) {
            match verify_default() {
                Ok(witness) => {
                    if !witness.accepted { report.failures.push("default-mode witness failed".into()); }
                    report.default_mode = Some(witness);
                }
                Err(error) => report.failures.push(format!("default-mode case failed: {error}")),
            }
            report.cases.push(execute_case(&app, false).report());
        }
        if matches!(options.selection, Selection::All | Selection::DeferredEngine) {
            let queued = execute_case(&app, true).report();
            report.cases.push(queued);
        }
        let can_exit = report.cases.iter().all(|case| case.actual_cleanup_proven && case.original_event_callback_returned);
        report.passed = report.failures.is_empty() && !report.cases.is_empty() && report.cases.iter().all(|case| case.passed);
        let written = (|| -> anyhow::Result<()> {
            let bytes = serde_json::to_vec_pretty(&report)?;
            ensure!(bytes.len() <= MAX_REPORT_BYTES, "component report exceeded its byte budget");
            options.report.write_all(&bytes)?;
            options.report.write_all(b"\n")?;
            options.report.sync_all()?;
            Ok(())
        })();
        let passed = report.passed && written.is_ok();
        if let Err(error) = written { eprintln!("MACOS_APPKIT_MAIN_LOOP_SMOKE_FAIL report write failed: {error}"); }
        eprintln!("MACOS_APPKIT_MAIN_LOOP_SMOKE_RESULT passed={passed} actual_cleanup_proven={can_exit} report={}", options.report_path.display());
        success.store(passed, Ordering::Release);
        completed.store(true, Ordering::Release);
        exit_allowed.store(can_exit, Ordering::Release);
        if can_exit { app.exit(if passed { 0 } else { 1 }); }
        else { eprintln!("MACOS_APPKIT_MAIN_LOOP_SMOKE_FAIL original WK owner remains retained; fixture stays alive"); }
    }

    pub fn run() -> anyhow::Result<i32> {
        let options = Options::parse()?;
        let completed = Arc::new(AtomicBool::new(false));
        let success = Arc::new(AtomicBool::new(false));
        let exit_allowed = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let driver_options = Arc::new(Mutex::new(Some(options)));
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = "com.nomifun.desktop.audit.mainloop".into();
        context.config_mut().product_name = Some("NomiFun AppKit Main Loop Audit".into());
        let app = tauri::Builder::default()
            .invoke_handler(|invoke| { invoke.resolver.reject("This native component fixture has no application commands"); true })
            .setup(|app| {
                tauri::window::WindowBuilder::new(app, "main")
                    .title("NomiFun AppKit Main Loop Audit — owned modal")
                    .inner_size(880.0, 600.0).min_inner_size(880.0, 600.0).build()?;
                Ok(())
            })
            .build(context)?;
        let driver_completed = completed.clone();
        let driver_success = success.clone();
        let result = app.run_return(move |handle, event| {
            if matches!(event, tauri::RunEvent::Ready) && !started.swap(true, Ordering::AcqRel) {
                let options = driver_options.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(options) = options {
                    let handle = handle.clone();
                    let completed = driver_completed.clone();
                    let success = driver_success.clone();
                    let exit_allowed = exit_allowed.clone();
                    if let Err(error) = thread::Builder::new().name("mac40-component-driver".into())
                        .spawn(move || run_driver(handle, options, completed, success, exit_allowed)) {
                        eprintln!("MACOS_APPKIT_MAIN_LOOP_SMOKE_FAIL driver could not start: {error}");
                    }
                }
            }
            match event {
                tauri::RunEvent::ExitRequested { api, .. } if !exit_allowed.load(Ordering::Acquire) => api.prevent_exit(),
                tauri::RunEvent::WindowEvent { event: tauri::WindowEvent::CloseRequested { api, .. }, .. } => api.prevent_close(),
                _ => {}
            }
        });
        Ok(if result == 0 && completed.load(Ordering::Acquire) && success.load(Ordering::Acquire) { 0 } else { 1 })
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let code = match native::run() {
        Ok(code) => code,
        Err(error) => { eprintln!("MACOS_APPKIT_MAIN_LOOP_SMOKE_FAIL {error}"); 1 }
    };
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macos_appkit_main_loop_smoke requires macOS 14 or newer");
    std::process::exit(1);
}
