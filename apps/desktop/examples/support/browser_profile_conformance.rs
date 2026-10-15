//! Disposable production-Profile witnesses across native close and app restart.
//! HTTP Cookie values and storage values are compared locally, never reported.
use nomifun_browser_platform::{
    runtime::*,
    workspace::{BrowserResourceService, BrowserWorkspace, managed_workspace_key},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};
use tauri::Manager;
#[path="browser_profile_process_witness.rs"]
mod process_witness;
#[path="browser_profile_constructor_conformance.rs"]
mod constructor_conformance;
#[path="browser_profile_raw_sdk_conformance.rs"]
mod raw_sdk_conformance;

const OWNER: &str = "profile-conformance-owner";
const SESSION: &str = "persisted-profile-conformance";
const OTHER_SESSION: &str = "isolated-profile-conformance";
const MARKER: &str = ".nomifun-profile-conformance-owned";
const PHASE_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_PHASE";
const ROOT_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_DATA_ROOT";
const TARGET_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_TARGET";
const OWNERSHIP_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_OWNERSHIP";
const EVIDENCE_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_EVIDENCE_DIR";
const PROCESS_DIAGNOSTIC_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_PROCESS_DIAGNOSTIC";
const KEEP_FAILURE_PROFILE_ENV: &str = "NOMIFUN_PROFILE_CONFORMANCE_KEEP_FAILURE_PROFILE";
const DELAY_DIAGNOSTIC_FLAG:&str="--diagnostic-delay-before-close";

struct Fixture {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<(String, bool)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "Profile fixture bind failed")?;
        let address = listener.local_addr().map_err(|_| "Profile fixture address failed")?;
        listener.set_nonblocking(true).map_err(|_| "Profile fixture listener failed")?;
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_requests = requests.clone();
        let cookie = uuid::Uuid::now_v7().to_string();
        let worker = std::thread::spawn(move || {
            let mut clients = Vec::new();
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let requests = worker_requests.clone();
                        let cookie = cookie.clone();
                        clients.push(std::thread::spawn(move || serve(stream, requests, &cookie)));
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock =>
                        std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
            for client in clients { let _ = client.join(); }
        });
        Ok(Self { address, requests, stop, worker: Some(worker) })
    }
    fn url(&self, path: &str) -> String { format!("http://{}{path}", self.address) }
    fn count(&self, path: &str) -> usize {
        self.requests.lock().unwrap().iter().filter(|(entry, _)| entry == path).count()
    }
    fn cookie_received(&self, path: &str) -> bool {
        self.requests.lock().unwrap().iter().any(|(entry, cookie)| entry == path && *cookie)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn serve(mut stream: TcpStream, requests: Arc<Mutex<Vec<(String, bool)>>>, cookie: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    while bytes.len() <= 16 * 1024 && !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(size) => bytes.extend_from_slice(&buffer[..size]),
        }
    }
    if bytes.len() > 16 * 1024 { return; }
    let request = String::from_utf8_lossy(&bytes);
    let mut first = request.lines().next().unwrap_or("").split_whitespace();
    if first.next() != Some("GET") { return; }
    let path = first.next().unwrap_or("/").split('?').next().unwrap_or("/");
    let cookie_header = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("cookie").then_some(value)
    });
    let cookie_named = cookie_header.is_some_and(|value| value.split(';')
        .any(|item|item.trim().starts_with("nomi_profile_probe=")));
    let cookie_matches = cookie_header.is_some_and(|value| value.split(';')
        .any(|item| item.trim() == format!("nomi_profile_probe={cookie}")));
    {
        let mut entries = requests.lock().unwrap();
        if entries.len() >= 128 { return; }
        entries.push((path.to_owned(), cookie_matches));
    }
    let (content_type, body, extra) = if path == "/profile-cookie-check" {
        ("application/json", json!({"cookie":cookie_matches,"cookie_header_present":cookie_header.is_some(),"cookie_named_present":cookie_named}).to_string(), String::new())
    } else if path.starts_with("/profile-") {
        let seed = path == "/profile-seed";
        let script = if seed { SEED } else { READ_STORAGE };
        let body = format!("<!doctype html><meta charset=utf-8><title>Profile conformance</title><p>Controlled profile fixture</p><script>window.profileConformanceResult=(async()=>{{{script}}})();window.profileConformanceResult.then(value=>window.profileConformanceSettled=value);</script>");
        let extra = if seed {
            format!("Set-Cookie: nomi_profile_probe={cookie}; Path=/; HttpOnly; SameSite=Lax; Max-Age=3600\r\n")
        } else { String::new() };
        ("text/html; charset=utf-8", body, extra)
    } else { ("text/plain", String::new(), String::new()) };
    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
}

const SEED: &str = r#"
localStorage.setItem('nomi_profile_probe','seeded');
await new Promise((resolve,reject)=>{const request=indexedDB.open('nomi-profile-conformance',1);request.onupgradeneeded=()=>request.result.createObjectStore('items');request.onerror=()=>reject(new Error('fixture seed failed'));request.onsuccess=()=>{const db=request.result;const tx=db.transaction('items','readwrite');tx.objectStore('items').put('seeded','probe');tx.oncomplete=()=>{db.close();resolve();};tx.onerror=()=>{db.close();reject(new Error('fixture seed transaction failed'));};};});
const response=await fetch('/profile-cookie-check',{credentials:'same-origin',cache:'no-store'});const witness=await response.json();
return {cookie:witness.cookie===true,cookie_header_present:witness.cookie_header_present===true,cookie_named_present:witness.cookie_named_present===true,local:localStorage.getItem('nomi_profile_probe')==='seeded',indexeddb:true,http_only_cookie:document.cookie===''};
"#;
const READ_STORAGE: &str = r#"
const exists=(await indexedDB.databases()).some(db=>db.name==='nomi-profile-conformance');
let stored=false;
if(exists)stored=await new Promise((resolve,reject)=>{const request=indexedDB.open('nomi-profile-conformance',1);request.onerror=()=>reject(new Error('fixture read failed'));request.onsuccess=()=>{const db=request.result;const tx=db.transaction('items','readonly');const get=tx.objectStore('items').get('probe');get.onsuccess=()=>{const valid=get.result==='seeded';tx.oncomplete=()=>{db.close();resolve(valid);};};get.onerror=()=>{db.close();reject(new Error('fixture read transaction failed'));};};});
const response=await fetch('/profile-cookie-check',{credentials:'same-origin',cache:'no-store'});const witness=await response.json();
return {cookie:witness.cookie===true,cookie_header_present:witness.cookie_header_present===true,cookie_named_present:witness.cookie_named_present===true,local:localStorage.getItem('nomi_profile_probe')==='seeded',indexeddb:stored,http_only_cookie:document.cookie===''};
"#;

async fn page_result(app: &tauri::AppHandle, workspace: &Arc<BrowserWorkspace>, url: &str) -> Result<(Value, BrowserRuntimeInfo), String> {
    workspace.user_command(BrowserTabCommand::Create { url: url.into() }).await
        .map_err(|error| format!("Profile fixture create: {}", error.code()))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let target = loop {
        let snapshot = workspace.snapshot().await.map_err(|error| error.code().to_owned())?;
        let runtime = snapshot.runtime.ok_or("Profile fixture runtime missing")?;
        if let Some(tab) = runtime.tabs.iter().find(|tab| tab.url == url && tab.lifecycle == BrowserTabLifecycle::Ready) {
            break tab.target.clone();
        }
        if Instant::now() >= deadline { return Err("Profile fixture page did not become ready".into()); }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let view = app.get_webview(&target.tab_id).ok_or("Profile fixture native view missing")?;
    let evaluated = super::windows::protocol_call(&view, "Runtime.evaluate", json!({
        "expression":"window.profileConformanceResult", "awaitPromise":true, "returnByValue":true,
    })).await.map_err(|_| "Profile fixture storage evaluation failed")?;
    if evaluated.get("exceptionDetails").is_some() { return Err("Profile fixture storage promise failed".into()); }
    let mut value = evaluated["result"]["value"].clone();
    let cookies = super::windows::protocol_call(&view,"Network.getCookies",json!({"urls":[url]}))
        .await.map_err(|_| "Profile fixture controlled Cookie inventory failed")?;
    let cookie = cookies["cookies"].as_array().and_then(|cookies|
        cookies.iter().find(|cookie|cookie["name"]=="nomi_profile_probe"));
    value["native_cookie_named_present"] = json!(cookie.is_some());
    value["native_cookie_session"] = json!(cookie.and_then(|cookie|cookie["session"].as_bool()));
    value["native_cookie_http_only"] = json!(cookie.and_then(|cookie|cookie["httpOnly"].as_bool()));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Profile fixture system clock unavailable")?.as_secs_f64();
    value["native_cookie_expiry_is_future"] = json!(cookie.and_then(|cookie|cookie["expires"].as_f64()).map(|expires|expires>now));
    value["cdp_cookie_partition_key_present"]=json!(cookie.map(|cookie|cookie.get("partitionKey").is_some()));
    value["cdp_cookie_partition_key_opaque"]=json!(cookie.and_then(|cookie|cookie["partitionKeyOpaque"].as_bool()));
    let sdk=process_witness::cookie_manager_witness(&view).await?;
    eprintln!("PROFILE_STORAGE_WITNESS {}",json!({
        "stage":url::Url::parse(url).ok().map(|url|url.path().to_owned()),
        "cookie":value["cookie"].as_bool(),"header_present":value["cookie_header_present"].as_bool(),
        "native_cookie_present":value["native_cookie_named_present"].as_bool(),
        "native_cookie_session":value["native_cookie_session"].as_bool(),
        "native_cookie_http_only":value["native_cookie_http_only"].as_bool(),
        "native_cookie_expiry_is_future":value["native_cookie_expiry_is_future"].as_bool(),
        "cdp_partition_key_present":value["cdp_cookie_partition_key_present"].as_bool(),
        "cdp_partition_key_opaque":value["cdp_cookie_partition_key_opaque"].as_bool(),
        "public_cookie_manager":sdk,
        "local":value["local"].as_bool(),"indexeddb":value["indexeddb"].as_bool(),
    }));
    let report = workspace.navigation_diagnostics(target).await.map_err(|error| error.code().to_owned())?;
    let runtime = report.runtime.ok_or("Profile fixture effective runtime missing")?;
    if runtime.profile_persistence != Some(BrowserProfilePersistence::Persistent)
        || runtime.profile_identity_token.as_ref().is_none_or(|token| token.len() != 64)
        || runtime.browser_version.is_none() || report.identity.is_none() {
        return Err("Profile fixture native environment metadata incomplete".into());
    }
    Ok((value, runtime))
}

fn expect_state(value: &Value, seeded: bool) -> Result<(), String> {
    if value["cookie"] != seeded || value["local"] != seeded || value["indexeddb"] != seeded
        || value["http_only_cookie"] != true {
        let observed = |name| value.get(name).and_then(Value::as_bool)
            .map(|value| value.to_string()).unwrap_or_else(|| "unavailable".into());
        return Err(format!("Profile storage expected seeded={seeded}; observed object={}, cookie={}, header={}, named={}, jar={}, local={}, indexeddb={}, http_only={}",
            value.is_object(), observed("cookie"), observed("cookie_header_present"),observed("cookie_named_present"),observed("native_cookie_named_present"), observed("local"), observed("indexeddb"), observed("http_only_cookie")));
    }
    Ok(())
}

fn owned_root(root: &Path, ownership: &str) -> Result<(), String> {
    if !root.is_absolute() || ownership.len() != 36
        || std::fs::read_to_string(root.join(MARKER)).ok().as_deref() != Some(ownership) {
        return Err("Profile fixture disposable ownership missing".into());
    }
    Ok(())
}

fn target_url() -> Result<url::Url, String> {
    let target = std::env::var(TARGET_ENV).map_err(|_| "Profile fixture target missing")?;
    let url = url::Url::parse(&target).map_err(|_| "Profile fixture target invalid")?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.port().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Profile fixture target is not controlled loopback".into());
    }
    Ok(url)
}

/// Clean child App phase, dispatched before any unrelated fixture controller.
pub(super) async fn child_phase(app: &tauri::AppHandle) -> Result<Value, String> {
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).ok_or("Profile fixture root missing")?);
    let ownership = std::env::var(OWNERSHIP_ENV).map_err(|_| "Profile fixture ownership missing")?;
    owned_root(&root, &ownership)?;
    let mode = std::env::var(PHASE_ENV).map_err(|_| "Profile fixture phase missing")?;
    let mut url = target_url()?;
    let canonical_root = std::fs::canonicalize(&root).map_err(|_| "Profile fixture canonical root unavailable")?;
    let store = BrowserProfileStore::new(&root).map_err(|error| error.code().to_owned())?;
    let service = BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())))
        .with_profile_store(store.clone());
    let key = managed_workspace_key(OWNER, SESSION).map_err(|error| error.code().to_owned())?;
    let profile = store.profile_for(&key, BrowserProfilePersistence::Persistent).map_err(|error| error.code().to_owned())?;
    let intended_udf=match &profile{BrowserProfile::Persistent(path)=>path.clone(),_=>return Err("Expected controlled Windows directory Profile".into())};
    if mode == "override" {
        // WebView2's override may ignore a not-yet-created requested folder.
        // Pin a real, distinct intended directory so this phase proves native
        // identity mismatch rather than an unavailable expected directory.
        let BrowserProfile::Persistent(expected) = &profile else { return Err("Expected directory Profile for override proof".into()); };
        if !expected.starts_with(&canonical_root) { return Err("Profile fixture directory escaped its disposable root".into()); }
        std::fs::create_dir_all(expected).map_err(|_| "Profile fixture intended directory creation failed")?;
    }
    let workspace = service.ensure_user(OWNER, SESSION, profile).await.map_err(|error| error.code().to_owned())?;
    let mut process_observer = None;
    let result = async {
        if mode=="constructor-raw-sdk-default"||mode=="constructor-raw-sdk-normalized-input" {
            url.set_path("/profile-seed");
            return raw_sdk_conformance::verify(app,&intended_udf,url.as_str(),mode=="constructor-raw-sdk-normalized-input").await;
        }
        if matches!(mode.as_str(),"constructor-legacy"|"constructor-external"|"constructor-retained-profile-manager") {
            url.set_path("/profile-seed");
            return constructor_conformance::verify(app,&intended_udf,url.as_str(),mode!="constructor-legacy",mode=="constructor-retained-profile-manager").await;
        }
        if matches!(mode.as_str(),"override"|"override-verbatim") {
            url.set_path(if mode=="override" {"/profile-override"}else{"/profile-override-verbatim"});
            let result = workspace.user_command(BrowserTabCommand::Create { url: url.to_string() }).await;
            if !matches!(result, Err(WorkspaceError::ProfileCleanupInvalid)) {
                return Err("Profile override did not fail with safe effective-directory rejection".into());
            }
            let rejected=workspace.snapshot().await.map_err(|error|error.code().to_owned())?;
            if rejected.runtime.as_ref().is_some_and(|runtime|!runtime.tabs.is_empty())
                || !app.webviews().is_empty() {
                return Err("Rejected Profile override retained or replaced a native page".into());
            }
            return Ok(json!({"effective_profile_override_rejected":true,"verbatim_matching_override_rejected":mode=="override-verbatim","no_reinitialization":true,"no_retained_or_replacement_native_page":true}));
        }
        if !matches!(mode.as_str(), "seed" | "restart" | "preserved") { return Err("Profile fixture phase invalid".into()); }
        let path = match mode.as_str() { "seed" => "/profile-seed", "restart" => "/profile-restart", _ => "/profile-preserved" };
        url.set_path(path);
        let (value, first) = page_result(app, &workspace, url.as_str()).await?;
        expect_state(&value, true).map_err(|error|format!("Initial {path}: {error}"))?;
        if first.environment_overrides_present != Some(mode == "preserved") {
            return Err("Native environment override presence did not match the child configuration".into());
        }
        if mode == "seed" {
            let current=workspace.snapshot().await.map_err(|error|error.code().to_owned())?;
            let tab=current.runtime.as_ref().and_then(|runtime|runtime.tabs.iter().find(|tab|tab.url==url.as_str())).ok_or("Profile seed native target missing")?;
            let view=app.get_webview(&tab.target.tab_id).ok_or("Profile seed native process view missing")?;
            let original_profile=process_witness::discriminate(&view).await?;
            eprintln!("PROFILE_DISCRIMINATOR_WITNESS {}",json!({"stage":"before_close_all","native":original_profile.snapshot(),"native_profile_path_is_within_intended_udf":original_profile.path_is_within(&intended_udf),"cookie_database_file_exists_within_owned_profile":original_profile.cookie_database_exists_within(&intended_udf)}));
            // Retaining Environment for an exit observer crosses the former
            // production lifetime and can change Cookie shutdown behavior.
            // This is an opt-in causal diagnostic, never an acceptance shortcut.
            if std::env::var(PROCESS_DIAGNOSTIC_ENV).ok().as_deref()==Some("1") {
                process_observer=Some(process_witness::Observer::install(&view).await?);
            }
            if std::env::args().any(|argument|argument==DELAY_DIAGNOSTIC_FLAG) {
                eprintln!("PROFILE_LIFETIME_WITNESS {}",json!({"stage":"controlled_before_close_hold","lifetime_intervention":true,"seconds":35,"cookie_database_file_exists_within_owned_profile":original_profile.cookie_database_exists_within(&intended_udf)}));
                tokio::time::sleep(Duration::from_secs(35)).await;
                eprintln!("PROFILE_LIFETIME_WITNESS {}",json!({"stage":"controlled_after_hold","lifetime_intervention":true,"cookie_database_file_exists_within_owned_profile":original_profile.cookie_database_exists_within(&intended_udf)}));
            }
            workspace.user_command(BrowserTabCommand::CloseAll { runtime_generation: workspace.runtime_generation() }).await
                .map_err(|error| error.code().to_owned())?;
            if let Some(observer)=&process_observer {
                eprintln!("PROFILE_PROCESS_WITNESS {}",json!({"stage":"close_all_return", "controlled_environment_lifetime_intervention":true,"native":observer.snapshot()}));
            }
            url.set_path("/profile-reopen");
            let (value, reopened) = page_result(app, &workspace, url.as_str()).await?;
            let current=workspace.snapshot().await.map_err(|error|error.code().to_owned())?;
            let tab=current.runtime.as_ref().and_then(|runtime|runtime.tabs.iter().find(|tab|tab.url==url.as_str())).ok_or("Profile reopened native target missing")?;
            let view=app.get_webview(&tab.target.tab_id).ok_or("Profile reopened native process view missing")?;
            let reopened_profile=process_witness::discriminate(&view).await?;
            eprintln!("PROFILE_DISCRIMINATOR_WITNESS {}",json!({"stage":"reopened_page_ready","native":original_profile.compare(&reopened_profile),"effective_profile_token_equal_within_process":first.profile_identity_token==reopened.profile_identity_token,"native_profile_path_is_within_intended_udf":reopened_profile.path_is_within(&intended_udf),"cookie_database_file_exists_within_owned_profile":reopened_profile.cookie_database_exists_within(&intended_udf)}));
            if let Some(observer)=&process_observer {
                eprintln!("PROFILE_PROCESS_WITNESS {}",json!({"stage":"reopened_page_ready", "controlled_environment_lifetime_intervention":true,"native":observer.compare_reopened(&view).await?}));
            }
            expect_state(&value, true).map_err(|error|format!("Ordinary reopen /profile-reopen: {error}"))?;
            if first.profile_identity_token != reopened.profile_identity_token { return Err("Ordinary reopen changed effective Profile identity".into()); }
        }
        if mode != "preserved" {
            let other_key = managed_workspace_key(OWNER, OTHER_SESSION).map_err(|error| error.code().to_owned())?;
            let other_profile = store.profile_for(&other_key, BrowserProfilePersistence::Persistent).map_err(|error| error.code().to_owned())?;
            let other = service.ensure_user(OWNER, OTHER_SESSION, other_profile).await.map_err(|error| error.code().to_owned())?;
            url.set_path(if mode == "seed" { "/profile-isolated" } else { "/profile-restart-isolated" });
            let (value, other_runtime) = page_result(app, &other, url.as_str()).await?;
            expect_state(&value, false).map_err(|error|format!("Other Session {}: {error}",url.path()))?;
            if first.profile_identity_token == other_runtime.profile_identity_token { return Err("Distinct Sessions shared effective Profile identity".into()); }
        }
        Ok(json!({"persistent_http_cookie_localstorage_indexeddb":true,"session_isolation":true,"native_effective_profile_verified":true,"close_reopen_verified":mode=="seed","application_restart_read":mode=="restart","override_preserved_source_storage":mode=="preserved"}))
    }.await;
    if let Some(observer)=process_observer {observer.close(app).await?;}
    service.shutdown().await.map_err(|error| format!("Profile fixture native cleanup: {}",error.code()))?;
    result
}

async fn child(mode: &str, root: &Path, target: &str, ownership: &str, override_profile: Option<&Path>) -> Result<Value, String> {
    owned_root(root, ownership)?;
    let executable = std::env::current_exe().map_err(|_| "Profile child executable missing")?;
    let mut command = tokio::process::Command::new(executable);
    command.arg("--profile-child").env(PHASE_ENV, mode).env(ROOT_ENV, root)
        .env(TARGET_ENV, target).env(OWNERSHIP_ENV, ownership)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .kill_on_drop(true).creation_flags(0x08000000); // CREATE_NO_WINDOW; native App still owns its window.
    if std::env::args().any(|argument|argument==DELAY_DIAGNOSTIC_FLAG){command.arg(DELAY_DIAGNOSTIC_FLAG);}
    for name in ["WEBVIEW2_BROWSER_EXECUTABLE_FOLDER","WEBVIEW2_USER_DATA_FOLDER","WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS","WEBVIEW2_CHANNEL_SEARCH_KIND","WEBVIEW2_RELEASE_CHANNELS","WEBVIEW2_RELEASE_CHANNEL_PREFERENCE","WEBVIEW2_WAIT_FOR_SCRIPT_DEBUGGER","WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER"] {
        command.env_remove(name);
    }
    if let Some(profile) = override_profile { command.env("WEBVIEW2_USER_DATA_FOLDER", profile); }
    let child = command.spawn().map_err(|_| "Profile child launch failed")?;
    let output = tokio::time::timeout(Duration::from_secs(45), child.wait_with_output()).await
        .map_err(|_| "Profile child deadline expired")?
        .map_err(|_| "Profile child completion failed")?;
    if let Some(directory) = std::env::var_os(EVIDENCE_ENV) {
        let directory = PathBuf::from(directory);
        if !directory.is_absolute() || !directory.is_dir() { return Err("Profile evidence destination invalid".into()); }
        std::fs::write(directory.join(format!("phase-{mode}.stdout.log")), &output.stdout)
            .map_err(|_| "Profile child stdout capture failed")?;
        std::fs::write(directory.join(format!("phase-{mode}.stderr.log")), &output.stderr)
            .map_err(|_| "Profile child stderr capture failed")?;
        std::fs::write(directory.join(format!("phase-{mode}.result.json")),
            json!({"phase":mode,"success":output.status.success(),"code":output.status.code()}).to_string())
            .map_err(|_| "Profile child result capture failed")?;
    }
    if !output.status.success() {
        let failure = String::from_utf8_lossy(&output.stderr).lines().find_map(|line|
            line.strip_prefix("BROWSER_WORKSPACE_SMOKE_FAIL ").map(|text|text.chars().take(180).collect::<String>()))
            .unwrap_or_else(|| "native child phase failed".into());
        return Err(format!("Profile {mode} child: {failure}"));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|_| "Profile child evidence invalid")?;
    let evidence = stdout.lines().find_map(|line| line.strip_prefix("BROWSER_WORKSPACE_SMOKE_PASS "))
        .ok_or("Profile child pass evidence missing")?;
    serde_json::from_str(evidence).map_err(|_| "Profile child evidence malformed".into())
}

/// Orchestrator does not open these Profiles; each phase is a separate App.
pub(super) async fn verify(_app: &tauri::AppHandle) -> Result<Value, String> {
    if std::env::args().any(|arg|arg=="--profile-construction-ab"){return construction_ab().await;}
    if std::env::args().any(|arg|arg=="--profile-native-udf-representation"){return native_udf_representation().await;}
    let fixture = Fixture::new()?;
    let root = tempfile::tempdir().map_err(|_| "Profile fixture temporary root failed")?;
    let mismatched = tempfile::tempdir().map_err(|_| "Profile override temporary root failed")?;
    let ownership = uuid::Uuid::now_v7().to_string();
    for directory in [root.path(), mismatched.path()] {
        std::fs::write(directory.join(MARKER), &ownership).map_err(|_| "Profile fixture ownership creation failed")?;
    }
    let result = async {
        child("seed",root.path(),&fixture.url("/profile-seed"),&ownership,None).await?;
        child("restart",root.path(),&fixture.url("/profile-restart"),&ownership,None).await?;
        let store = BrowserProfileStore::new(root.path()).map_err(|error| error.code().to_owned())?;
        let key = managed_workspace_key(OWNER,SESSION).map_err(|error| error.code().to_owned())?;
        let BrowserProfile::Persistent(profile) = store.profile_for(&key,BrowserProfilePersistence::Persistent)
            .map_err(|error| error.code().to_owned())? else { return Err("Expected Windows directory Profile".into()); };
        child("override",mismatched.path(),&fixture.url("/profile-override"),&ownership,Some(&profile)).await?;
        if fixture.count("/profile-override") != 0 { return Err("Profile override sent target request before rejection".into()); }
        child("override-verbatim",root.path(),&fixture.url("/profile-override-verbatim"),&ownership,Some(&profile)).await?;
        if fixture.count("/profile-override-verbatim") != 0 { return Err("Verbatim matching override sent target request before rejection".into()); }
        // An override that selects the exact intended Profile is valid. This
        // ordinary spelling is verified against the same directory object;
        // no Profile identity or storage is moved for an environment override.
        let ordinary_profile=dunce::canonicalize(&profile).map_err(|_|"Matching ordinary override unavailable")?;
        if !matches!(ordinary_profile.components().next(),Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(),std::path::Prefix::Disk(_))) {
            return Err("Matching ordinary override is not a native ordinary disk path".into());
        }
        child("preserved",root.path(),&fixture.url("/profile-preserved"),&ownership,Some(&ordinary_profile)).await?;
        for path in ["/profile-seed","/profile-reopen","/profile-isolated","/profile-restart","/profile-restart-isolated","/profile-preserved"] {
            if fixture.count(path) != 1 { return Err("Profile fixture unexpected main-document request count".into()); }
        }
        for path in ["/profile-reopen","/profile-restart","/profile-preserved"] {
            if !fixture.cookie_received(path) { return Err("Persistent HTTP Cookie was absent from the real navigation request".into()); }
        }
        if fixture.cookie_received("/profile-isolated") || fixture.cookie_received("/profile-restart-isolated") {
            return Err("Other Session sent the persistent HTTP Cookie".into());
        }
        Ok(json!({"production_profile_store":true,"production_desktop_host":true,"http_only_cookie_network_witness":true,"cookie_localstorage_indexeddb_survive_close_reopen":true,"cookie_localstorage_indexeddb_survive_application_restart":true,"other_session_isolated_before_and_after_restart":true,"effective_profile_token_stable_and_isolated_within_process":true,"effective_profile_token_scope":"process","environment_override_rejected_before_target_get":true,"matching_environment_override_measured":true,"matching_verbatim_override_rejected_before_target_get":true,"override_target_get_count":0,"verbatim_override_target_get_count":0,"override_source_cookie_localstorage_indexeddb_preserved":true,"separate_application_phases":5,"controlled_environment_lifetime_intervention":std::env::var(PROCESS_DIAGNOSTIC_ENV).ok().as_deref()==Some("1"),"controlled_before_close_lifetime_intervention":std::env::args().any(|argument|argument==DELAY_DIAGNOSTIC_FLAG)}))
    }.await;
    if let Some(directory) = std::env::var_os(EVIDENCE_ENV) {
        let directory = PathBuf::from(directory);
        if directory.is_absolute() && directory.is_dir() {
            // Paths are controlled fixture routes; receipts retain no query,
            // header values, body, URL origin or Cookie value.
            std::fs::write(directory.join("profile-request-witness.json"),
                serde_json::to_vec_pretty(&*fixture.requests.lock().unwrap())
                    .map_err(|_| "Profile request witness serialization failed")?)
                .map_err(|_| "Profile request witness capture failed")?;
        }
    }
    if result.is_err() && std::env::var(KEEP_FAILURE_PROFILE_ENV).ok().as_deref()==Some("1") {
        let directory=std::env::var_os(EVIDENCE_ENV).map(PathBuf::from)
            .filter(|path|path.is_absolute()&&path.is_dir()).ok_or("Preserved Profile evidence destination missing")?;
        // This retains only these new, marker-owned synthetic fixtures after
        // child application exit. A shared browser PID is not declared exited.
        let source=root.keep();
        let mismatch=mismatched.keep();
        std::fs::write(directory.join("profile-failure-owned-roots.json"),serde_json::to_vec_pretty(&json!({
            "source_root":source,"mismatched_root":mismatch,"marker_filename":MARKER,
            "child_application_exit_confirmation":"see matching phase result metadata","browser_process_exit_proven":false,
            "controlled_fixture_only":true,"no_cookie_values_exported":true,
        })).map_err(|_|"Preserved Profile manifest serialization failed")?)
            .map_err(|_|"Preserved Profile manifest write failed")?;
        return result;
    }
    // These exact TempDir owners are the complete cleanup authority. Native
    // children have exited; never kill a shared WebView2 process for cleanup.
    super::cleanup_fixture_profile(&root)?;
    super::cleanup_fixture_profile(&mismatched)?;
    result
}

async fn construction_ab()->Result<Value,String> {
    let fixture=Fixture::new()?;
    let ownership=uuid::Uuid::now_v7().to_string();
    let mut results=Vec::new();
    for mode in ["constructor-legacy","constructor-external","constructor-retained-profile-manager"] {
        let root=tempfile::Builder::new().prefix("nomifun-profile-constructor-").tempdir().map_err(|_|"Constructor diagnostic owned root failed")?;
        std::fs::write(root.path().join(MARKER),&ownership).map_err(|_|"Constructor diagnostic marker failed")?;
        let result=child(mode,root.path(),&fixture.url("/profile-seed"),&ownership,None).await;
        // Separate new UDFs avoid reusing locked or still-active browser state.
        super::cleanup_fixture_profile(&root)?;
        results.push(result?);
    }
    Ok(json!({"diagnostic_only":true,"same_origin_set_cookie_and_empty_state":true,"independent_fresh_owned_udfs":true,"no_cookie_policy_or_runtime_settings_changed":true,"variants":results}))
}

// A bounded native-property measurement for the effective-directory guard.
// Each public-SDK variant owns its fresh root, controller and child App. The
// getter representation is reported only as booleans before canonicalization.
async fn native_udf_representation()->Result<Value,String> {
    let fixture=Fixture::new()?;
    let ownership=uuid::Uuid::now_v7().to_string();
    let mut variants=Vec::new();
    for mode in ["constructor-raw-sdk-default","constructor-raw-sdk-normalized-input"] {
        let root=tempfile::Builder::new().prefix("nomifun-profile-udf-property-").tempdir().map_err(|_|"UDF property measurement root failed")?;
        std::fs::write(root.path().join(MARKER),&ownership).map_err(|_|"UDF property measurement marker failed")?;
        let result=child(mode,root.path(),&fixture.url("/profile-seed"),&ownership,None).await;
        super::cleanup_fixture_profile(&root)?;
        variants.push(result?);
    }
    Ok(json!({"diagnostic_only":true,"native_property_measurement":"UserDataFolder representation before canonicalization","independent_owned_udfs":true,"variants":variants}))
}
