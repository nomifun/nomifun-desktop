//! Opt-in real Linux GTK3/AT-SPI acceptance, without a model or fault injection.
//! owned_linux_smoke <absolute worker> <absolute python3> <absolute fixture.py>
//! DISPLAY must select the authorized existing X11/XWayland session.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("owned_linux_smoke requires a real Linux host");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
mod native {
    use nomifun_agent_contracts::digest_bytes;
    use nomifun_agent_kernel::InvocationControl;
    use nomifun_computer::{Computer, WorkerConfig, protocol::ActionCompletion, result, surface};
    use serde_json::{Value, json};
    use std::{collections::BTreeSet, io::{self, Write}, os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        path::{Path, PathBuf}, time::{Duration, Instant}};

    type Check<T> = Result<T, Box<dyn std::error::Error>>;
    const TEXT: &str = "NomiFun owned Linux GTK3 text.\n6 × 7 = 42; 中文保存。";
    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    }
    fn require(ok: bool, reason: &str) -> Check<()> {
        if ok { Ok(()) } else { Err(reason.to_owned().into()) }
    }
    fn hash(path: &Path) -> Check<String> {
        Ok(digest_bytes(&std::fs::read(path)?).as_ref().to_owned())
    }
    fn atomic(root: &Path, name: &str, bytes: &[u8]) -> Check<()> {
        let temporary = root.join(format!(".{name}-{}", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary, root.join(name))?;
        Ok(())
    }
    fn write(root: &Path, name: &str, value: &Value) -> Check<()> {
        atomic(root, name, &serde_json::to_vec_pretty(value)?)
    }
    fn read(path: &Path) -> Check<Value> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    async fn wait_json(root: &Path, name: &str) -> Check<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if root.join(name).exists() { return read(&root.join(name)); }
            if root.join("fixture-error.json").exists() {
                return Err("the owned GTK callback reported an error".into());
            }
            require(Instant::now() < deadline, &format!("owned {name} did not arrive"))?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    #[derive(Clone)]
    struct Process {
        pid: u32,
        started: u64,
        pgid: u32,
        session: u32,
        state: char,
        exe: PathBuf,
        uid: u32,
    }
    impl Process {
        fn read(pid: u32) -> Check<Self> {
            let root = PathBuf::from(format!("/proc/{pid}"));
            let raw = std::fs::read_to_string(root.join("stat"))?;
            let fields = raw.rsplit_once(')').ok_or("invalid process stat")?.1.split_whitespace().collect::<Vec<_>>();
            require(fields.len() > 19, "incomplete process stat")?;
            Ok(Self {pid, started: fields[19].parse()?, pgid: fields[2].parse()?,
                session: fields[3].parse()?, state: fields[0].chars().next().ok_or("process state missing")?,
                exe: std::fs::read_link(root.join("exe"))?, uid: std::fs::metadata(root)?.uid()})
        }
        fn same_alive(&self) -> bool {
            Self::read(self.pid).is_ok_and(|now| now.started == self.started && now.exe == self.exe
                && now.uid == self.uid && !matches!(now.state, 'Z' | 'X'))
        }
        fn evidence(&self) -> Value {
            json!({"pid":self.pid,"start_time":self.started,"pgid":self.pgid,
                "session":self.session,"exe":self.exe,"uid":self.uid})
        }
    }
    async fn wait_exit(owner: &Process) -> Check<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut status = 0;
            let pid = unsafe { waitpid(owner.pid.try_into()?, &mut status, 1) };
            if pid == owner.pid as i32 {
                return Ok(json!({"exact_pid":owner.pid,"raw_wait_status":status,
                    "exit_code":if status & 127 == 0 {Some((status >> 8) & 255)} else {None},
                    "signal":if status & 127 != 0 {Some(status & 127)} else {None},
                    "reaped":true}));
            }
            if pid < 0 && io::Error::last_os_error().raw_os_error() != Some(10) {
                return Err(io::Error::last_os_error().into());
            }
            require(Instant::now() < deadline, "exact fixture did not exit/reap normally; no signal sent")?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    struct Calls<'a> {
        computer: &'a Computer,
        allowed: BTreeSet<String>,
        session: &'a str,
        rows: Vec<Value>,
    }
    impl Calls<'_> {
        async fn invoke(&mut self, driver: &str, arguments: Value) -> Check<Value> {
            let selected = surface::tool(driver).ok_or("driver is absent from frozen catalog")?;
            let sequence = self.rows.len() + 1;
            let started = Instant::now();
            let call = self.computer.call(self.session, "Owned Linux GTK3", &selected.action_id,
                arguments.clone(), &self.allowed, false, InvocationControl::default()).await;
            let mut row = json!({"sequence":sequence,"action":selected.action_id,"arguments":arguments,
                "elapsed_ms":started.elapsed().as_millis(),"completed":false});
            let mut output = None;
            let error = match call {
                Ok(reply) => {
                    let normalized = result::normalize(&reply.result, false);
                    row["completion"] = json!(reply.completion);
                    row["foreground"] = json!(reply.foreground);
                    row["generation"] = json!(reply.generation);
                    let raw_images = reply.result["content"].as_array().map_or(0, |parts|
                        parts.iter().filter(|part|part["type"]=="image").count());
                    row["raw_image_parts"] = json!(raw_images);
                    if raw_images == 0 { row["raw_result"] = reply.result; }
                    row["normalized_effect"] = json!(normalized.effect);
                    row["is_error"] = json!(normalized.is_error);
                    let images = normalized.parts.iter().filter(|part|matches!(part,result::ResultPart::Image{..})).count();
                    if reply.completion != ActionCompletion::Completed || normalized.is_error || reply.foreground || images != 0 || raw_images != 0 {
                        Some("owned SDK call failed; no input is replayed".to_owned())
                    } else if let Some(value) = normalized.structured_content {
                        row["completed"] = json!(true);
                        output = Some(value);
                        None
                    } else {
                        Some("owned SDK call returned no structured native result".to_owned())
                    }
                }
                Err(error) => { row["sdk_error"] = json!(error); Some("owned SDK call returned an error".to_owned()) }
            };
            eprintln!("{}", json!({"call":sequence,"action":selected.action_id,"completed":row["completed"]}));
            self.rows.push(row);
            match error {Some(error)=>Err(error.into()),None=>output.ok_or_else(||"native output missing".into())}
        }

        async fn capture(&mut self, owner: &Process, xid: u64, python: &Path) -> Check<Value> {
            require(owner.same_alive(), "capture owner identity changed")?;
            let selected=surface::tool("get_window_state").ok_or("capture tool is not frozen")?;
            let sequence=self.rows.len()+1;
            let mut row=json!({"sequence":sequence,"action":selected.action_id,"target_pid":owner.pid,
                "target_xid":xid,"full_window_observation":true,"completed":false,"image_payload_persisted":false});
            let checked: Check<Value> = async {
                let reply=match self.computer.call(self.session,"Owned Linux GTK3",&selected.action_id,
                    json!({"pid":owner.pid,"window_id":xid,"include_screenshot":true,
                        "include_accessibility_tree":false,"display_only":true,"timeout_ms":3000}),
                    &self.allowed,true,InvocationControl::default()).await {
                    Ok(reply)=>reply,
                    Err(error)=>{
                        row["sdk_error_kind"]=json!(error.kind);
                        row["sdk_error_code"]=json!(error.code);
                        return Err("owned window capture SDK request failed".into());
                    }
                };
                row["completion"]=json!(reply.completion);
                row["foreground"]=json!(reply.foreground);
                let is_error=reply.result["isError"].as_bool().unwrap_or(false);
                row["is_error"]=json!(is_error);
                require(reply.completion==ActionCompletion::Completed && !is_error && !reply.foreground
                    && owner.same_alive(),"owned window capture did not complete normally")?;
                let structured=reply.result["structuredContent"].as_object().ok_or("capture metadata missing")?;
                require(structured.get("pid")==Some(&json!(owner.pid))
                    && structured.get("window_id")==Some(&json!(xid)),"capture metadata target differs")?;
                row["capture_id"]=structured.get("capture_id").cloned().unwrap_or(Value::Null);
                let images=reply.result["content"].as_array().ok_or("capture content missing")?
                    .iter().filter(|part|part["type"]=="image").collect::<Vec<_>>();
                require(images.len()==1,"expected one actual owned-window image")?;
                let mime=images[0]["mimeType"].as_str().ok_or("capture MIME missing")?;
                require(matches!(mime,"image/png"|"image/jpeg"),"unexpected owned image format")?;
                let data=images[0]["data"].as_str().ok_or("capture image data missing")?;
                // The existing GdkPixbuf codec decodes the actual bytes. This
                // helper imports no Gtk/Gdk, opens no display, and prints no pixels.
                const CHECK_IMAGE: &str = r#"import base64,hashlib,json,sys
import gi
gi.require_version("GdkPixbuf","2.0")
from gi.repository import GdkPixbuf
value=json.load(sys.stdin)
blob=base64.b64decode(value["data"],validate=True)
loader=GdkPixbuf.PixbufLoader.new()
loader.write(blob)
loader.close()
image=loader.get_pixbuf()
fmt=loader.get_format()
if image is None or fmt is None or value["mime"] not in fmt.get_mime_types():
    raise RuntimeError("owned capture codec or MIME mismatch")
print(json.dumps({"mime_type":value["mime"],"decoded_bytes":len(blob),
 "width":image.get_width(),"height":image.get_height(),
 "decoded_sha256":hashlib.sha256(blob).hexdigest()}))
"#;
                let mut decoder=std::process::Command::new(python).args(["-c",CHECK_IMAGE])
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped()).spawn()?;
                let sent=decoder.stdin.take().ok_or("image decoder stdin missing")?
                    .write_all(&serde_json::to_vec(&json!({"mime":mime,"data":data}))?);
                let decoded=decoder.wait_with_output()?;
                row["image_validator_exit"]=json!(decoded.status.code());
                row["image_validator_stderr_sha256"]=json!(digest_bytes(&decoded.stderr).as_ref());
                require(sent.is_ok() && decoded.status.success(),"actual owned image failed codec validation")?;
                let image:Value=serde_json::from_slice(&decoded.stdout)?;
                require(image["width"].as_u64().is_some_and(|v|v>0)
                    && image["height"].as_u64().is_some_and(|v|v>0)
                    && image["decoded_bytes"].as_u64().is_some_and(|v|v>0)
                    && image["mime_type"]==mime,"decoded image metadata invalid")?;
                require(structured.get("screenshot_width")==Some(&image["width"])
                    && structured.get("screenshot_height")==Some(&image["height"]),
                    "actual decoded dimensions differ from delivered window metadata")?;
                row["image"]=image.clone();
                Ok(image)
            }.await;
            row["completed"]=json!(checked.is_ok());
            if checked.is_err() {row["error_class"]=json!("OWNED_WINDOW_CAPTURE_FAILED");}
            self.rows.push(row);
            checked
        }

        async fn snapshot(&mut self, owner: &Process, xid: u64) -> Check<Value> {
            require(owner.same_alive(), "owned Python process identity changed")?;
            let value = self.invoke("get_window_state", json!({"pid":owner.pid,"window_id":xid,
                "include_screenshot":false,"tree_format":"elements","max_elements":100,"max_depth":20,"timeout_ms":3000})).await?;
            require(owner.same_alive() && value["pid"] == owner.pid && value["window_id"] == xid,
                "AX snapshot does not belong to the exact owned process/window")?;
            require(value["snapshot_id"].is_string(), "exact-window AX snapshot identity missing")?;
            Ok(value)
        }
        async fn ready_snapshot(&mut self, owner: &Process, xid: u64) -> Check<Value> {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(value) = self.snapshot(owner,xid).await {
                    if ["Owned left operand","Owned right operand","Multiply owned operands","Owned product","Owned editor","Save owned text"]
                        .iter().all(|name|element(&value,name).is_ok()) {return Ok(value);}
                }
                require(Instant::now() < deadline, "owned GTK AT-SPI controls did not become ready")?;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        async fn input(&mut self, owner: &Process, xid: u64, label: &str, text: Option<&str>) -> Check<()> {
            let state = self.snapshot(owner,xid).await?;
            let token = element(&state,label)?["element_token"].as_str().ok_or("fresh element token missing")?;
            require(token.split_once(':').is_some_and(|(id,_)|state["snapshot_id"] == id), "token belongs to another snapshot")?;
            let mut args = json!({"element_token":token,"target":{"kind":"window","pid":owner.pid,"window_id":xid},"delivery_mode":"background"});
            let driver = if let Some(text) = text {args["text"]=json!(text);"type_text"} else {"click"};
            let receipt = self.invoke(driver,args).await?;
            require(owner.same_alive(), "owned application exited during native input")?;
            require(receipt["route"] == "accessibility"
                && receipt.pointer("/delivery/mode")==Some(&json!("background")),
                "input did not return the actual background accessibility receipt; it is not replayed")?;
            Ok(())
        }
    }
    fn element<'a>(state: &'a Value, name: &str) -> Check<&'a Value> {
        let matches = state["elements"].as_array().ok_or("structured AX elements missing")?.iter()
            .filter(|item|item["label"] == name && item["element_token"].is_string()).collect::<Vec<_>>();
        require(matches.len()==1, "owned AX label is absent or ambiguous")?;
        Ok(matches[0])
    }

    pub async fn run() -> Check<()> {
        let args = std::env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>();
        require(args.len()==3 && args.iter().all(|p|p.is_absolute() && p.is_file()),
            "pass absolute worker, python3 and fixture.py files")?;
        let worker = std::fs::canonicalize(&args[0])?;
        let python = std::fs::canonicalize(&args[1])?;
        let fixture = std::fs::canonicalize(&args[2])?;
        require(hash(&fixture)? == digest_bytes(include_bytes!("fixtures/owned_linux_smoke.py")).as_ref(),
            "explicit fixture does not match this example's compiled fixture source")?;
        require(std::env::var_os("DISPLAY").is_some(), "an authorized existing X11 display is required")?;
        // Adopt only this test's descendants after their parent worker exits;
        // this permits exact waitpid proof without sending any signal.
        require(unsafe {prctl(36,1_usize,0_usize,0_usize,0_usize)} == 0, "cannot establish test child subreaper")?;
        let token = uuid::Uuid::new_v4().to_string();
        let root = std::env::temp_dir().join(format!("nomifun-owned-linux-{token}"));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        atomic(&root,"fixture-sentinel",token.as_bytes())?;
        // Only the twelve path fields in the host launch context, never the
        // general environment or provider credentials. Preserve Unix bytes.
        use std::os::unix::ffi::OsStrExt;
        let keys = ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA",
            "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME",
            "TMPDIR", "TMP", "TEMP", "XDG_DATA_DIRS"];
        let expected_paths = keys.into_iter().map(|key| {
            (key, std::env::var_os(key).map(|value| value.as_bytes().to_vec()))
        }).collect::<std::collections::BTreeMap<_, _>>();
        write(&root, "expected-user-paths.json", &json!({
            "environment": expected_paths, "cwd": std::env::current_dir()?.as_os_str().as_bytes()
        }))?;

        let mut report = json!({"schema":"nomifun.owned-linux-gtk3.v1","ok":false,
            "scope":"one real GTK3 fixture via Computer SDK; not a stock calculator/editor, model or canonical Runtime Stop",
            "source_sha256":digest_bytes(include_bytes!("owned_linux_smoke.rs")).as_ref(),
            "fixture_path":fixture,"fixture_sha256":hash(&fixture)?,"python_path":python,"python_sha256":hash(&python)?,
            "worker_path":worker,"worker_sha256":hash(&worker)?,
            "executable":std::env::current_exe()?,"executable_sha256":hash(&std::env::current_exe()?)?,
            "root":root,"public_text":TEXT,"verified_window_images":0,"window_image_requests":0,"foreground_requests":0,"input_replays":0,"harness_signals_sent":0});
        println!("OWNED_LINUX_ARTIFACTS {}",root.display());
        write(&root,"report.json",&report)?;
        let computer = Computer::new(WorkerConfig::new(&worker,root.join("host-data")));
        let mut calls = Calls {computer:&computer,session:&token,allowed:surface::manifest().tools.iter()
            .filter(|tool|tool.platforms.contains_key(surface::platform())).map(|tool|tool.action_id.clone()).collect(),rows:Vec::new()};
        let mut owner: Option<Process> = None;
        let mut ready = Value::Null;
        let mut xid = 0_u64;
        let outcome: Check<()> = async {
            let launch = calls.invoke("launch_app",json!({"launch_path":python,
                "additional_arguments":[fixture,root,token]})).await?;
            ready = wait_json(&root,"ready.json").await?;
            let pid = ready["pid"].as_u64().and_then(|v|u32::try_from(v).ok()).ok_or("ready PID missing")?;
            let process = Process::read(pid)?;
            require(ready["token"] == token && launch["pid"] == pid && process.exe == python
                && ready["start_time"] == process.started && ready["pgid"] == process.pgid
                && ready["session"] == process.session && process.uid == std::fs::metadata("/proc/self")?.uid(),
                "ready/launch do not identify the exact owned Python process")?;
            let command = std::fs::read(format!("/proc/{pid}/cmdline"))?;
            let command = command.split(|byte|*byte==0).collect::<Vec<_>>();
            require(command.get(1)==Some(&fixture.as_os_str().as_encoded_bytes())
                && command.get(2)==Some(&root.as_os_str().as_encoded_bytes())
                && command.get(3)==Some(&token.as_bytes()), "owned Python arguments differ")?;
            owner = Some(process.clone());
            xid = ready["window_id"].as_u64().filter(|id|*id>0).ok_or("owned XID missing")?;
            report["target"]=json!({"process":process.evidence(),"ready":ready});
            let windows = calls.invoke("list_windows",json!({"pid":pid})).await?;
            require(windows["windows"].as_array().is_some_and(|windows|windows.iter().filter(|window|
                window["window_id"]==xid && window["pid"]==pid && window["title"]==ready["marker"]).count()==1),
                "owned XID/title marker was not independently observed")?;
            calls.ready_snapshot(&process,xid).await?;
            calls.input(&process,xid,"Owned left operand",Some("6")).await?;
            calls.input(&process,xid,"Owned right operand",Some("7")).await?;
            calls.input(&process,xid,"Multiply owned operands",None).await?;
            let state = calls.snapshot(&process,xid).await?;
            require(element(&state,"Owned product")?["value"]=="42","fresh AX did not read actual product 42")?;
            let calculation = wait_json(&root,"calculation.json").await?;
            require(calculation["token"]==token && calculation["pid"]==pid && calculation["result"]=="42"
                && calculation["multiply_count"]==1,"native Multiply callback did not agree")?;
            report["calculation"]=calculation;
            report["calculation_ax_verified"]=json!(true);
            calls.input(&process,xid,"Owned editor",Some(TEXT)).await?;
            let state = calls.snapshot(&process,xid).await?;
            require(element(&state,"Owned editor")?["value"]==TEXT,"fresh AX did not read the actual public text")?;
            report["editor_ax_verified"]=json!(true);
            calls.input(&process,xid,"Save owned text",None).await?;
            let saved = wait_json(&root,"saved.json").await?;
            let bytes = std::fs::read(root.join("saved.txt"))?;
            require(bytes==TEXT.as_bytes() && saved["sha256"]==digest_bytes(&bytes).as_ref()
                && saved["token"]==token && saved["pid"]==pid && saved["save_count"]==1,
                "independent saved file/hash did not match the public text")?;
            report["saved_file"]=json!({"sha256":digest_bytes(&bytes).as_ref(),"bytes":bytes.len(),"callback":saved});
            report["text_file_verified"]=json!(true);
            report["window_image_requests"]=json!(1);
            report["window_capture"]=calls.capture(&process,xid,&python).await?;
            report["verified_window_images"]=json!(1);
            let worker_pid=computer.status().process_id.ok_or("actual worker PID missing")?;
            report["worker_process"]=Process::read(worker_pid)?.evidence();
            report["worker_status_before_close"]=json!(computer.status());
            Ok(())
        }.await;
        let closed = computer.close_session(&token).await;
        let app_after_close = owner.as_ref().is_some_and(Process::same_alive);
        let stopped = computer.shutdown().await;
        let app_after_shutdown = owner.as_ref().is_some_and(Process::same_alive);
        let health: Check<Value> = async {
            require(app_after_shutdown,"owned user application died during normal worker shutdown")?;
            let nonce=uuid::Uuid::new_v4().to_string();
            write(&root,"health.request",&json!({"token":token,"nonce":nonce}))?;
            let value=wait_json(&root,"health.json").await?;
            let process=owner.as_ref().ok_or("owned process missing")?;
            require(process.same_alive() && value["pid"]==process.pid && value["start_time"]==process.started
                && value["token"]==token && value["nonce"]==nonce && value["window_id"]==xid,
                "post-worker GTK health response identity differs")?;
            Ok(value)
        }.await;
        // A late-starting owned fixture also sees this token; no PID signal or
        // operation replay is used if launch/AX/normal shutdown failed.
        let close_request=atomic(&root,"close.request",token.as_bytes());
        let terminal = match &owner {Some(process)=>wait_exit(process).await,None=>Err("fixture identity was not established".into())};
        let exit_record=read(&root.join("exit.json")).ok();
        let normal_exit=terminal.as_ref().is_ok_and(|value|value["exit_code"]==0)
            && exit_record.as_ref().is_some_and(|value|value["token"]==token && value["close_requested"]==true
                && owner.as_ref().is_some_and(|process|value["pid"]==process.pid && value["start_time"]==process.started));
        let environment_ok=ready.pointer("/environment/cua_keys_present")==Some(&json!(false))
            && ready.pointer("/environment/worker_private_root_present")==Some(&json!(false))
            && ready.pointer("/environment/original_user_paths_match")==Some(&json!(true))
            && ready.pointer("/environment/original_cwd_match")==Some(&json!(true));
        report["functional_postconditions_passed"]=json!(outcome.is_ok());
        report["failure"]=json!(outcome.as_ref().err().map(ToString::to_string));
        report["calls"]=json!(calls.rows);
        report["environment_isolation_ok"]=json!(environment_ok);
        report["cleanup"]=json!({"session_closed":closed.is_ok(),"shutdown":stopped.is_ok(),
            "worker_running":computer.status().running,"app_alive_after_session_close":app_after_close,
            "app_alive_after_worker_shutdown":app_after_shutdown,"gtk_health":health.as_ref().ok(),
            "gtk_health_error":health.as_ref().err().map(ToString::to_string),"close_request_written":close_request.is_ok(),
            "exact_fixture_terminal":terminal.as_ref().ok(),"terminal_error":terminal.as_ref().err().map(ToString::to_string),
            "exit_record":exit_record,"normal_fixture_exit":normal_exit});
        let passed=outcome.is_ok() && closed.is_ok() && stopped.is_ok() && !computer.status().running
            && app_after_close && app_after_shutdown && health.is_ok() && close_request.is_ok() && normal_exit && environment_ok;
        report["ok"]=json!(passed);
        write(&root,"report.json",&report)?;
        println!("{}",json!({"ok":passed,"report":root.join("report.json")}));
        require(passed,"owned Linux acceptance failed; original facts retained; no retry/fault/forced cleanup")
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(),Box<dyn std::error::Error>> { native::run().await }
