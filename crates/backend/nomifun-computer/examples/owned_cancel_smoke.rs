//! Windows opt-in, read-only cancellation against a newly spawned owned fixture.
//! Usage: owned_cancel_smoke <absolute worker> <absolute owned WinForms fixture>
//! A naturally unsatisfied verify_state keeps the real worker occupied; B is
//! withdrawn before dispatch, then A's real result is joined and C reads once.
//! This is a Computer host/worker witness, not Agent Runtime/canonical Stop,
//! desktop-input cancellation, a crash test, or a model acceptance case.

#[cfg(windows)]
mod native {
    use nomifun_agent_contracts::digest_bytes;
    use nomifun_agent_kernel::InvocationControl;
    use nomifun_computer::{
        CallError, CallErrorKind, Computer, DriverResult, WorkerConfig, WorkerStatus,
        protocol::ActionCompletion, result,
    };
    use serde_json::{Value, json};
    use std::{
        collections::BTreeSet,
        ffi::c_void,
        future::Future,
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::Arc,
        time::{Duration, Instant},
    };
    use tokio::{sync::oneshot, task::JoinHandle};
    use tokio_util::sync::CancellationToken;

    type Check<T> = Result<T, Box<dyn std::error::Error>>;
    type Handle = *mut c_void;
    type CallTask = JoinHandle<Result<DriverResult, CallError>>;
    const VERIFY_TIMEOUT_MS: u64 = 10_000;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetProcessId(process: Handle) -> u32;
        fn GetProcessTimes(process: Handle, created: *mut FileTime, exited: *mut FileTime,
            kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, path: *mut u16,
            length: *mut u32) -> i32;
        fn WaitForSingleObject(process: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn IsWindow(window: Handle) -> i32;
        fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
    }

    fn require(condition: bool, reason: &'static str) -> Check<()> {
        if condition { Ok(()) } else { Err(reason.into()) }
    }
    fn write(root: &Path, name: &str, value: &Value) -> Check<()> {
        std::fs::write(root.join(name), serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
    fn read(path: &Path) -> Check<Value> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    fn hash(path: &Path) -> Check<String> {
        Ok(digest_bytes(&std::fs::read(path)?).as_ref().to_owned())
    }
    fn alive(process: Handle) -> Check<bool> {
        match unsafe { WaitForSingleObject(process, 0) } {
            0 => Ok(false),
            258 => Ok(true),
            _ => Err(io::Error::last_os_error().into()),
        }
    }
    fn identity(process: Handle, expected: &Path) -> Check<Value> {
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let (mut created, mut exited, mut kernel, mut user) = (
            FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
        if unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) } == 0
            || unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let image = PathBuf::from(String::from_utf16(&path[..length as usize])?).canonicalize()?;
        require(image.to_string_lossy().eq_ignore_ascii_case(&expected.canonicalize()?.to_string_lossy()),
            "retained native process image differs from the supplied binary")?;
        let pid = unsafe { GetProcessId(process) };
        require(pid != 0 && alive(process)?, "retained native process is not alive")?;
        Ok(json!({"pid":pid,"image":image,
            "creation_filetime":(((created.high as u64)<<32)|created.low as u64).to_string()}))
    }
    fn owned_window(child: &Child, window: u64) -> Check<()> {
        let mut owner = 0;
        let handle = window as usize as Handle;
        require(alive(child.as_raw_handle())? && unsafe { IsWindow(handle) } != 0
            && unsafe { GetWindowThreadProcessId(handle, &mut owner) } != 0 && owner == child.id(),
            "exact live fixture Child no longer owns the target HWND")
    }
    fn same_worker(status: &WorkerStatus, pid: u32, generation: &str) -> Check<()> {
        require(status.running && status.process_id == Some(pid)
            && status.generation.as_deref() == Some(generation) && status.bound_sessions == 1,
            "worker process, generation or original bound session changed")
    }
    fn completed(reply: &DriverResult) -> Check<Value> {
        require(reply.completion == ActionCompletion::Completed && !reply.foreground,
            "read-only request did not return its real background Completed result")?;
        require(!reply.result["content"].as_array().is_some_and(|parts|
            parts.iter().any(|part|part["type"] == "image")), "read-only case returned raw image content")?;
        let normalized = result::normalize(&reply.result, false);
        require(!normalized.is_error, "read-only native request was refused")?;
        normalized.structured_content.ok_or_else(|| "native structured result missing".into())
    }
    fn untouched(state: &Value, pid: u32, window: u64) -> Check<()> {
        let elements = state["elements"].as_array().ok_or("native AX elements missing")?;
        require(state["pid"] == pid && state["window_id"] == window
            && state["snapshot_id"].as_str().is_some_and(|id|!id.is_empty())
            && elements.iter().filter(|e|e["label"] == "Owned calculation" && e["value"] == "0").count() == 1
            && elements.iter().any(|e|e["label"] == "Owned editor")
            && !state["screenshot_width"].as_u64().is_some_and(|v|v > 0),
            "AX snapshot did not prove the exact untouched owned window")
    }
    fn ax_args(pid: u32, window: u64) -> Value {
        json!({"pid":pid,"window_id":window,"include_screenshot":false,
            "tree_format":"elements","max_elements":100,"timeout_ms":3000})
    }

    // The actor does not drain its mpsc inbox while A is in flight, so
    // WorkerStatus.queued is not an inbox-depth witness. A first pending poll
    // proves B reached Computer.call's synchronous send before cancellation.
    // Its exact Cancelled error then proves the host kept it before DISPATCHED.
    async fn admitted(future: impl Future<Output=Result<DriverResult,CallError>> + Send + 'static)
        -> Check<CallTask> {
        let (sender, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut future = Box::pin(future);
            let mut sender = Some(sender);
            std::future::poll_fn(|context| {
                let result = future.as_mut().poll(context);
                if let Some(sender) = sender.take() { let _ = sender.send(result.is_pending()); }
                result
            }).await
        });
        require(receiver.await?, "B completed before its admission barrier")?;
        Ok(task)
    }

    struct Fixture { root: PathBuf, child: Child }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::write(self.root.join("close.request"), b"close only this fixture"); }
    }
    impl Fixture {
        async fn close(&mut self) -> Check<Value> {
            std::fs::write(self.root.join("close.request"), b"close only this fixture")?;
            let started = Instant::now();
            let exit = loop {
                if let Some(exit) = self.child.try_wait()? { break exit; }
                if started.elapsed() > Duration::from_secs(10) {
                    return Err("exact fixture did not exit normally; no forced stop attempted".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            let receipt = read(&self.root.join("exit.json"))?;
            require(exit.success() && !alive(self.child.as_raw_handle())?
                && receipt["pid"] == self.child.id() && receipt["graceful"] == true
                && receipt["control_file"] == true, "fixture normal exit did not match the retained Child")?;
            Ok(json!({"exact_child_exited":true,"pid":self.child.id(),"exit_code":exit.code(),
                "graceful_receipt":true,"control_file":true}))
        }
    }

    pub async fn run() -> Check<()> {
        let args = std::env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>();
        require(args.len() == 2, "pass absolute real worker and owned WinForms fixture paths")?;
        let (worker, fixture_exe) = (&args[0], &args[1]);
        require(worker.is_absolute() && worker.is_file() && fixture_exe.is_absolute() && fixture_exe.is_file(),
            "both explicitly supplied binaries must be absolute existing files")?;
        let run = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("nomifun-computer-cancel-{run}"));
        std::fs::create_dir(&root)?;
        println!("OWNED_CANCEL_ARTIFACTS {}", root.display());
        let mut report = json!({"schema":"nomifun.computer-owned-cancel.v1","ok":false,
            "scope":"one Computer host/real worker; read-only per-invocation cancellation; no canonical Agent Stop or input-effect claim",
            "root":root,"host_pid":std::process::id(),"host_sha256":hash(&std::env::current_exe()?)?,
            "source_sha256":digest_bytes(include_bytes!("owned_cancel_smoke.rs")).as_ref(),
            "worker_path":worker,"worker_sha256":hash(worker)?,"fixture_path":fixture_exe,"fixture_sha256":hash(fixture_exe)?,
            "verify_timeout_ms":VERIFY_TIMEOUT_MS,"input_calls_requested":0,"screenshots_requested":0,
            "forced_stop_requested":false,"automatic_replay":false});
        write(&root,"cancel-report.json",&report)?;
        let child = Command::new(fixture_exe).arg("functional").arg(&root)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        let mut fixture = Fixture { root:root.clone(),child };
        let computer = Arc::new(Computer::new(WorkerConfig::new(worker,root.join("host-data"))));
        let authorized = BTreeSet::from(["computer/get_window_state".to_owned(),"computer/verify_state".to_owned()]);
        let session = format!("owned-cancel-a-{run}");
        let queued_session = format!("owned-cancel-b-{run}");
        let cancel_a = CancellationToken::new();
        let cancel_b = CancellationToken::new();
        let mut active: Option<CallTask> = None;
        let mut queued: Option<CallTask> = None;
        let mut witness: Option<OwnedHandle> = None;
        let mut submissions = 0;
        let mut native_completions = 0;
        let mut queued_cancelled = 0;
        let started = Instant::now();
        let outcome: Check<()> = async {
            tokio::time::timeout(Duration::from_secs(20),async {
                while !root.join("ready.json").is_file() { tokio::time::sleep(Duration::from_millis(20)).await; }
            }).await?;
            let ready = read(&root.join("ready.json"))?;
            let pid = fixture.child.id();
            let window = ready["window"].as_str().ok_or("fixture HWND missing")?.parse::<u64>()?;
            let native = identity(fixture.child.as_raw_handle(),fixture_exe)?;
            require(ready["pid"] == pid && ready["creation_filetime"] == native["creation_filetime"],
                "fixture ready receipt did not identify the retained exact Child")?;
            owned_window(&fixture.child,window)?;
            report["fixture_identity"] = native;
            report["window_id"] = json!(window);
            report["dpi"] = ready["dpi"].clone();
            submissions += 1;
            let baseline = computer.call(&session,"Owned read-only cancellation","computer/get_window_state",
                ax_args(pid,window),&authorized,false,InvocationControl::default()).await?;
            native_completions += 1;
            let state = completed(&baseline)?;
            untouched(&state,pid,window)?;
            write(&root,"baseline-ax.json",&baseline.result)?;
            let before = computer.status();
            let worker_pid = before.process_id.ok_or("worker PID missing")?;
            let generation = baseline.generation.clone();
            same_worker(&before,worker_pid,&generation)?;
            let handle = unsafe { OpenProcess(0x0010_0000|0x1000,0,worker_pid) };
            if handle.is_null() { return Err(io::Error::last_os_error().into()); }
            let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
            report["worker_identity"] = identity(owned.as_raw_handle(),worker)?;
            report["generation"] = json!(generation);
            witness = Some(owned);
            let impossible_value = format!("never-written-{run}");
            let arguments = json!({"pid":pid,"window_id":window,"include_screenshot":false,
                "timeout_ms":VERIFY_TIMEOUT_MS,"stable_samples":1,
                "expect":[{"element":{"selector":{"label_contains":"Owned calculation"},"value_equals":impossible_value}}]});
            report["a_arguments"] = arguments.clone();
            owned_window(&fixture.child,window)?;
            let (a_computer,a_authorized,a_session,a_control) =
                (computer.clone(),authorized.clone(),session.clone(),cancel_a.clone());
            submissions += 1;
            active = Some(tokio::spawn(async move {
                a_computer.call(&a_session,"Owned read-only cancellation","computer/verify_state",arguments,
                    &a_authorized,false,InvocationControl::new(a_control)).await
            }));
            tokio::time::timeout(Duration::from_secs(5),async {
                loop {
                    require(!active.as_ref().unwrap().is_finished(),"A finished before an in-flight cancellation witness")?;
                    let status = computer.status();
                    same_worker(&status,worker_pid,&generation)?;
                    if status.phase == "busy" { return Ok::<_,Box<dyn std::error::Error>>(status); }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await??;
            report["a_busy_observed_ms"] = json!(started.elapsed().as_millis());
            let (b_computer,b_authorized,b_session,b_control) =
                (computer.clone(),authorized.clone(),queued_session.clone(),cancel_b.clone());
            submissions += 1;
            queued = Some(admitted(async move {
                b_computer.call(&b_session,"Owned queued cancellation","computer/get_window_state",ax_args(pid,window),
                    &b_authorized,false,InvocationControl::new(b_control)).await
            }).await?);
            report["b_admission"] = json!({"single_poll_pending":true,"observed_ms":started.elapsed().as_millis(),
                "source":"Computer.call synchronous send before its first pending await; queued status excludes mpsc inbox"});
            require(!active.as_ref().unwrap().is_finished(),"A completed before cancellation signals")?;
            let before_cancel = computer.status();
            same_worker(&before_cancel,worker_pid,&generation)?;
            require(before_cancel.phase == "busy", "A was not busy at the cancellation boundary")?;
            report["before_cancel"] = json!(before_cancel);
            cancel_b.cancel();
            cancel_a.cancel();
            report["cancellation_signals_ms"] = json!(started.elapsed().as_millis());
            let b_result = tokio::time::timeout(Duration::from_secs(2),queued.as_mut().unwrap()).await??;
            queued.take();
            report["b_result"] = json!(b_result);
            let error = b_result.err().ok_or("queued B unexpectedly reached a successful native result")?;
            require(error.kind == CallErrorKind::Cancelled && error.code == "COMPUTER_CALL_CANCELLED",
                "B did not report exact pre-dispatch cancellation")?;
            queued_cancelled += 1;
            require(!active.as_ref().unwrap().is_finished(),"A result was not still pending when B withdrew")?;
            report["b_cancelled_while_a_pending_ms"] = json!(started.elapsed().as_millis());
            let waiting = tokio::time::timeout(Duration::from_secs(2),async {
                loop {
                    require(!active.as_ref().unwrap().is_finished(),"A finished before waiting-phase observation")?;
                    let status = computer.status();
                    same_worker(&status,worker_pid,&generation)?;
                    if status.phase == "waiting_for_current_operation" { return Ok::<_,Box<dyn std::error::Error>>(status); }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await??;
            report["a_waiting"] = json!({"status":waiting,"observed_ms":started.elapsed().as_millis()});
            let a_result = active.as_mut().unwrap().await?;
            active.take();
            report["a_result"] = json!(a_result);
            let a_reply = a_result?;
            native_completions += 1;
            let verification = completed(&a_reply)?;
            write(&root,"a-verify-result.json",&a_reply.result)?;
            let predicates = verification["predicates"].as_array().ok_or("verification predicates missing")?;
            require(a_reply.generation == generation && verification["status"] == "unsatisfied"
                && verification["stable"] == false && verification["elapsed_ms"].as_u64().is_some_and(|ms|ms >= VERIFY_TIMEOUT_MS)
                && verification["samples"].as_u64().is_some_and(|count|count >= 2)
                && predicates.len() == 1 && predicates[0]["status"] == "unsatisfied"
                && predicates[0]["unknown_reason"].is_null(),
                "A was not the real naturally expired unsatisfied verification; Cancelled/Unknown are not success")?;
            let observed: Value = serde_json::from_str(predicates[0]["observed_json"].as_str().ok_or("matched native element evidence missing")?)?;
            require(observed["label"] == "Owned calculation" && observed["value"] == "0",
                "verification did not observe the unchanged dedicated control")?;
            same_worker(&computer.status(),worker_pid,&generation)?;
            owned_window(&fixture.child,window)?;
            // C is an explicit new read, never a replay/resume of cancelled A/B.
            submissions += 1;
            let successor = computer.call(&session,"Owned read-only cancellation","computer/get_window_state",
                ax_args(pid,window),&authorized,false,InvocationControl::default()).await?;
            native_completions += 1;
            let state = completed(&successor)?;
            untouched(&state,pid,window)?;
            require(successor.generation == generation,"C used a different worker generation")?;
            same_worker(&computer.status(),worker_pid,&generation)?;
            require(alive(witness.as_ref().unwrap().as_raw_handle())?,"original worker handle exited before C")?;
            write(&root,"c-successor-ax.json",&successor.result)?;
            report["c_result"] = json!({"completion":successor.completion,"generation":successor.generation,
                "snapshot_id":state["snapshot_id"],"original_session_usable":true,"same_worker_handle_alive":true});
            report["status_after_c"] = json!(computer.status());
            Ok(())
        }.await;

        // Always retain and join original request futures before ordinary
        // shutdown. A timeout/failure in the witness never authorizes replay,
        // aborting an OS operation, another input, or a process kill by this test.
        cancel_b.cancel();
        cancel_a.cancel();
        if let Some(task) = queued.take() { report["pending_b_cleanup_result"] = json!(task.await.map_err(|e|e.to_string())); }
        if let Some(task) = active.take() { report["pending_a_cleanup_result"] = json!(task.await.map_err(|e|e.to_string())); }
        let close_a = computer.close_session(&session).await;
        let close_b = computer.close_session(&queued_session).await;
        let shutdown = computer.shutdown().await;
        let worker_exit: Check<Value> = async {
            let handle = witness.as_ref().ok_or("no original native worker witness")?.as_raw_handle();
            tokio::time::timeout(Duration::from_secs(10),async {
                while alive(handle)? { tokio::time::sleep(Duration::from_millis(20)).await; }
                Ok::<_,Box<dyn std::error::Error>>(())
            }).await??;
            let mut code = 0;
            if unsafe { GetExitCodeProcess(handle,&mut code) } == 0 { return Err(io::Error::last_os_error().into()); }
            require(code == 0,"original worker did not exit normally")?;
            Ok(json!({"original_retained_handle_exited":true,"exit_code":code}))
        }.await;
        let fixture_exit = fixture.close().await;
        report["counts"] = json!({"host_submissions":submissions,"completed_native_results":native_completions,
            "queued_pre_dispatch_cancelled":queued_cancelled,"input_calls_requested":0,"screenshots_requested":0});
        report["cleanup"] = json!({"a_session_closed":close_a.is_ok(),"b_session_closed":close_b.is_ok(),
            "normal_computer_shutdown":shutdown.is_ok(),"owner_status":computer.status(),
            "worker":worker_exit.as_ref().ok(),"worker_error":worker_exit.as_ref().err().map(ToString::to_string),
            "fixture":fixture_exit.as_ref().ok(),"fixture_error":fixture_exit.as_ref().err().map(ToString::to_string)});
        report["failure"] = json!(outcome.as_ref().err().map(ToString::to_string));
        let passed = outcome.is_ok() && close_a.is_ok() && close_b.is_ok() && shutdown.is_ok()
            && worker_exit.is_ok() && fixture_exit.is_ok() && !computer.status().running
            && submissions == 4 && native_completions == 3 && queued_cancelled == 1;
        report["ok"] = json!(passed);
        write(&root,"cancel-report.json",&report)?;
        require(passed,"native read-only cancellation witness failed; inspect cancel-report.json")?;
        println!("OWNED_CANCEL_OBSERVED {}",root.join("cancel-report.json").display());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    { native::run().await }
    #[cfg(not(windows))]
    { Err("this read-only native cancellation fixture is Windows-only".into()) }
}
