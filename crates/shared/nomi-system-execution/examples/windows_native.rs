//! Native UAC/IPC acceptance, separate from the default Agent product driver.
//! Run explicitly on a Windows desktop; approve only the OS UAC dialog.
use std::{io::{self, Write}, time::{Duration, Instant}};
use serde_json::{Value, json};
use nomi_system_execution::SystemExecutionClient;

type Failure = Box<dyn std::error::Error>;

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if let Some(code) = nomi_system_execution::helper_entry() { return code; }
    let parent_exit = std::env::args().skip(1).any(|arg| arg == "--parent-exit-probe");
    let file_symlinks = std::env::args().skip(1).any(|arg| arg == "--file-symlink-fixture");
    if parent_exit && file_symlinks {
        eprintln!("Select only one explicit native fixture mode");
        return std::process::ExitCode::FAILURE;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    match runtime.block_on(async { if file_symlinks { file_symlink_fixture().await } else { run(parent_exit).await } }) {
        Ok(report) => {
            let passed = report["status"] == "passed";
            if let Err(error) = emit(&report) { eprintln!("Native acceptance receipt write failed: {error}"); return std::process::ExitCode::FAILURE; }
            if passed { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
        }
        Err(error) => { eprintln!("{error}"); std::process::ExitCode::FAILURE }
    }
}

#[cfg(windows)]
fn fixture_file_identity(path: &std::path::Path) -> Result<Value, Failure> {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx};
    let file = nomi_native_files::windows::open_metadata(None, path)?;
    let mut identity: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    require(unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
        (&mut identity as *mut FILE_ID_INFO).cast(), std::mem::size_of_val(&identity) as u32) } != 0,
        format!("Cannot read native fixture file identity: {}", io::Error::last_os_error()))?;
    let metadata = file.metadata()?;
    Ok(json!({"volume_serial":identity.VolumeSerialNumber.to_string(),
        "file_id":identity.FileId.Identifier.iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
        "bytes":metadata.len(),"attributes":metadata.file_attributes()}))
}

#[cfg(windows)]
async fn file_symlink_fixture() -> Result<Value, Failure> {
    use std::{fs, path::{Component, Path, PathBuf, Prefix}};
    let connected = tokio::task::spawn_blocking(SystemExecutionClient::connect).await?;
    let client = match connected {
        Ok(client) => client,
        Err(error) => {
            let cleanup = tokio::task::spawn_blocking(SystemExecutionClient::unregister).await?;
            return Ok(json!({"schema":"nomifun.windows-file-symlink-fixture.v1","status":"failed","canonical_data_involved":false,
                "connection_error":{"code":error.code,"message":error.message,"details":error.details},
                "facility_exit_proven":cleanup.is_ok(),"facility_error":cleanup.err().map(|error|error.to_string()),"fixtures_retained":true}));
        }
    };
    let mut cases = Vec::new();
    let mut retained_roots = Vec::new();
    let mut identity = Value::Null;
    let mut process_receipt = Value::Null;
    let mut cross_volume = json!({"requested":"D:\\","available":false});
    let result: Result<(), Failure> = async {
        identity = client.call(json!({"op":"identity"})).await?;
        require(identity["windows"]["elevated"] == true, "Native fixture runner did not prove elevation")?;
        let c_base = fs::canonicalize(std::env::temp_dir())?;
        require(matches!(c_base.components().next(),Some(Component::Prefix(prefix))
            if matches!(prefix.kind(),Prefix::Disk(letter)|Prefix::VerbatimDisk(letter) if letter.eq_ignore_ascii_case(&b'C'))),
            "This explicit probe requires the original user's existing C: temporary directory")?;
        let d_base = match fs::metadata(r"D:\") {
            Ok(metadata) => { require(metadata.is_dir(), "D: exists but is not a directory")?; Some(fs::canonicalize(r"D:\")?) },
            Err(error) if error.kind() == io::ErrorKind::NotFound => { cross_volume["unavailable_reason"] = json!({"kind":format!("{:?}",error.kind()),"raw_os_error":error.raw_os_error()}); None },
            Err(error) => return Err(error.into()),
        };
        cross_volume["available"] = json!(d_base.is_some());
        for cross in [false, true] {
            if cross && d_base.is_none() { continue; }
            for role in ["target", "source"] {
                let nonce = uuid::Uuid::now_v7().to_string();
                let root = tempfile::Builder::new().prefix("nomifun-native-replace-probe-").tempdir_in(&c_base)?.keep();
                retained_roots.push(root.clone());
                fs::write(root.join(".nomifun-native-replace-probe"), b"file-symlink-replace-race-v1")?;
                let external_root = if cross {
                    let root = tempfile::Builder::new().prefix("nomifun-native-replace-probe-").tempdir_in(d_base.as_ref().unwrap())?.keep();
                    retained_roots.push(root.clone());
                    fs::write(root.join(".nomifun-native-replace-probe"), b"file-symlink-replace-race-v1")?;
                    root
                } else { root.clone() };
                let sentinel = external_root.join("sentinel.txt");
                let content = format!("native-file-symlink-sentinel:{nonce}:{role}:{cross}");
                fs::write(&sentinel, content.as_bytes())?;
                let ads = PathBuf::from(format!("{}:probe-metadata", sentinel.display()));
                let ads_content = format!("native-file-symlink-ads:{nonce}"); fs::write(&ads, ads_content.as_bytes())?;
                let link = root.join("link.template");
                require(fs::symlink_metadata(&link).is_err_and(|error|error.kind()==io::ErrorKind::NotFound), "The unique fixture link already exists")?;
                let root_identity = fixture_file_identity(&root)?;
                let sentinel_identity = fixture_file_identity(&sentinel)?;
                require((root_identity["volume_serial"] != sentinel_identity["volume_serial"]) == cross,
                    "The actual native volumes do not match the selected same/cross-volume fixture")?;
                cases.push(json!({"id":format!("{}-{role}",if cross {"cross-volume"}else{"same-volume"}),"role":role,"cross_volume":cross,
                    "root":root,"external_root":external_root,"fixture_link":link,"sentinel":sentinel,"sentinel_bytes":content,
                    "sentinel_ads_path":ads,"sentinel_ads_bytes":ads_content,"root_identity":root_identity,"sentinel_identity_before":sentinel_identity,
                    "environment":{"NOMIFUN_REPLACEFILE_SYMLINK_FIXTURE":link,"NOMIFUN_REPLACEFILE_SYMLINK_ROLE":role}}));
            }
        }
        let inputs = serde_json::to_string(&cases)?.replace('\'', "''");
        let script = format!(r#"$ErrorActionPreference='Stop'; Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NomiNativeFileSymlinkFixture {{
  [DllImport("kernel32.dll", EntryPoint="CreateSymbolicLinkW", CharSet=CharSet.Unicode, SetLastError=true)]
  [return: MarshalAs(UnmanagedType.I1)] public static extern bool CreateFileLink(string link, string target, uint flags);
}}
'@;
$fixtures=ConvertFrom-Json -InputObject '{inputs}';
$observed=@(); foreach($fixture in $fixtures) {{
  $root=[string]$fixture.root; $link=[string]$fixture.fixture_link; $target=[string]$fixture.sentinel;
  if([IO.File]::ReadAllText([IO.Path]::Combine($root,'.nomifun-native-replace-probe')) -cne 'file-symlink-replace-race-v1') {{throw 'Exact fixture marker differs'}};
  if([IO.File]::ReadAllText($target) -cne [string]$fixture.sentinel_bytes) {{throw 'Exact sentinel differs'}};
  if(-not[NomiNativeFileSymlinkFixture]::CreateFileLink($link,$target,0)) {{throw (New-Object ComponentModel.Win32Exception([Runtime.InteropServices.Marshal]::GetLastWin32Error()))}};
  $entry=Get-Item -LiteralPath $link -Force -ErrorAction Stop;
  if($entry.PSIsContainer -or $entry.LinkType -cne 'SymbolicLink') {{throw 'The native fixture must be a FILE symbolic link'}};
  $observed += [pscustomobject]@{{id=$fixture.id;link=$link;target=$target;link_type=$entry.LinkType;is_directory=$entry.PSIsContainer;root_owner_sid=([IO.Directory]::GetAccessControl($root)).GetOwner([Security.Principal.SecurityIdentifier]).Value}};
}};
$user=[Security.Principal.WindowsIdentity]::GetCurrent(); $principal=New-Object Security.Principal.WindowsPrincipal($user);
[pscustomobject]@{{pid=$PID;creation_filetime=(Get-Process -Id $PID).StartTime.ToUniversalTime().ToFileTimeUtc().ToString();sid=$user.User.Value;elevated=$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);session_id=(Get-Process -Id $PID).SessionId;fixtures=$observed}} | ConvertTo-Json -Depth 5 -Compress"#);
        let process = Session::start(&client, &c_base, &script, 30000, false).await?;
        let terminal = process.terminal(&client, Duration::from_secs(40)).await?;
        process_receipt = json!({"started":process.started,"terminal":terminal});
        require(terminal["state"]=="exited" && terminal["code"]==0 && terminal["cleanup"]["reaped"]==true,
            format!("The original native fixture process did not complete/reap: {terminal}"))?;
        let observation:Value = serde_json::from_str(text(&terminal).trim())?;
        require(observation["elevated"]==true,"The fixture command did not prove its actual administrator token")?;
        process_receipt["observation"] = observation;
        for case in &mut cases {
            let link = Path::new(case["fixture_link"].as_str().unwrap()); let sentinel = Path::new(case["sentinel"].as_str().unwrap());
            require(fs::symlink_metadata(link)?.file_type().is_symlink(), "The returned fixture is not an actual native symbolic link")?;
            require(fs::read_link(link)?.is_absolute(), "The fixture symlink target is not absolute")?;
            require(fs::canonicalize(link)?==fs::canonicalize(sentinel)?, "The native link resolves to a different sentinel")?;
            require(fs::metadata(link)?.is_file(), "The symbolic link resolves to a directory")?;
            require(fs::read(sentinel)?==case["sentinel_bytes"].as_str().unwrap().as_bytes(), "The sentinel changed during fixture creation")?;
            require(fs::read(case["sentinel_ads_path"].as_str().unwrap())?==case["sentinel_ads_bytes"].as_str().unwrap().as_bytes(), "The sentinel ADS changed during fixture creation")?;
            let after = fixture_file_identity(sentinel)?;
            require(after==case["sentinel_identity_before"], "The sentinel native identity/metadata changed during fixture creation")?;
            case["sentinel_identity_after"] = after; case["status"] = json!("ready");
        }
        Ok(())
    }.await;
    let shutdown = client.call(json!({"op":"shutdown"})).await;
    let facility = tokio::task::spawn_blocking(SystemExecutionClient::unregister).await?;
    let passed = result.is_ok() && shutdown.as_ref().is_ok_and(|value|value["exact"]==true) && facility.is_ok();
    Ok(json!({"schema":"nomifun.windows-file-symlink-fixture.v1","status":if passed{"passed"}else{"failed"},"canonical_data_involved":false,
        "identity":identity,"cross_volume":cross_volume,"cases":cases,"retained_roots":retained_roots,"parent_pid":std::process::id(),"process":process_receipt,"failure":result.err().map(|error|error.to_string()),
        "shutdown":shutdown.as_ref().ok(),"shutdown_error":shutdown.err().map(|error|error.to_string()),"facility_exit_proven":facility.is_ok(),
        "facility_error":facility.err().map(|error|error.to_string()),"fixtures_retained":true,"test":"service::tests::replacement_external_interference_retains_uncertain_outcome --ignored --nocapture"}))
}
#[cfg(not(windows))]
async fn file_symlink_fixture() -> Result<Value, Failure> { Err(io::Error::other("File-symlink fixtures require native Windows").into()) }
fn emit(value: &Value) -> io::Result<()> {
    let mut output = io::stdout().lock();
    output.write_all(serde_json::to_string(value)?.as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()
}
fn require(condition: bool, message: impl Into<String>) -> Result<(), Failure> {
    if condition { Ok(()) } else { Err(io::Error::other(message.into()).into()) }
}
fn text(value: &Value) -> String {
    value["output"]["chunks"].as_array().into_iter().flatten()
        .filter_map(|chunk| chunk["text"].as_str()).collect::<Vec<_>>().concat()
}
#[cfg(windows)]
fn native_process_identity(pid: u32) -> Result<Value, Failure> {
    use windows_sys::Win32::{Foundation::{CloseHandle,FILETIME},System::Threading::{OpenProcess,GetProcessTimes,PROCESS_QUERY_LIMITED_INFORMATION}};
    let handle=unsafe {OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,0,pid)};
    require(!handle.is_null(),format!("Cannot independently open exact fixture PID {pid}: {}",io::Error::last_os_error()))?;
    let (mut creation,mut exit,mut kernel,mut user):(FILETIME,FILETIME,FILETIME,FILETIME)=unsafe {std::mem::zeroed()};
    let success=unsafe {GetProcessTimes(handle,&mut creation,&mut exit,&mut kernel,&mut user)};
    let error=io::Error::last_os_error();unsafe {CloseHandle(handle);}
    require(success!=0,format!("Cannot read creation identity for fixture PID {pid}: {error}"))?;
    Ok(json!({"pid":pid,"creation_filetime":(((creation.dwHighDateTime as u64)<<32)|creation.dwLowDateTime as u64).to_string()}))
}
#[cfg(not(windows))]
fn native_process_identity(_pid: u32) -> Result<Value, Failure> {Err(io::Error::other("This native acceptance example requires Windows").into())}
struct Session { base: Value, started: Value }
impl Session {
    async fn start(client: &SystemExecutionClient, cwd: &std::path::Path, script: &str, timeout_ms: u64, pty: bool) -> Result<Self, Failure> {
        let base = json!({"invocation_id":uuid::Uuid::now_v7().to_string(),"call_id":uuid::Uuid::now_v7().to_string()});
        let mut start = base.clone(); start["op"] = json!("start"); start["cwd"] = json!(cwd);
        start["command"] = json!({"kind":"shell","script":format!("$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=New-Object Text.UTF8Encoding($false); {script}")});
        let profile = std::env::var("USERPROFILE")?;
        start["env"] = json!({"USERPROFILE":profile,"HOME":profile});
        start["timeout_ms"] = json!(timeout_ms);
        if pty { start["transport"] = json!({"kind":"pty","cols":80,"rows":24}); }
        let started = client.call(start).await?;
        require(started["pid"].as_u64().is_some_and(|pid| pid > 1) && started["session_id"].is_string(), "Native start omitted exact process identity")?;
        Ok(Self { base, started })
    }
    async fn call(&self, client: &SystemExecutionClient, op: &str, fields: Value) -> Result<Value, nomi_system_execution::SystemExecutionError> {
        let mut request = self.base.clone(); request["op"] = json!(op); request["session_id"] = self.started["session_id"].clone();
        for (key,value) in fields.as_object().expect("fixture RPC fields must be an object") { request[key] = value.clone(); }
        client.call(request).await
    }
    async fn terminal(&self, client: &SystemExecutionClient, timeout: Duration) -> Result<Value, Failure> {
        let until = Instant::now() + timeout;
        loop {
            let value = self.call(client,"terminal",json!({})).await?;
            if !value["terminal"].is_null() { return Ok(value["terminal"].clone()); }
            require(Instant::now() < until, "Native terminal observation deadline expired")?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    async fn output_until(&self, client: &SystemExecutionClient, marker: &str) -> Result<(String,Vec<Value>), Failure> {
        let mut output = String::new(); let mut polls = Vec::new(); let mut cursor = 0;
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let value = self.call(client,"poll",json!({"cursor":cursor,"yield_ms":1000})).await?;
            require(value["state"] == "running", format!("Expected native running process: {value}"))?;
            cursor = value["output"]["next_cursor"].as_u64().unwrap_or(cursor);
            output.push_str(&text(&value)); polls.push(value);
            if output.contains(marker) { return Ok((output,polls)); }
            require(Instant::now() < until, "Native process did not produce its fixture readiness marker")?;
        }
    }
}
async fn exercise(client: &SystemExecutionClient, root: &std::path::Path, cases: &mut Vec<Value>) -> Result<(), Failure> {
    let source = root.join("source.txt"); std::fs::write(&source,b"native-read-source\r\n")?;
    let read = client.acquire_read(nomi_native_files::NativeReadRequest::Source { path:source.to_str().unwrap().into(),max_bytes:8192 }).await?;
    cases.push(json!({"id":"readonly_complete_typed_receipt","receipt":read}));

    let waiting = Session::start(client,root,"[Console]::Out.WriteLine('native-runner-ready'); Start-Sleep -Seconds 90",120000,false).await?;
    let (_,running) = waiting.output_until(client,"native-runner-ready").await?;
    let stopped = waiting.call(client,"cancel",json!({})).await?;
    require(stopped["state"] == "cancelled" && stopped["cleanup"]["reaped"] == true, format!("Native cancellation/reap failed: {stopped}"))?;
    cases.push(json!({"id":"pipe_cancel","started":waiting.started,"running_polls":running,"terminal":stopped}));

    let pipe = Session::start(client,root,"[Console]::InputEncoding=New-Object Text.UTF8Encoding($false); $v=[Console]::In.ReadToEnd(); [Console]::Write('NOMIFUN_PIPE_INPUT:'); [Console]::Write($v)",30000,false).await?;
    let literal = "stdin 中文 'quotes' % ! & | < > ^ $\r\n";
    let write = pipe.call(client,"write",json!({"bytes":literal.as_bytes()})).await?;
    let close = pipe.call(client,"close_stdin",json!({})).await?;
    let natural = pipe.terminal(client,Duration::from_secs(20)).await?;
    require(natural["state"] == "exited" && natural["code"] == 0 && natural["cleanup"]["reaped"] == true, format!("Natural Pipe exit/reap failed: {natural}"))?;
    require(text(&natural) == format!("NOMIFUN_PIPE_INPUT:{literal}"), "Native Pipe stdin bytes or Unicode changed")?;
    let late_write = pipe.call(client,"write",json!({"bytes":b"must-not-write"})).await;
    let late_resize = pipe.call(client,"resize",json!({"cols":120,"rows":40})).await;
    require(late_write.is_err() && late_resize.is_err(), "Late stdin/resize must not revive a naturally exited process")?;
    let terminal_after = pipe.call(client,"terminal",json!({})).await?;
    require(terminal_after["terminal"] == natural, "Late operations changed the immutable actual terminal outcome")?;
    cases.push(json!({"id":"pipe_stdin_close_natural_exit_late_terminal","started":pipe.started,"write":write,"close_stdin":close,"terminal":natural,"late_write_error":late_write.err().map(|e|e.code),"late_resize_error":late_resize.err().map(|e|e.code),"terminal_after":terminal_after}));

    let deadline = Session::start(client,root,"[Console]::Out.WriteLine('NOMIFUN_DEADLINE_WAITING'); Start-Sleep -Seconds 90",1500,false).await?;
    let timed_out = deadline.terminal(client,Duration::from_secs(15)).await?;
    require(timed_out["state"] == "timed_out" && timed_out["cleanup"]["reaped"] == true, format!("Native deadline/reap failed: {timed_out}"))?;
    cases.push(json!({"id":"native_deadline","started":deadline.started,"terminal":timed_out}));

    let pty = Session::start(client,root,"[Console]::Out.WriteLine('NOMIFUN_PTY_READY'); $line=[Console]::ReadLine(); [Console]::WriteLine(('NOMIFUN_PTY_SIZE:{0}:{1}' -f [Console]::WindowWidth,[Console]::WindowHeight)); Start-Sleep -Seconds 90",120000,true).await?;
    let (_,ready_polls) = pty.output_until(client,"NOMIFUN_PTY_READY").await?;
    let resize = pty.call(client,"resize",json!({"cols":100,"rows":35})).await?;
    let write = pty.call(client,"write",json!({"bytes":b"size\r"})).await?;
    let (output,size_polls) = pty.output_until(client,"NOMIFUN_PTY_SIZE:100:35").await?;
    require(output.contains("NOMIFUN_PTY_SIZE:100:35"), "ConPTY child did not observe the actual resized dimensions")?;
    let cancelled = pty.call(client,"cancel",json!({})).await?;
    require(cancelled["state"] == "cancelled" && cancelled["cleanup"]["reaped"] == true, format!("ConPTY cancel/reap failed: {cancelled}"))?;
    cases.push(json!({"id":"conpty_resize_stdin_cancel","started":pty.started,"ready_polls":ready_polls,"resize":resize,"write":write,"size_polls":size_polls,"terminal":cancelled}));
    Ok(())
}
async fn parent_exit_probe(client: &SystemExecutionClient, identity: Value, root: tempfile::TempDir) -> Result<Value, Failure> {
    let power_shell = std::path::PathBuf::from(std::env::var("SystemRoot")?).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = format!("$leaf=Start-Process -FilePath '{}' -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 180') -PassThru -WindowStyle Hidden; [Console]::WriteLine(('NOMIFUN_DESCENDANT_PID:'+ $leaf.Id)); [Console]::WriteLine('NOMIFUN_PARENT_EXIT_READY'); Start-Sleep -Seconds 180",power_shell.display().to_string().replace('\'',"''"));
    let session = Session::start(client,root.path(),&script,240000,false).await?;
    let (output,polls) = session.output_until(client,"NOMIFUN_PARENT_EXIT_READY").await?;
    let descendant = output.lines().find_map(|line|line.trim().strip_prefix("NOMIFUN_DESCENDANT_PID:").and_then(|pid|pid.parse::<u32>().ok()))
        .ok_or_else(||io::Error::other("Parent-exit fixture omitted exact descendant PID"))?;
    let helper_pid=identity["windows"]["pid"].as_u64().and_then(|pid|u32::try_from(pid).ok()).ok_or_else(||io::Error::other("Native helper omitted PID"))?;
    let child_pid=session.started["pid"].as_u64().and_then(|pid|u32::try_from(pid).ok()).ok_or_else(||io::Error::other("Native child omitted PID"))?;
    let processes=[native_process_identity(helper_pid)?,native_process_identity(child_pid)?,native_process_identity(descendant)?];
    let report = json!({"schema":"nomifun.windows-system-parent-exit-probe.v1","status":"ready_for_parent_exit","canonical_data_involved":false,"parent_pid":std::process::id(),"parent_exit_code":23,"identity":identity,"started":session.started,"descendant_pids":[descendant],"exact_processes":processes,"running_polls":polls,"fixture":root.path(),"parent_cleanup_requested":false,"receipt_flushed":true});
    emit(&report)?;
    // Deliberately leave the runner and Job-owned descendants alive here. The
    // independent driver observes their exact PIDs; this is never a canonical
    // Effect settlement or permission to reconcile an App crash dataset.
    std::process::exit(23)
}
async fn run(parent_exit: bool) -> Result<Value, Failure> {
    let client = tokio::task::spawn_blocking(SystemExecutionClient::connect).await??;
    let identity = client.call(json!({"op":"identity"})).await?;
    require(identity["windows"]["elevated"] == true, format!("Native helper did not prove elevation: {identity}"))?;
    let root = tempfile::tempdir()?;
    if parent_exit { return parent_exit_probe(&client,identity,root).await; }
    let mut cases = Vec::new();
    let result = exercise(&client,root.path(),&mut cases).await;
    let shutdown = client.call(json!({"op":"shutdown"})).await;
    let facility = tokio::task::spawn_blocking(SystemExecutionClient::unregister).await?;
    let passed = result.is_ok() && shutdown.as_ref().is_ok_and(|value|value["exact"] == true) && facility.is_ok();
    Ok(json!({"schema":"nomifun.windows-system-native-acceptance.v2","status":if passed {"passed"} else {"failed"},"canonical_data_involved":false,"identity":identity,"cases":cases,"failure":result.err().map(|error|error.to_string()),"shutdown":shutdown.as_ref().ok(),"shutdown_error":shutdown.err().map(|error|error.to_string()),"facility_exit_proven":facility.is_ok(),"facility_error":facility.err().map(|error|error.to_string())}))
}
