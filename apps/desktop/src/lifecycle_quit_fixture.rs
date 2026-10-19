//! Explicit debug-only Windows receipt fixture for the real Desktop startup
//! and ExitCoordinator. Never selects a daily data root or user browser.
use std::{path::{Path,PathBuf},sync::{Arc,OnceLock,atomic::{AtomicBool,Ordering}},time::Duration};
use tauri::Manager;
use nomifun_browser_platform::{runtime::{BrowserProfilePersistence,BrowserProfileStore,BrowserTabCommand},
    workspace::{BrowserResourceService,managed_workspace_key}};

struct Options {root:PathBuf,data:PathBuf,token:String,mode:String}
static OPTIONS:OnceLock<Options>=OnceLock::new();
static READY:AtomicBool=AtomicBool::new(false);
static REQUESTED:AtomicBool=AtomicBool::new(false);
static FAILED:AtomicBool=AtomicBool::new(false);

fn plain(path:&Path)->anyhow::Result<()> {
    use std::os::windows::fs::MetadataExt;
    let metadata=std::fs::symlink_metadata(path)?;
    anyhow::ensure!(metadata.file_attributes()&0x400==0,"fixture paths must not be links or reparse points");
    Ok(())
}

/// Called before storage/PATH workers or Tauri's single-instance plugin. A
/// random marker plus exact canonical Temp/data/work paths is mandatory.
pub(super) fn prepare_launch()->anyhow::Result<()> {
    let Some(root)=std::env::var_os("NOMIFUN_LIFECYCLE_FIXTURE_DIR") else {return Ok(());};
    let root=PathBuf::from(root);
    let token=std::env::var("NOMIFUN_LIFECYCLE_FIXTURE_TOKEN")?;
    let parsed=uuid::Uuid::parse_str(&token)?;
    anyhow::ensure!(parsed.get_version_num()==7&&parsed.to_string()==token,"fixture token must be canonical UUIDv7");
    plain(&root)?;
    let root=std::fs::canonicalize(root)?;
    let temp=std::fs::canonicalize(std::env::temp_dir())?;
    anyhow::ensure!(root.parent()==Some(temp.as_path())&&root.file_name().and_then(|name|name.to_str())
        ==Some(format!("nomifun-lifecycle-{token}").as_str()),"fixture must be the exact random direct Temp child");
    let marker=root.join("fixture-sentinel");plain(&marker)?;
    anyhow::ensure!(std::fs::read_to_string(&marker)?==token,"fixture sentinel mismatch");
    let data=PathBuf::from(std::env::var_os("NOMIFUN_DATA_DIR").ok_or_else(||anyhow::anyhow!("fixture data root required"))?);
    let work=PathBuf::from(std::env::var_os("NOMIFUN_WORK_DIR").ok_or_else(||anyhow::anyhow!("fixture work root required"))?);
    plain(&data)?;plain(&work)?;
    anyhow::ensure!(std::fs::canonicalize(&data)?==root.join("data")&&std::fs::canonicalize(work)?==root.join("work"),
        "fixture data/work must remain inside its exact disposable root");
    anyhow::ensure!(crate::browser_surface::windows::effective_environment::isolated_fixture_environment_proven(),
        "fixture requires a positively unredirected WebView2 environment/policy");
    let mode=std::env::var("NOMIFUN_LIFECYCLE_FIXTURE_MODE").unwrap_or_else(|_|"quit".into());
    anyhow::ensure!(matches!(mode.as_str(),"quit"|"parent-death"),"unsupported fixture mode");
    OPTIONS.set(Options {root,data:std::fs::canonicalize(data)?,token,mode})
        .map_err(|_|anyhow::anyhow!("fixture launch already prepared"))?;
    Ok(())
}

pub(super) fn active()->bool {OPTIONS.get().is_some()}
pub(super) fn shell_profile()->Option<PathBuf> {OPTIONS.get().map(|options|dunce::simplified(&options.root).join("shell-profile"))}
pub(super) fn identifier()->Option<String> {OPTIONS.get().map(|options|format!("fun.nomi.lifecycle.fixture.run{}",options.token.replace('-',"")))}

fn report(name:&str,value:&serde_json::Value)->anyhow::Result<()> {
    let options=OPTIONS.get().ok_or_else(||anyhow::anyhow!("fixture not prepared"))?;
    let temporary=options.root.join(format!("{name}.tmp"));
    std::fs::write(&temporary,serde_json::to_vec(value)?)?;
    std::fs::rename(temporary,options.root.join(format!("{name}.json")))?;
    Ok(())
}

#[derive(serde::Serialize)]
struct NativeProcess {pid:u32,kind:&'static str,kind_code:i32}

#[derive(serde::Serialize)]
struct NativeEnvironment {role:&'static str,user_data_folder:PathBuf,browser_pid:u32,processes:Vec<NativeProcess>}

async fn browser_processes(view:&tauri::Webview,role:&'static str)->anyhow::Result<NativeEnvironment> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Environment7,ICoreWebView2Environment8,
        COREWEBVIEW2_PROCESS_KIND,COREWEBVIEW2_PROCESS_KIND_BROWSER,COREWEBVIEW2_PROCESS_KIND_RENDERER,
        COREWEBVIEW2_PROCESS_KIND_GPU,COREWEBVIEW2_PROCESS_KIND_UTILITY,COREWEBVIEW2_PROCESS_KIND_SANDBOX_HELPER,
        COREWEBVIEW2_PROCESS_KIND_PPAPI_PLUGIN,COREWEBVIEW2_PROCESS_KIND_PPAPI_BROKER};
    use ::windows::{core::{Interface,PWSTR},Win32::System::Com::CoTaskMemFree};
    let (sender,receiver)=tokio::sync::oneshot::channel();
    view.with_webview(move |native| {
        let result=(|| {
            let core=unsafe {native.controller().CoreWebView2()}?;
            let mut process=0;
            unsafe {core.BrowserProcessId(&mut process)}?;
            anyhow::ensure!(process!=0,"fixture Browser process identity missing");
            let environment=native.environment();
            let environment7=environment.cast::<ICoreWebView2Environment7>()?;
            let mut raw=PWSTR::null();
            let received=unsafe {environment7.UserDataFolder(&mut raw)};
            if let Err(error)=received {unsafe {CoTaskMemFree(Some(raw.0.cast()))};return Err(error.into());}
            let folder:anyhow::Result<String>=if raw.is_null(){Err(anyhow::anyhow!("native fixture UDF is missing"))}
                else {
                    let units=unsafe {raw.as_wide()};
                    if units.len()>32768 {Err(anyhow::anyhow!("native fixture UDF exceeds its path boundary"))}
                    else {String::from_utf16(units).map_err(Into::into)}
                };
            unsafe {CoTaskMemFree(Some(raw.0.cast()))};
            let user_data_folder=std::fs::canonicalize(folder?)?;
            let options=OPTIONS.get().ok_or_else(||anyhow::anyhow!("fixture identity missing"))?;
            anyhow::ensure!(user_data_folder.starts_with(&options.root),"native inventory escaped the isolated fixture UDF");
            let environment8=environment.cast::<ICoreWebView2Environment8>()?;
            let infos=unsafe {environment8.GetProcessInfos()}?;
            let mut count=0;unsafe {infos.Count(&mut count)}?;
            anyhow::ensure!(count>0,"native Environment ProcessInfos is empty");
            let mut processes=Vec::new();
            for index in 0..count {
                let info=unsafe {infos.GetValueAtIndex(index)}?;
                let mut pid=0;let mut kind=COREWEBVIEW2_PROCESS_KIND::default();
                unsafe {info.ProcessId(&mut pid)}?;unsafe {info.Kind(&mut kind)}?;
                anyhow::ensure!(pid>0,"native ProcessInfo has no process identity");
                let name=match kind {
                    COREWEBVIEW2_PROCESS_KIND_BROWSER=>"browser",COREWEBVIEW2_PROCESS_KIND_RENDERER=>"renderer",
                    COREWEBVIEW2_PROCESS_KIND_GPU=>"gpu",COREWEBVIEW2_PROCESS_KIND_UTILITY=>"utility",
                    COREWEBVIEW2_PROCESS_KIND_SANDBOX_HELPER=>"sandbox_helper",COREWEBVIEW2_PROCESS_KIND_PPAPI_PLUGIN=>"ppapi_plugin",
                    COREWEBVIEW2_PROCESS_KIND_PPAPI_BROKER=>"ppapi_broker",_=>"unknown",
                };
                processes.push(NativeProcess {pid:pid as u32,kind:name,kind_code:kind.0});
            }
            processes.sort_by_key(|process|process.pid);
            anyhow::ensure!(processes.iter().any(|info|info.pid==process&&info.kind=="browser"),"Environment inventory omitted its exact Browser process");
            // All COM interfaces above are dropped on this UI callback before
            // sending scalar identities. They cannot keep WebView2 alive.
            Ok::<_,anyhow::Error>(NativeEnvironment {role,user_data_folder,browser_pid:process,processes})
        })();
        let _=sender.send(result);
    })?;
    receiver.await?
}

async fn ready_processes(view:&tauri::Webview,role:&'static str)->anyhow::Result<NativeEnvironment> {
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let inventory=browser_processes(view,role).await?;
            if inventory.processes.iter().any(|process|process.kind=="renderer") {return Ok(inventory);}
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await?
}

/// Only the already-verified disposable fixture can ask its own Desktop host
/// to establish first-run credentials. The real setup/login routes retain all
/// their normal authority checks; no local-trust capability leaves this host.
async fn prepare_agent_owner(options:&Options,server:&nomifun_app::DesktopServer)->anyhow::Result<bool> {
    let request=options.root.join("agent-resources.request");
    match std::fs::symlink_metadata(&request) {
        Ok(_)=>plain(&request)?,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(false),
        Err(error)=>return Err(error.into()),
    }
    anyhow::ensure!(std::fs::read_to_string(&request)?==options.token,"agent-resource fixture request token mismatch");
    let origin=format!("http://127.0.0.1:{}",server.loopback_port());
    let credentials=serde_json::json!({"username":"lifecycle-fixture","password":format!("Fixture-only-{}!",options.token)});
    let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15)).pool_max_idle_per_host(0).build()?;
    let setup=client.post(format!("{origin}/api/auth/setup"))
        .header("x-nomi-local-trust",server.local_trust_secret()).json(&credentials).send().await?;
    anyhow::ensure!(setup.status().is_success(),"isolated owner setup failed with status {}",setup.status());
    // Do not persist or log the setup response/cookie. The sidecar receives
    // only the ordinary owner JWT from the subsequent real password login.
    drop(setup);
    let login=client.post(format!("{origin}/login")).json(&credentials).send().await?;
    anyhow::ensure!(login.status().is_success(),"isolated owner login failed with status {}",login.status());
    let response:serde_json::Value=login.json().await?;
    let token=response.get("token").and_then(serde_json::Value::as_str)
        .filter(|token|!token.is_empty()&&token.len()<=8192)
        .ok_or_else(||anyhow::anyhow!("isolated owner login returned no valid JWT"))?;
    let user=response.pointer("/user/user_id").and_then(serde_json::Value::as_str)
        .ok_or_else(||anyhow::anyhow!("isolated owner login returned no owner identity"))?;
    uuid::Uuid::parse_str(user)?;
    use std::io::Write;
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true)
        .open(options.root.join("agent-resources-owner.jwt"))?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    report("agent-resources-auth",&serde_json::json!({"token":options.token,"authenticated":true,
        "owner_id":user,"host_local_setup":true,"password_login":true,"local_trust_exported":false}))?;
    Ok(true)
}

pub(super) fn start(app:&tauri::AppHandle,server:Arc<nomifun_app::DesktopServer>)->anyhow::Result<()> {
    let Some(options)=OPTIONS.get() else {return Ok(());};
    let resources=app.state::<Option<Arc<BrowserResourceService>>>().inner().clone()
        .ok_or_else(||anyhow::anyhow!("fixture requires the real managed Browser owner"))?;
    let app=app.clone();
    tauri::async_runtime::spawn(async move {
        let result=async {
            // This host-created domain resource is owned only by the fixture;
            // no Agent/model, credential or persisted daily Session is used.
            let owner=format!("lifecycle-fixture-{}",options.token);
            let session=uuid::Uuid::now_v7().to_string();
            let key=managed_workspace_key(&owner,&session)?;
            let profile=BrowserProfileStore::new(options.data.clone())?.profile_for(&key,BrowserProfilePersistence::Persistent)?;
            let workspace=resources.ensure_user(&owner,&session,profile).await?;
            // A local page remains alive until Quit; hiding/idle cannot provide
            // a false automatic-retirement pass for this unknown webpage.
            let listener=std::net::TcpListener::bind("127.0.0.1:0")?;
            let page_port=listener.local_addr()?.port();
            std::thread::spawn(move || {
                use std::io::{Read,Write};
                for stream in listener.incoming() {
                    let Ok(mut stream)=stream else {break;};
                    let _=stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request=[0u8;1024];let _=stream.read(&mut request);
                    let body="<!doctype html><title>Lifecycle fixture</title><h1>Disposable lifecycle fixture</h1><script>setInterval(()=>document.body.dataset.tick=Date.now(),100)</script>";
                    let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes());
                }
            });
            let state=workspace.user_command(BrowserTabCommand::Create {url:format!("http://127.0.0.1:{page_port}/fixture")}).await?;
            let target=state.tabs.first().ok_or_else(||anyhow::anyhow!("fixture native tab missing"))?.target.clone();
            let view=app.get_webview(&target.tab_id).ok_or_else(||anyhow::anyhow!("fixture native registration missing"))?;
            let managed=ready_processes(&view,"managed").await?;
            let shell=app.get_webview("main").ok_or_else(||anyhow::anyhow!("fixture shell native registration missing"))?;
            let shell_inventory=ready_processes(&shell,"shell").await?;
            let process=managed.browser_pid;let shell_process=shell_inventory.browser_pid;
            anyhow::ensure!(managed.user_data_folder!=shell_inventory.user_data_folder&&process!=shell_process,
                "fixture shell and managed environments must have independent native UDF owners");
            let processes=std::collections::BTreeSet::from([process,shell_process]);
            let native_processes=managed.processes.iter().chain(&shell_inventory.processes).map(|process|process.pid)
                .collect::<std::collections::BTreeSet<_>>();
            let generation=workspace.runtime_generation();
            drop(shell);drop(view);drop(workspace);drop(resources);
            let agent_resources=prepare_agent_owner(options,server.as_ref()).await?;
            report("ready",&serde_json::json!({"token":options.token,"mode":options.mode,"host_pid":std::process::id(),
                "browser_pid":process,"browser_pids":processes,"native_process_pids":native_processes,
                "native_environments":[managed,shell_inventory],"backend_port":server.loopback_port(),"runtime_generation":generation,
                "identifier":app.config().identifier,"data_root":options.data,"agent_resources":agent_resources}))?;
            READY.store(true,Ordering::Release);
            tokio::time::timeout(Duration::from_secs(90),async {
                loop {
                    match tokio::fs::read_to_string(options.root.join("quit.request")).await {
                        Ok(token) if token==options.token=>break,
                        Ok(_)=>anyhow::bail!("fixture quit request token mismatch"),
                        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                        Err(error)=>return Err(error.into()),
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Ok::<(),anyhow::Error>(())
            }).await??;
            anyhow::ensure!(options.mode=="quit","parent-death fixture must be ended by the exact external host handle");
            REQUESTED.store(true,Ordering::Release);
            app.exit(0); // The real RunEvent/ExitCoordinator owns all cleanup.
            Ok::<(),anyhow::Error>(())
        }.await;
        if let Err(error)=result {
            FAILED.store(true,Ordering::Release);
            let _=report("failure",&serde_json::json!({"error":error.to_string(),"token":options.token}));
            app.exit(1);
        }
    });
    Ok(())
}

/// Runs after the real event loop returns. This is only the coordinator
/// receipt; the external runner independently owns OS process handles.
pub(super) fn after_run(code:i32,coordinator:&crate::ExitCoordinator)->i32 {
    let Some(options)=OPTIONS.get() else {return code;};
    let verified=coordinator.cleanup_verified.load(Ordering::Acquire);
    let pass=code==0&&verified&&READY.load(Ordering::Acquire)&&REQUESTED.load(Ordering::Acquire)&&!FAILED.load(Ordering::Acquire);
    if report("terminal",&serde_json::json!({"token":options.token,"exit_code":code,"coordinator_cleanup_verified":verified,
        "ready":READY.load(Ordering::Acquire),"quit_requested":REQUESTED.load(Ordering::Acquire),"pass":pass})).is_err() {return 1;}
    if pass {code} else {1}
}
