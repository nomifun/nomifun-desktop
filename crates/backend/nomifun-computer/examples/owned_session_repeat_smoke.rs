//! Windows native session repetition against one explicitly supplied test window.
//! Usage: owned_session_repeat_smoke <absolute worker> <absolute owned fixture> [measurement rounds]
//! Two warmups and the samples (default ten) share one host, generation and Agent label.
//! Only AX reads occur. Resource counts are evidence, never a leak verdict or quota.
//! This does not exercise the complete Agent Runtime or a real model.

#[cfg(windows)]
mod native {
    use nomifun_agent_contracts::digest_bytes;
    use nomifun_agent_kernel::InvocationControl;
    use nomifun_computer::{Computer, WorkerConfig, protocol::ActionCompletion, result};
    use serde_json::{Value, json};
    use std::{
        collections::BTreeSet,
        ffi::c_void,
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    type Check<T> = Result<T, Box<dyn std::error::Error>>;
    type Handle = *mut c_void;
    const WARMUPS: usize = 2;
    const DEFAULT_MEASUREMENT_ROUNDS: usize = 10;
    const IDLE: Duration = Duration::from_secs(10);
    const SETTLE: Duration = Duration::from_millis(300);
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime { low: u32, high: u32 }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetCurrentProcess() -> Handle;
        fn GetProcessId(process: Handle) -> u32;
        fn GetProcessTimes(process: Handle, created: *mut FileTime, exited: *mut FileTime,
            kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, length: *mut u32) -> i32;
        fn GetProcessHandleCount(process: Handle, count: *mut u32) -> i32;
        fn WaitForSingleObject(process: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn SetLastError(code: u32);
        fn GetLastError() -> u32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetGuiResources(process: Handle, kind: u32) -> u32;
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
    fn file_hash(path: &Path) -> Check<String> {
        Ok(digest_bytes(&std::fs::read(path)?).as_ref().to_owned())
    }
    async fn wait_file(path: &Path) -> Check<()> {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !path.is_file() { tokio::time::sleep(Duration::from_millis(20)).await; }
        }).await?;
        Ok(())
    }
    fn alive(process: Handle) -> Check<bool> {
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => Ok(false), WAIT_TIMEOUT => Ok(true),
            _ => Err(io::Error::last_os_error().into()),
        }
    }
    fn process_identity(process: Handle, expected: &Path) -> Check<Value> {
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
            "native process image differs from the supplied binary")?;
        let pid = unsafe { GetProcessId(process) };
        require(pid != 0 && alive(process)?, "native process is not alive")?;
        Ok(json!({"pid":pid,"image":image,"creation_filetime":(((created.high as u64)<<32)|created.low as u64).to_string()}))
    }
    fn owned_window(child: &Child, window: u64) -> Check<()> {
        let mut owner = 0;
        let window = window as usize as Handle;
        require(alive(child.as_raw_handle())? && unsafe { IsWindow(window) } != 0
            && unsafe { GetWindowThreadProcessId(window, &mut owner) } != 0 && owner == child.id(),
            "the exact live fixture no longer owns the HWND")
    }
    fn gui_count(process: Handle, kind: u32) -> Check<u32> {
        // Zero is legitimate for a non-GUI process. Clear/check the error so
        // an unavailable native observation is not silently reported as zero.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-getguiresources
        unsafe { SetLastError(0) };
        let count = unsafe { GetGuiResources(process, kind) };
        if count == 0 && unsafe { GetLastError() } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(count)
    }
    fn metrics(process: Handle) -> Check<Value> {
        require(alive(process)?, "cannot sample an exited native process")?;
        let mut handles = 0;
        if unsafe { GetProcessHandleCount(process, &mut handles) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(json!({"handles":handles,"user_objects":gui_count(process, 1)?,"gdi_objects":gui_count(process, 0)?}))
    }
    fn same_worker(computer: &Computer, pid: u32, generation: &str, sessions: usize) -> Check<()> {
        let status = computer.status();
        require(status.running && status.process_id == Some(pid)
            && status.generation.as_deref() == Some(generation)
            && status.bound_sessions == sessions && status.queued == 0,
            "worker generation, process or session accounting changed")
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
                if started.elapsed() > Duration::from_secs(10) { return Err("exact fixture did not close normally".into()); }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            let receipt = read(&self.root.join("exit.json"))?;
            require(exit.success() && !alive(self.child.as_raw_handle())?
                && receipt["pid"] == self.child.id() && receipt["graceful"] == true && receipt["control_file"] == true,
                "fixture normal-exit receipt did not match the exact Child")?;
            Ok(json!({"pid":self.child.id(),"exact_child_exited":true,"exit_code":exit.code(),"graceful_receipt":true}))
        }
    }

    pub async fn run() -> Check<()> {
        let args = std::env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>();
        require(matches!(args.len(), 2 | 3), "pass the absolute worker and fixture paths, with optional positive measurement rounds")?;
        let measurement_rounds = args.get(2).map(|value| {
            value.to_str().ok_or("measurement rounds must be a positive decimal integer")?
                .parse::<usize>().map_err(|_| "measurement rounds must be a positive decimal integer")
        }).transpose()?.unwrap_or(DEFAULT_MEASUREMENT_ROUNDS);
        require(measurement_rounds > 0, "measurement rounds must be positive")?;
        let total_rounds = WARMUPS.checked_add(measurement_rounds).ok_or("measurement rounds overflow")?;
        let (worker, fixture_exe) = (&args[0], &args[1]);
        require(worker.is_absolute() && worker.is_file() && fixture_exe.is_absolute() && fixture_exe.is_file(),
            "both explicitly supplied binaries must be absolute existing files")?;
        let worker_sha = file_hash(worker)?;
        let fixture_sha = file_hash(fixture_exe)?;
        let run = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("nomifun-computer-repeat-{run}"));
        std::fs::create_dir(&root)?;
        println!("OWNED_SESSION_REPEAT_ARTIFACTS {}", root.display());
        let mut report = json!({"schema":"nomifun.computer-owned-session-repeat.v1","ok":false,
            "scope":"one Computer host and real worker; AX-only native session repetition; not complete Agent Runtime, real-model or long-term profile evidence",
            "resource_growth_verdict":"not asserted; inspect the raw measurement sequence",
            "root":root,"host_pid":std::process::id(),"host_sha256":file_hash(&std::env::current_exe()?)?,
            "source_sha256":digest_bytes(include_bytes!("owned_session_repeat_smoke.rs")).as_ref(),
            "worker_path":worker,"worker_sha256":worker_sha,"fixture_path":fixture_exe,"fixture_sha256":fixture_sha,
            "warmup_rounds":WARMUPS,"measurement_rounds":measurement_rounds,"idle_timeout_ms":IDLE.as_millis(),
            "settle_ms":SETTLE.as_millis(),"agent_label":"Owned native session repeat",
            "screenshots":0,"input_calls":0,"forced_stops":0,"rounds":[]});
        write(&root, "repeat-report.json", &report)?;
        let child = Command::new(fixture_exe).arg("functional").arg(&root)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        let mut fixture = Fixture { root: root.clone(), child };
        let mut config = WorkerConfig::new(worker, root.join("host-data"));
        // This fixture's owner only; no application preference is changed.
        config.idle_timeout = IDLE;
        let computer = Computer::new(config);
        let authorized = BTreeSet::from(["computer/get_window_state".to_owned()]);
        let mut witness: Option<OwnedHandle> = None;
        let mut worker_pid = None;
        let mut generation: Option<String> = None;
        let mut active_session: Option<String> = None;
        let mut rounds = Vec::<Value>::new();
        let outcome: Check<()> = async {
            wait_file(&root.join("ready.json")).await?;
            let ready = read(&root.join("ready.json"))?;
            let pid = fixture.child.id();
            let window = ready["window"].as_str().ok_or("fixture HWND missing")?.parse::<u64>()?;
            let fixture_identity = process_identity(fixture.child.as_raw_handle(), fixture_exe)?;
            require(ready["pid"] == pid && ready["creation_filetime"] == fixture_identity["creation_filetime"],
                "ready receipt did not identify the retained exact fixture Child")?;
            owned_window(&fixture.child, window)?;
            report["fixture_identity"] = fixture_identity;
            report["window_id"] = json!(window);
            report["dpi"] = ready["dpi"].clone();
            for round in 0..total_rounds {
                let session = format!("owned-repeat-{run}-{round}");
                active_session = Some(session.clone());
                owned_window(&fixture.child, window)?;
                let call_started = Instant::now();
                let reply = computer.call(&session, "Owned native session repeat", "computer/get_window_state",
                    json!({"pid":pid,"window_id":window,"include_screenshot":false,"tree_format":"elements","max_elements":100,"timeout_ms":3000}),
                    &authorized, false, InvocationControl::default()).await?;
                require(reply.completion == ActionCompletion::Completed && !reply.foreground,
                    "AX request did not complete as a background observation")?;
                require(!reply.result["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "image")),
                    "AX-only observation returned raw image content")?;
                let normalized = result::normalize(&reply.result, false);
                require(!normalized.is_error, "owned-window observation was refused")?;
                let state = normalized.structured_content.ok_or("native structured snapshot missing")?;
                let elements = state["elements"].as_array().ok_or("native AX elements missing")?;
                require(state["pid"] == pid && state["window_id"] == window
                    && state["snapshot_id"].as_str().is_some_and(|id| !id.is_empty())
                    && elements.iter().any(|element| element["label"] == "Owned calculation" && element["value"] == "0")
                    && elements.iter().any(|element| element["label"] == "Owned editor")
                    && !state["screenshot_width"].as_u64().is_some_and(|size| size > 0),
                    "snapshot did not prove the exact untouched owned window")?;
                let call_ms = call_started.elapsed().as_millis();
                let status = computer.status();
                if witness.is_none() {
                    let pid = status.process_id.ok_or("worker PID missing")?;
                    // Open only once, before warmup sampling. Do not manufacture
                    // apparent host handle growth by opening an observer per round.
                    let handle = unsafe { OpenProcess(0x0010_0000 | 0x1000, 0, pid) };
                    if handle.is_null() { return Err(io::Error::last_os_error().into()); }
                    let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
                    let identity = process_identity(owned.as_raw_handle(), worker)?;
                    require(identity["pid"] == pid, "worker native handle identified another PID")?;
                    report["worker_identity"] = identity;
                    worker_pid = Some(pid);
                    generation = Some(reply.generation.clone());
                    report["generation"] = json!(generation);
                    witness = Some(owned);
                }
                let pid = worker_pid.ok_or("retained worker PID missing")?;
                let generation = generation.as_deref().ok_or("retained worker generation missing")?;
                require(reply.generation == generation, "a later request used another generation")?;
                same_worker(&computer, pid, generation, 1)?;
                computer.close_session(&session).await?;
                active_session = None;
                tokio::time::sleep(SETTLE).await;
                same_worker(&computer, pid, generation, 0)?;
                owned_window(&fixture.child, window)?;
                let worker_handle = witness.as_ref().ok_or("worker native handle missing")?.as_raw_handle();
                rounds.push(json!({"round":round+1,"phase":if round<WARMUPS {"warmup"} else {"measurement"},
                    "session_id":session,"pid":pid,"generation":generation,"close_session_succeeded":true,
                    "bound_sessions_after_close":computer.status().bound_sessions,"call_elapsed_ms":call_ms,
                    "snapshot_id":state["snapshot_id"],"ax_element_count":elements.len(),
                    "host":metrics(unsafe { GetCurrentProcess() })?,"worker":metrics(worker_handle)?}));
                report["rounds"] = json!(rounds);
                write(&root, "repeat-report.json", &report)?;
            }
            let handle = witness.as_ref().ok_or("worker native handle missing")?.as_raw_handle();
            let idle_started = Instant::now();
            // No shutdown request before this observation. Both native process
            // exit and the retained Computer owner's stopped state are required.
            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let status = computer.status();
                    if !alive(handle)? && !status.running && status.phase == "stopped" && status.bound_sessions == 0 {
                        return Ok::<(), Box<dyn std::error::Error>>(());
                    }
                    if status.running {
                        require(status.process_id == worker_pid && status.generation == generation,
                            "idle observation encountered another worker generation")?;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await??;
            let mut exit_code = 0;
            if unsafe { GetExitCodeProcess(handle, &mut exit_code) } == 0 { return Err(io::Error::last_os_error().into()); }
            report["idle"] = json!({"explicit_shutdown_requested":false,"exact_worker_exited":true,
                "worker_exit_code":exit_code,"owner_status":computer.status(),"observed_after_last_sample_ms":idle_started.elapsed().as_millis(),
                "host_with_same_observer_handles":metrics(unsafe { GetCurrentProcess() })?});
            require(exit_code == 0, "worker did not exit normally during ordinary idle retirement")?;
            Ok(())
        }.await;
        if let Some(session) = active_session { let _ = computer.close_session(&session).await; }
        let shutdown = computer.shutdown().await;
        let closed = fixture.close().await;
        report["rounds"] = json!(rounds);
        report["cleanup"] = json!({"normal_shutdown_succeeded":shutdown.is_ok(),"status":computer.status(),
            "fixture":closed.as_ref().ok(),"fixture_close_error":closed.as_ref().err().map(ToString::to_string)});
        report["failure"] = json!(outcome.as_ref().err().map(ToString::to_string));
        let passed = outcome.is_ok() && rounds.len() == total_rounds && shutdown.is_ok()
            && !computer.status().running && closed.is_ok();
        report["ok"] = json!(passed);
        write(&root, "repeat-report.json", &report)?;
        require(passed, "native session repetition or normal cleanup failed; inspect repeat-report.json")?;
        println!("OWNED_SESSION_REPEAT_OBSERVED {}", root.join("repeat-report.json").display());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    { native::run().await }
    #[cfg(not(windows))]
    { Err("this native observation fixture is Windows-only".into()) }
}
