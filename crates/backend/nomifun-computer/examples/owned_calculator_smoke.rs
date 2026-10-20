//! Windows native acceptance for one newly activated packaged Calculator.
//! Usage: owned_calculator_smoke <absolute worker> <absolute CalculatorApp.exe> <installed AUMID> [--observe-only]
//! Resolve the last two arguments from the installed package manifest beforehand.
//! Default: fresh-token background invokes of the observed Chinese C/2/+/3/= controls.
//! Observe-only dispatches no input. Neither mode uses images, a model, settings or forced stop.

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
        time::{Duration, Instant},
    };

    type Check<T> = Result<T, Box<dyn std::error::Error>>;
    type Handle = *mut c_void;
    const SESSION: &str = "owned-calculator-native";
    #[repr(C)]
    #[derive(Default)]
    struct FileTime { low: u32, high: u32 }
    #[repr(C)]
    struct ProcessEntry {
        size: u32, usage: u32, pid: u32, heap: usize, module: u32,
        threads: u32, parent: u32, priority: i32, flags: u32, executable: [u16; 260],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(process: Handle, flags: u32, path: *mut u16, length: *mut u32) -> i32;
        fn GetProcessTimes(process: Handle, created: *mut FileTime, exited: *mut FileTime,
            kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GetSystemTimeAsFileTime(time: *mut FileTime);
        fn WaitForSingleObject(process: Handle, milliseconds: u32) -> u32;
        fn GetLastError() -> u32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(isize, isize) -> i32, data: isize) -> i32;
        fn EnumChildWindows(window: isize, callback: unsafe extern "system" fn(isize, isize) -> i32, data: isize) -> i32;
        fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
        fn GetClassNameW(window: isize, name: *mut u16, capacity: i32) -> i32;
        fn IsWindow(window: isize) -> i32;
        fn GetDpiForWindow(window: isize) -> u32;
        fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    fn require(condition: bool, reason: &'static str) -> Check<()> {
        if condition { Ok(()) } else { Err(reason.into()) }
    }
    fn write(root: &Path, name: &str, value: &Value) -> Check<()> {
        std::fs::write(root.join(name), serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
    fn file_hash(path: &Path) -> Check<String> {
        Ok(digest_bytes(&std::fs::read(path)?).as_ref().to_owned())
    }
    fn ticks(time: &FileTime) -> u64 { (u64::from(time.high) << 32) | u64::from(time.low) }
    fn now() -> u64 {
        let mut value = FileTime::default();
        unsafe { GetSystemTimeAsFileTime(&mut value) };
        ticks(&value)
    }
    fn alive(process: Handle) -> Check<bool> {
        match unsafe { WaitForSingleObject(process, 0) } {
            0 => Ok(false), 258 => Ok(true), _ => Err(io::Error::last_os_error().into()),
        }
    }
    struct Witness { pid: u32, handle: OwnedHandle, identity: Value, created: u64 }
    impl Witness {
        fn open(pid: u32, expected_image: Option<&Path>) -> Check<Self> {
            let raw = unsafe { OpenProcess(0x0010_0000 | 0x1000, 0, pid) };
            if raw.is_null() { return Err(io::Error::last_os_error().into()); }
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            let mut path = vec![0u16; 32768];
            let mut length = path.len() as u32;
            let (mut created, mut exited, mut kernel, mut user) = (
                FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
            if unsafe { QueryFullProcessImageNameW(raw, 0, path.as_mut_ptr(), &mut length) } == 0
                || unsafe { GetProcessTimes(raw, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            let image = PathBuf::from(String::from_utf16(&path[..length as usize])?).canonicalize()?;
            if let Some(expected) = expected_image {
                require(image.to_string_lossy().eq_ignore_ascii_case(&expected.canonicalize()?.to_string_lossy()),
                    "native process image differs from the installed Calculator executable")?;
            }
            require(alive(raw)?, "native process exited before its identity was retained")?;
            let created = ticks(&created);
            Ok(Self { pid, handle, created,
                identity: json!({"pid":pid,"image":image,"creation_filetime":created.to_string()}) })
        }
        fn alive(&self) -> Check<bool> { alive(self.handle.as_raw_handle()) }
    }

    // Snapshot only executable names/PIDs. No old application's window title,
    // UIA tree, command line, profile, documents or session state is read.
    fn process_preflight() -> Check<Vec<Witness>> {
        let raw = unsafe { CreateToolhelp32Snapshot(2, 0) };
        if raw == -1isize as Handle { return Err(io::Error::last_os_error().into()); }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
        entry.size = std::mem::size_of::<ProcessEntry>() as u32;
        let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
        let mut hosts = Vec::new();
        while found != 0 {
            let end = entry.executable.iter().position(|unit| *unit == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&entry.executable[..end]);
            require(!name.eq_ignore_ascii_case("CalculatorApp.exe"),
                "a CalculatorApp process already exists; no activation is permitted")?;
            if name.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
                hosts.push(Witness::open(entry.pid, None)?);
            }
            found = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
        }
        require(unsafe { GetLastError() } == 18, "native process inventory could not finish")?;
        Ok(hosts)
    }
    fn owner(window: isize) -> Option<u32> {
        let mut pid = 0;
        (unsafe { GetWindowThreadProcessId(window, &mut pid) } != 0).then_some(pid)
    }
    fn class(window: isize) -> Option<String> {
        let mut name = [0u16; 256];
        let count = unsafe { GetClassNameW(window, name.as_mut_ptr(), name.len() as i32) };
        (count > 0).then(|| String::from_utf16_lossy(&name[..count as usize]))
    }
    fn top_windows() -> Check<BTreeSet<isize>> {
        unsafe extern "system" fn collect(window: isize, data: isize) -> i32 {
            unsafe { &mut *(data as *mut BTreeSet<isize>) }.insert(window);
            1
        }
        let mut windows = BTreeSet::new();
        if unsafe { EnumWindows(collect, (&mut windows as *mut BTreeSet<isize>) as isize) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(windows)
    }
    fn core_owner(frame: isize, host_pid: u32) -> Option<u32> {
        if owner(frame) != Some(host_pid) || class(frame).as_deref() != Some("ApplicationFrameWindow") { return None; }
        struct Scan { host: u32, apps: BTreeSet<u32> }
        unsafe extern "system" fn collect(child: isize, data: isize) -> i32 {
            let scan = unsafe { &mut *(data as *mut Scan) };
            if class(child).as_deref() == Some("Windows.UI.Core.CoreWindow")
                && let Some(pid) = owner(child).filter(|pid| *pid != scan.host) {
                    scan.apps.insert(pid);
                }
            1
        }
        let mut scan = Scan { host: host_pid, apps: BTreeSet::new() };
        // Match the fixed vendor's frame -> hosted CoreWindow attribution,
        // but require one unambiguous app PID rather than taking the first.
        unsafe { EnumChildWindows(frame, collect, (&mut scan as *mut Scan) as isize) };
        (scan.apps.len() == 1).then(|| *scan.apps.first().unwrap())
    }
    struct Target { frame: isize, host: Witness, app: Witness }
    impl Target {
        fn verify(&self) -> Check<()> {
            require(self.host.alive()? && self.app.alive()?
                && core_owner(self.frame, self.host.pid) == Some(self.app.pid),
                "the exact frame no longer hosts the retained new Calculator process")
        }
        async fn close(&self) -> Check<Value> {
            self.verify()?;
            if unsafe { PostMessageW(self.frame, 0x0010, 0, 0) } == 0 { return Err(io::Error::last_os_error().into()); }
            let started = Instant::now();
            while unsafe { IsWindow(self.frame) } != 0 {
                if started.elapsed() > Duration::from_secs(15) {
                    return Err("owned Calculator frame did not close normally; no forced stop attempted".into());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(json!({"wm_close_sent_to_exact_frame":true,"frame_destroyed":true,
                "calculator_process_alive_after_window_close":self.app.alive()?,"afh_process_alive_after_window_close":self.host.alive()?}))
        }
    }
    async fn invoke(computer: &Computer, allowed: &BTreeSet<String>, name: &str,
        arguments: Value, calls: &mut Vec<Value>) -> Check<Value> {
        let index = calls.len();
        calls.push(json!({"sequence":index+1,"action":name,"arguments":arguments,"status":"pending"}));
        let response = computer.call(SESSION, "Owned Calculator acceptance", name, arguments,
            allowed, false, InvocationControl::default()).await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                calls[index]["status"] = json!("transport_error");
                calls[index]["error_kind"] = json!(error.kind);
                calls[index]["error_code"] = json!(error.code);
                return Err(error.into());
            }
        };
        let normalized = result::normalize(&response.result, false);
        calls[index]["status"] = json!("returned");
        calls[index]["completion"] = json!(response.completion);
        calls[index]["is_error"] = json!(normalized.is_error);
        calls[index]["effect"] = json!(normalized.effect);
        calls[index]["foreground"] = json!(response.foreground);
        calls[index]["generation"] = json!(response.generation);
        require(response.completion == ActionCompletion::Completed && !normalized.is_error && !response.foreground,
            "owned native call did not complete; no action is replayed")?;
        require(!response.result["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "image")),
            "AX-only acceptance unexpectedly received an image")?;
        normalized.structured_content.ok_or_else(|| "native structured response missing".into())
    }
    async fn snapshot(computer: &Computer, allowed: &BTreeSet<String>, target: &Target,
        calls: &mut Vec<Value>, root: &Path) -> Check<Value> {
        target.verify()?;
        let state = invoke(computer,allowed,"computer/get_window_state",json!({"pid":target.host.pid,"window_id":target.frame as u64,
            "include_screenshot":false,"tree_format":"both","max_elements":300,"max_depth":25,"timeout_ms":3000}),calls).await?;
        target.verify()?;
        require(state["pid"] == target.host.pid && state["window_id"] == target.frame as u64
            && state["snapshot_id"].as_str().is_some_and(|id| !id.is_empty())
            && state["elements"].as_array().is_some_and(|elements| !elements.is_empty()),
            "native AX response did not identify the exact new Calculator frame")?;
        write(root,&format!("owned-calculator-ax-{:02}.json",calls.len()),&state)?;
        Ok(state)
    }
    fn button<'a>(state: &'a Value, label: &str) -> Check<&'a Value> {
        let matches = state["elements"].as_array().ok_or("native AX elements missing")?.iter().filter(|entry|
            entry["role"] == "Button" && entry["label"] == label && entry["enabled"] != false
                && entry["actions"].as_array().is_some_and(|actions|actions.iter().any(|action|action == "invoke"))
                && entry["element_token"].is_string()).collect::<Vec<_>>();
        require(matches.len() == 1,"observed Chinese invoke Button is missing or ambiguous; no input dispatched")?;
        Ok(matches[0])
    }

    pub async fn run() -> Check<()> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        require(matches!(args.len(),3|4) && args.get(3).is_none_or(|arg|arg == "--observe-only"),
            "pass absolute worker, installed CalculatorApp.exe, its manifest AUMID and optional --observe-only")?;
        let observe_only = args.len() == 4;
        let worker = PathBuf::from(&args[0]);
        let calculator = PathBuf::from(&args[1]);
        let aumid = args[2].to_str().ok_or("AUMID must be text")?;
        require(worker.is_absolute() && worker.is_file() && calculator.is_absolute() && calculator.is_file()
            && calculator.file_name().is_some_and(|name|name.to_string_lossy().eq_ignore_ascii_case("CalculatorApp.exe"))
            && aumid.contains('!') && aumid.trim() == aumid,
            "explicit binary paths and installed AUMID are required")?;
        let root = std::env::temp_dir().join(format!("nomifun-calculator-native-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        println!("OWNED_CALCULATOR_ARTIFACTS {}",root.display());
        let mut report = json!({"schema":"nomifun.owned-calculator-native.v1","ok":false,"observe_only":observe_only,
            "scope":"one newly activated packaged Calculator; no Agent Runtime or real model",
            "root":root,"worker_path":worker,"worker_sha256":file_hash(&worker)?,
            "calculator_path":calculator,"calculator_sha256":file_hash(&calculator)?,"aumid":aumid,
            "example_executable_sha256":file_hash(&std::env::current_exe()?)?,
            "source_sha256":digest_bytes(include_bytes!("owned_calculator_smoke.rs")).as_ref(),
            "screenshots":0,"input_calls":0,"foreground_requests":0,"forced_stops":0,"activation_attempts":0,"input_replayed":false,"steps":[]});
        write(&root,"calculator-report.json",&report)?;
        let computer = Computer::new(WorkerConfig::new(&worker,root.join("host-data")));
        let mut calls = Vec::new();
        let mut steps = Vec::<Value>::new();
        let mut allowed = BTreeSet::from(["computer/launch_app".to_owned(),"computer/get_window_state".to_owned()]);
        if !observe_only { allowed.insert("computer/click".to_owned()); }
        let mut target: Option<Target> = None;
        let mut old_hosts: Vec<Witness> = Vec::new();
        let outcome: Check<()> = async {
            old_hosts = process_preflight()?;
            let existing_windows = top_windows()?;
            report["preflight"] = json!({"calculator_process_absent":true,"old_top_window_count":existing_windows.len(),
                "old_afh":old_hosts.iter().map(|host|host.identity.clone()).collect::<Vec<_>>()});
            let launched_after = now();
            report["activation_attempts"] = json!(1);
            let launch = invoke(&computer,&allowed,"computer/launch_app",json!({"aumid":aumid,"start_minimized":false}),&mut calls).await?;
            let host_pid = launch["pid"].as_u64().and_then(|pid|u32::try_from(pid).ok()).ok_or("launch returned no typed pid")?;
            let windows = launch["windows"].as_array().ok_or("launch returned no typed windows")?;
            report["launch"] = json!({"pid":host_pid,"bundle_id":launch["bundle_id"],
                "window_ids":windows.iter().map(|window|window["window_id"].clone()).collect::<Vec<_>>()});
            require(launch["bundle_id"] == aumid && windows.len() == 1,
                "launch did not return one window for the exact installed AUMID")?;
            let frame = windows[0]["window_id"].as_u64().ok_or("launch window HWND missing")? as isize;
            require(!existing_windows.contains(&frame), "activation reused an existing frame; no AX read permitted")?;
            let app_pid = core_owner(frame,host_pid).ok_or("new frame has no unique hosted CoreWindow owner; no AX read permitted")?;
            let app = Witness::open(app_pid,Some(&calculator))?;
            require(app.created >= launched_after, "activation reused a pre-existing Calculator process; no AX read permitted")?;
            let host = Witness::open(host_pid,None)?;
            require(host.identity["image"].as_str().is_some_and(|path|path.to_ascii_lowercase().ends_with("\\applicationframehost.exe")),
                "returned frame is not owned by ApplicationFrameHost")?;
            target = Some(Target {frame,host,app});
            let owned = target.as_ref().ok_or("owned target missing")?;
            owned.verify()?;
            report["target"] = json!({"frame_hwnd":frame as u64,"host":owned.host.identity,"calculator":owned.app.identity,
                "new_frame":true,"new_calculator_process":true,"dpi":unsafe {GetDpiForWindow(frame)}});
            let state = snapshot(&computer,&allowed,owned,&mut calls,&root).await?;
            write(&root,"owned-calculator-ax.json",&state)?;
            report["ax_element_count"] = json!(state["elements"].as_array().unwrap().len());
            report["owned_ax_observed"] = json!(true);
            if !observe_only {
                require(state["elements"].as_array().unwrap().iter().filter(|entry|
                    entry["role"] == "Text" && entry["label"] == "标准 计算器模式").count() == 1,
                    "the observed standard Calculator mode is not present; no settings are changed")?;
                for label in ["清除","二","加","三","等于"] { button(&state,label)?; }
                report["initial_standard_mode_verified"] = json!(true);
                let mut input_snapshots = BTreeSet::new();
                for label in ["清除","二","加","三","等于"] {
                    let fresh = snapshot(&computer,&allowed,owned,&mut calls,&root).await?;
                    let snapshot_id = fresh["snapshot_id"].as_str().ok_or("fresh snapshot ID missing")?;
                    require(input_snapshots.insert(snapshot_id.to_owned()),"input snapshot was reused; no input replay")?;
                    let token = button(&fresh,label)?["element_token"].as_str().ok_or("fresh input token missing")?;
                    require(token.split_once(':').is_some_and(|(source,_)|source == snapshot_id),
                        "input token belongs to another snapshot; no input dispatched")?;
                    owned.verify()?;
                    let index = steps.len();
                    steps.push(json!({"button_label":label,"snapshot_id":snapshot_id,"element_token":token,
                        "snapshot_call_sequence":calls.len(),"click_call_sequence":calls.len()+1,"completed":false}));
                    let receipt = invoke(&computer,&allowed,"computer/click",json!({"element_token":token,
                        "target":{"kind":"window","pid":host_pid,"window_id":frame as u64},
                        "action":"invoke","delivery_mode":"background"}),&mut calls).await?;
                    owned.verify()?;
                    require(receipt["route"] == "accessibility" && receipt.pointer("/delivery/mode") == Some(&json!("background")),
                        "click lacked a native background accessibility receipt; it is not repeated")?;
                    steps[index]["completed"] = json!(true);
                    steps[index]["native_receipt"] = receipt;
                    report["steps"] = json!(steps);
                    write(&root,"calculator-report.json",&report)?;
                }
                let final_state = snapshot(&computer,&allowed,owned,&mut calls,&root).await?;
                require(final_state["elements"].as_array().unwrap().iter().filter(|entry|
                    entry["role"] == "Text" && entry["label"] == "显示为 5").count() == 1,
                    "fresh Calculator AX did not uniquely report the observed result 5")?;
                write(&root,"owned-calculator-final-ax.json",&final_state)?;
                report["final_result"] = json!({"role":"Text","label":"显示为 5","derived_numeric_result":5,"result_source":"exact native AX Text label",
                    "snapshot_id":final_state["snapshot_id"],"ax_call_sequence":calls.len(),"verified":true});
                report["calculation_verified"] = json!(true);
            }
            Ok(())
        }.await;
        let closed = match &target {
            Some(owned) => owned.close().await,
            None => Err::<Value,Box<dyn std::error::Error>>("no proven owned frame; no close message sent".into()),
        };
        let session_closed = computer.close_session(SESSION).await;
        let stopped = computer.shutdown().await;
        let hosts = old_hosts.iter().map(|host| {
            host.alive().map(|alive|json!({"identity":host.identity,"alive_after_cleanup":alive}))
        }).collect::<Check<Vec<_>>>();
        let shared_preserved = hosts.as_ref().is_ok_and(|hosts|hosts.iter().all(|host|host["alive_after_cleanup"] == true));
        report["steps"] = json!(steps);
        report["input_calls"] = json!(calls.iter().filter(|call|call["action"] == "computer/click").count());
        report["click_success_count"] = json!(steps.iter().filter(|step|step["completed"] == true).count());
        report["calls"] = json!(calls);
        report["cleanup"] = json!({"frame_close":closed.as_ref().ok(),"frame_close_error":closed.as_ref().err().map(ToString::to_string),
            "old_afh":hosts.as_ref().ok(),"old_afh_observation_error":hosts.as_ref().err().map(ToString::to_string),
            "shared_afh_preserved":shared_preserved,"session_closed":session_closed.is_ok(),
            "computer_shutdown":stopped.is_ok(),"worker_running":computer.status().running});
        report["failure"] = json!(outcome.as_ref().err().map(ToString::to_string));
        let inputs_proven = if observe_only { report["input_calls"] == 0 }
            else { report["input_calls"] == 5 && report["click_success_count"] == 5 && report["calculation_verified"] == true };
        let passed = outcome.is_ok() && inputs_proven && closed.is_ok() && shared_preserved && session_closed.is_ok()
            && stopped.is_ok() && !computer.status().running;
        report["ok"] = json!(passed);
        write(&root,"calculator-report.json",&report)?;
        require(passed,"owned Calculator acceptance/cleanup did not pass; inspect calculator-report.json")?;
        println!("OWNED_CALCULATOR_OBSERVED {}",root.join("calculator-report.json").display());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(),Box<dyn std::error::Error>> {
    #[cfg(windows)]
    { native::run().await }
    #[cfg(not(windows))]
    { Err("this native observation requires Windows".into()) }
}
