//! Opt-in Windows Tauri/WebView2 acceptance against one disposable Desktop.
//! Pass explicit Desktop and Computer worker executables. No model, account,
//! provider, shared browser profile, desktop coordinate, or forced stop.
//! Read-only diagnostics may save one owned-window image in the private root.

#[cfg(not(windows))]
fn main() {
    eprintln!("owned_tauri_smoke requires a real Windows Desktop");
    std::process::exit(2);
}

#[cfg(windows)]
mod windows {
    use nomifun_agent_contracts::digest_bytes;
    use nomifun_agent_kernel::InvocationControl;
    use nomifun_computer::{Computer, WorkerConfig, protocol::ActionCompletion, result};
    use serde_json::{Value, json};
    use std::{collections::BTreeSet, ffi::c_void, os::windows::{io::AsRawHandle, process::CommandExt},
        path::{Path, PathBuf}, process::{Child, Command, Stdio}, time::{Duration, Instant}};

    type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
    const DUMMY: &str = "Owned Tauri Computer acceptance";
    const SESSION: &str = "owned-tauri-acceptance";
    const THEME: &[&str] = &["主题", "Theme"];
    const LIGHT: &[&str] = &["浅色", "Light"];
    const DARK: &[&str] = &["深色", "Dark"];

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(isize, isize) -> i32, data: isize) -> i32;
        fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(window: isize) -> i32;
        fn GetDpiForWindow(window: isize) -> u32;
        fn GetWindowTextW(window: isize, buffer: *mut u16, capacity: i32) -> i32;
    }

    fn write(root: &Path, name: &str, value: &Value) -> Outcome<()> {
        std::fs::write(root.join(name), serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }

    fn file_hash(path: &Path) -> Outcome<String> {
        Ok(digest_bytes(&std::fs::read(path)?).as_ref().to_owned())
    }

    fn owner(window: isize) -> Option<u32> {
        let mut pid = 0;
        (unsafe { GetWindowThreadProcessId(window, &mut pid) } != 0).then_some(pid)
    }

    struct WindowQuery { pid: u32, windows: Vec<isize> }
    unsafe extern "system" fn owned_window(window: isize, data: isize) -> i32 {
        let query = unsafe { &mut *(data as *mut WindowQuery) };
        // Inspect no other application's title, content, or visibility.
        if owner(window) == Some(query.pid) && unsafe { IsWindowVisible(window) } != 0 {
            query.windows.push(window);
        }
        1
    }

    struct Desktop { root: PathBuf, token: String, child: Child }
    impl Desktop {
        fn live(&mut self) -> Outcome<()> {
            if self.child.try_wait()?.is_some() { return Err("owned Desktop exited before acceptance".into()); }
            Ok(())
        }
        fn target(&mut self, window: isize) -> Outcome<()> {
            self.live()?;
            if owner(window) != Some(self.child.id()) { return Err("owned HWND identity changed".into()); }
            Ok(())
        }
        async fn ready(&mut self) -> Outcome<Value> {
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                self.live()?;
                if let Ok(bytes) = std::fs::read(self.root.join("ready.json")) {
                    let ready: Value = serde_json::from_slice(&bytes)?;
                    if ready["token"] != self.token || ready["host_pid"] != self.child.id() || ready["mode"] != "quit" {
                        return Err("Desktop fixture readiness identity mismatch".into());
                    }
                    if ready["agent_resources"] != false { return Err("unexpected account/Agent fixture preparation".into()); }
                    return Ok(ready);
                }
                if Instant::now() >= deadline { return Err("owned Desktop readiness deadline exceeded".into()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        fn windows(&mut self) -> Outcome<Vec<isize>> {
            self.live()?;
            let mut query = WindowQuery {pid:self.child.id(),windows:Vec::new()};
            if unsafe { EnumWindows(owned_window, (&mut query as *mut WindowQuery) as isize) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(query.windows)
        }
        async fn shell_window(&mut self) -> Outcome<isize> {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let mut matches = Vec::new();
                for window in self.windows()? {
                    self.target(window)?;
                    let mut title = [0u16; 512];
                    let count = unsafe { GetWindowTextW(window,title.as_mut_ptr(),title.len() as i32) };
                    if count > 0 && String::from_utf16_lossy(&title[..count as usize]) == "NomiFun Lifecycle Fixture" {
                        matches.push(window);
                    }
                }
                match matches.as_slice() {
                    [window] => return Ok(*window),
                    [] if Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(100)).await,
                    [] => return Err("exact owned fixture shell title was not found".into()),
                    _ => return Err("exact owned fixture shell title is ambiguous".into()),
                }
            }
        }
        async fn quit(&mut self) -> Outcome<Value> {
            std::fs::write(self.root.join("quit.request"), &self.token)?;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                if let Some(status) = self.child.try_wait()? {
                    let terminal: Value = serde_json::from_slice(&std::fs::read(self.root.join("terminal.json"))?)?;
                    if !status.success() || terminal["token"] != self.token || terminal["pass"] != true
                        || terminal["quit_requested"] != true || terminal["coordinator_cleanup_verified"] != true {
                        return Err("owned Desktop normal quit was not proven".into());
                    }
                    return Ok(terminal);
                }
                if Instant::now() >= deadline { return Err("owned Desktop cleanup remains pending; no forced stop attempted".into()); }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    impl Drop for Desktop {
        fn drop(&mut self) { let _ = std::fs::write(self.root.join("quit.request"), &self.token); }
    }

    fn element<'a>(state: &'a Value, role: &str, labels: &[&str], value: Option<&str>) -> Option<&'a Value> {
        let matches = state["elements"].as_array()?.iter().filter(|entry| {
            entry["in_web_content"] == true
                && entry["role"].as_str().is_some_and(|actual| actual.eq_ignore_ascii_case(role))
                && (entry["label"].as_str().is_some_and(|label| labels.contains(&label))
                    || value.is_some_and(|value| entry["value"] == value))
                && entry["element_token"].is_string()
        }).collect::<Vec<_>>();
        (matches.len() == 1).then(|| matches[0])
    }

    async fn call(computer: &Computer, desktop: &mut Desktop, window: isize, name: &str,
        arguments: Value, calls: &mut Vec<Value>, image_enabled: bool) -> Outcome<Value> {
        desktop.target(window)?;
        let expected_pid = desktop.child.id();
        let authorized: BTreeSet<_> = ["computer/get_window_state", "computer/type_text", "computer/click"]
            .into_iter().map(str::to_owned).collect();
        let reply = computer.call(SESSION, "Owned Tauri acceptance", name, arguments, &authorized,
            image_enabled, InvocationControl::default()).await?;
        let normalized = result::normalize(&reply.result, image_enabled);
        let image_parts = reply.result["content"].as_array().map_or(0,|parts|
            parts.iter().filter(|part|part["type"]=="image").count());
        calls.push(json!({"action":name,"completion":reply.completion,"effect":normalized.effect,
            "is_error":normalized.is_error,"foreground":reply.foreground,"generation":reply.generation,
            "target_pid":desktop.child.id(),"target_window_id":window as u64,"image_parts":image_parts}));
        if !image_enabled && reply.result["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "image")) {
            return Err("unexpected image part in AX-only acceptance".into());
        }
        if name == "computer/get_window_state" {
            if reply.completion == ActionCompletion::Completed && !normalized.is_error {
                let state = normalized.structured_content.as_ref().ok_or("owned AX structured response missing")?;
                if state["pid"].as_u64() != Some(u64::from(expected_pid)) || state["window_id"].as_u64() != Some(window as u64) {
                    return Err("AX response identity differs from the exact owned Child/HWND; no token accepted".into());
                }
            }
            let bytes = serde_json::to_vec_pretty(&reply.result)?;
            if bytes.len() > 2 * 1024 * 1024 { return Err("owned AX evidence exceeded its byte bound".into()); }
            std::fs::write(desktop.root.join(format!("owned-ax-{:02}.json",calls.len())),bytes)?;
        }
        if reply.completion != ActionCompletion::Completed || normalized.is_error {
            return Err(format!("owned native call failed: {name}; no input is replayed").into());
        }
        Ok(normalized.structured_content.unwrap_or(reply.result))
    }

    async fn snapshot(computer: &Computer, desktop: &mut Desktop, window: isize, calls: &mut Vec<Value>) -> Outcome<Value> {
        let pid = desktop.child.id();
        call(computer, desktop, window, "computer/get_window_state", json!({"pid":pid,"window_id":window as u64,
            "include_screenshot":false,"tree_format":"both","max_elements":200,"max_depth":25,"timeout_ms":3000}), calls, false).await
    }

    fn has_action(entry: &Value, action: &str) -> bool {
        entry["actions"].as_array().is_some_and(|actions|actions.iter().any(|value|value==action))
    }

    fn guid_edit<'a>(state: &'a Value, expected: Option<&str>) -> Option<&'a Value> {
        let entries=state["elements"].as_array()?;
        if !entries.iter().any(|entry|entry["role"]=="Document" &&
            entry["value"].as_str().is_some_and(|value|value.ends_with("#/guid"))) { return None; }
        let matches=entries.iter().filter(|entry|entry["in_web_content"]==true && entry["role"]=="Edit"
            && has_action(entry,"set_value") && has_action(entry,"text") && entry["element_token"].is_string()
            && (entry["label"].as_str().is_some_and(|label|label.contains("发消息") || label.to_ascii_lowercase().contains("send a message"))
                || expected.is_some_and(|value|entry["value"]==value))).collect::<Vec<_>>();
        (matches.len()==1).then(||matches[0])
    }

    fn theme_radios(state: &Value) -> Option<(&Value,&Value)> {
        let light=element(state,"RadioButton",LIGHT,None)?;
        let dark=element(state,"RadioButton",DARK,None)?;
        let parent=light.get("parent_index")?;
        (dark.get("parent_index")==Some(parent)).then_some((light,dark))
    }

    async fn observe(computer: &Computer, desktop: &mut Desktop, window: isize, calls: &mut Vec<Value>,
        expected_text: Option<&str>, require_radios: bool) -> Outcome<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(state) = snapshot(computer, desktop, window, calls).await {
                let editor=guid_edit(&state,expected_text);
                let value_matches=expected_text.is_none_or(|text|editor.is_some_and(|entry|entry["value"].as_str()==Some(text)));
                let theme=element(&state,"Button",THEME,None).is_some_and(|entry|has_action(entry,"expand"));
                if editor.is_some() && value_matches && theme && (!require_radios || theme_radios(&state).is_some()) { return Ok(state); }
            }
            if Instant::now() >= deadline { return Err("owned Guid AX control/value postcondition unavailable; no input replay".into()); }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    pub async fn run() -> Outcome<()> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        let observe_only = args.get(2).is_some_and(|value| value == "--observe-only");
        if !(args.len() == 2 || (args.len() == 3 && observe_only)) {
            return Err("pass Desktop executable, Computer worker executable, and optional --observe-only".into());
        }
        let desktop_exe = std::fs::canonicalize(&args[0])?;
        let worker_exe = std::fs::canonicalize(&args[1])?;
        let example_exe = std::env::current_exe()?;
        let token = uuid::Uuid::now_v7().to_string();
        let root = std::env::temp_dir().join(format!("nomifun-lifecycle-{token}"));
        std::fs::create_dir(&root)?;
        for name in ["data", "work"] { std::fs::create_dir(root.join(name))?; }
        std::fs::write(root.join("fixture-sentinel"), &token)?;
        let mut report = json!({"ok":false,"evidence_kind":"real_windows_tauri_webview2_native_controls",
            "model_used":false,"account_created":false,"provider_configured":false,"input_replayed":false,
            "screenshots":0,"pixel_calls":0,"fixture_mode":"quit","artifact_root":root,
            "observe_only":observe_only,"controls_executed":false,
            "desktop_executable":desktop_exe,"desktop_sha256":file_hash(&desktop_exe)?,
            "external_test_worker_executable":worker_exe,"external_test_worker_sha256":file_hash(&worker_exe)?,
            "source_sha256":digest_bytes(include_bytes!("owned_tauri_smoke.rs")).as_ref(),
            "example_executable":example_exe,"example_executable_sha256":file_hash(&example_exe)?});
        let mut command = Command::new(&desktop_exe);
        command.current_dir(&root).creation_flags(0x08000000).stdin(Stdio::null())
            .stdout(std::fs::File::create(root.join("desktop-stdout.log"))?)
            .stderr(std::fs::File::create(root.join("desktop-stderr.log"))?)
            .env("NOMIFUN_DATA_DIR", root.join("data")).env("NOMIFUN_WORK_DIR", root.join("work"))
            .env("NOMIFUN_LIFECYCLE_FIXTURE_DIR", &root).env("NOMIFUN_LIFECYCLE_FIXTURE_TOKEN", &token)
            .env("NOMIFUN_LIFECYCLE_FIXTURE_MODE", "quit").env("NOMI_LOG_LEVEL", "warn");
        for name in ["WEBVIEW2_USER_DATA_FOLDER","WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS","WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
            "WEBVIEW2_CHANNEL_SEARCH_KIND","WEBVIEW2_RELEASE_CHANNELS","WEBVIEW2_RELEASE_CHANNEL_PREFERENCE",
            "NOMIFUN_RELIABILITY_DROP_TURN_ACK_KEY","NOMIFUN_ADMIN_PASSWORD","NOMIFUN_ADMIN_USERNAME","JWT_SECRET","NOMIFUN_JWT_SECRET"] {
            command.env_remove(name);
        }
        let mut desktop = Desktop {root:root.clone(), token, child:command.spawn()?};
        let handle: *mut c_void = desktop.child.as_raw_handle();
        if handle.is_null() { return Err("owned Desktop process handle missing".into()); }
        report["desktop_pid"] = json!(desktop.child.id());
        report["desktop_exact_child_handle_retained"] = json!(true);
        let computer = Computer::new(WorkerConfig::new(&worker_exe, root.join("external-computer-data")));
        let mut calls = Vec::new();
        let outcome = async {
            let ready = desktop.ready().await?;
            report["desktop_fixture_ready"] = ready;
            if observe_only {
                report["evidence_kind"] = json!("real_windows_tauri_webview2_window_observation");
                let window = desktop.shell_window().await?;
                report["owned_window_id"] = json!(window as u64);
                report["owned_window_dpi"] = json!(unsafe { GetDpiForWindow(window) });
                report["target_selection"] = json!("exact fixture title within exact owned Desktop PID");
                let deadline = Instant::now() + Duration::from_secs(15);
                let mut observations = Vec::new();
                let mut state;
                loop {
                    let pid = desktop.child.id();
                    state = call(&computer,&mut desktop,window,"computer/get_window_state",json!({"pid":pid,
                        "window_id":window as u64,"include_screenshot":false,"tree_format":"both",
                        "max_elements":200,"max_depth":25,"timeout_ms":3000}),&mut calls,false).await?;
                    let web_elements = state["elements"].as_array().map_or(0,|elements|elements.iter()
                        .filter(|element|element["in_web_content"]==true).count());
                    observations.push(json!({"call":calls.len(),"element_count":state["element_count"],
                        "web_content_elements":web_elements,"truncated":state["truncated"]}));
                    if Instant::now() >= deadline { break; }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                report["bounded_observations"] = json!(observations);
                let web_content_known = state["elements"].as_array().is_some_and(|elements|
                    elements.iter().any(|element|element["in_web_content"]==true));
                if !web_content_known {
                    report["screenshot_probe_requested"] = json!(true);
                    let pid = desktop.child.id();
                    state = call(&computer,&mut desktop,window,"computer/get_window_state",json!({"pid":pid,
                        "window_id":window as u64,"include_screenshot":true,"tree_format":"both",
                        "max_elements":200,"max_depth":25,"timeout_ms":3000}),&mut calls,true).await?;
                    report["screenshots"] = calls.last().map(|call|call["image_parts"].clone()).unwrap_or(json!(0));
                    report["private_screenshot_response"] = json!(format!("owned-ax-{:02}.json",calls.len()));
                }
                report["observed_state_keys"] = json!(state.as_object().map(|object|object.keys().collect::<Vec<_>>()));
                report["observed_element_count"] = json!(state["elements"].as_array().map(Vec::len));
                report["tauri_window_observed"] = json!(true);
                return Ok::<(),Box<dyn std::error::Error>>(());
            }
            let window = desktop.shell_window().await?;
            report["target_selection"] = json!("exact owned fixture shell title, then current Guid Document and unique web Edit");
            report["owned_window_id"] = json!(window as u64);
            report["owned_window_dpi"] = json!(unsafe { GetDpiForWindow(window) });
            let initial = observe(&computer,&mut desktop,window,&mut calls,None,false).await?;
            report["tauri_window_observed"] = json!(true);
            if theme_radios(&initial).is_some() { return Err("appearance controls were already present before the click".into()); }
            let editor=guid_edit(&initial,None).ok_or("owned Guid editor missing")?;
            let token=editor["element_token"].as_str().ok_or("owned Guid editor token missing")?;
            report["text_input_attempted"] = json!(true);
            call(&computer,&mut desktop,window,"computer/type_text",json!({"element_token":token,"text":DUMMY,"delivery_mode":"background"}),&mut calls,false).await?;
            let typed=observe(&computer,&mut desktop,window,&mut calls,Some(DUMMY),false).await?;
            report["guid_editor_value_verified"] = json!(true);
            let token=element(&typed,"Button",THEME,None).filter(|entry|has_action(entry,"expand"))
                .and_then(|entry|entry["element_token"].as_str()).ok_or("owned appearance button token missing")?;
            report["theme_expand_attempted"] = json!(true);
            call(&computer,&mut desktop,window,"computer/click",json!({"element_token":token,"action":"expand","delivery_mode":"background"}),&mut calls,false).await?;
            let expanded=observe(&computer,&mut desktop,window,&mut calls,Some(DUMMY),true).await?;
            let (light,dark)=theme_radios(&expanded).ok_or("appearance radio group missing")?;
            report["guid_editor_value_after_expand"] = guid_edit(&expanded,Some(DUMMY)).ok_or("owned editor missing after expand")?["value"].clone();
            report["theme_radio_labels"] = json!([light["label"],dark["label"]]);
            report["theme_popover_verified"] = json!(true);
            report["message_send_actions"] = json!(0);
            report["theme_selection_actions"] = json!(0);
            report["controls_executed"] = json!(true);
            Ok::<(),Box<dyn std::error::Error>>(())
        }.await;
        report["external_test_worker_pid"] = json!(computer.status().process_id);
        report["external_test_worker_version"] = json!(computer.status().worker_version);
        let session_closed = computer.close_session(SESSION).await;
        let worker_stopped = computer.shutdown().await;
        report["calls"] = json!(calls);
        let quit = desktop.quit().await;
        report["cleanup"] = json!({"computer_session_closed":session_closed.is_ok(),
            "external_test_computer_shutdown":worker_stopped.is_ok(),"external_test_worker_running":computer.status().running,
            "desktop_exact_child_exited_normally":quit.is_ok(),"forced_stop":false});
        if let Ok(terminal) = &quit { report["desktop_terminal"] = terminal.clone(); }
        let passed = outcome.is_ok() && session_closed.is_ok() && worker_stopped.is_ok()
            && !computer.status().running && quit.is_ok();
        report["ok"] = json!(passed);
        if let Err(error) = outcome { report["failure"] = json!(error.to_string()); }
        if let Err(error) = quit { report["cleanup_failure"] = json!(error.to_string()); }
        write(&root,"owned-tauri-report.json",&report)?;
        println!("{}",json!({"ok":passed,"report":root.join("owned-tauri-report.json")}));
        if passed { Ok(()) } else { Err("owned Tauri acceptance did not pass; see its private report".into()) }
    }
}

#[cfg(windows)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> { windows::run().await }
