use std::{fs, path::Path, sync::Arc, time::Duration};
use nomifun_app_hosting::{AppOperation, AppState, HostingOwner, ImportKind, LocalConnection, detect_environment};
use nomi_process_runtime::{ChildProcessBuilder, capture_child_identity, probe_process_identity, resolve_command_path};
use tempfile::TempDir;

async fn owner(root: &Path) -> Arc<HostingOwner> {
    HostingOwner::new(root.to_owned(), LocalConnection { host_url: "http://127.0.0.1:5177".into(), local_trust: "exact-test-host-secret-123456789".into() }).await.unwrap()
}

fn write(root: &Path, file: &str, contents: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(file), contents).unwrap();
}

async fn wait_for(path: &Path) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !path.exists() { tokio::time::sleep(Duration::from_millis(25)).await; }
    }).await.unwrap();
}

#[tokio::test]
async fn static_artifact_serves_real_http_and_original_survives_delete() {
    assert!(resolve_command_path("node").is_some(), "real Node is required for hosting integration tests");
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "index.html", "<h1>ordinary static application</h1>");
    let host = owner(&temp.path().join("hosting")).await;
    let app = host.import(&source, Some("static".into()), ImportKind::Artifact).await.unwrap();
    assert!(app.settings.install_command.is_none());
    assert_eq!(app.settings.output_directory.as_deref(), Some("."));
    let app = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(app.state, AppState::Running);
    let html = reqwest::get(app.settings.web_url.as_ref().unwrap()).await.unwrap().text().await.unwrap();
    assert!(html.contains("ordinary static application"));
    let pid = app.pid.unwrap();
    let again = host.operate(&app.id, AppOperation::Start).await.unwrap();
    assert_eq!(again.pid, Some(pid), "repeat start must keep the single owner");
    host.delete(&app.id).await.unwrap();
    assert!(source.join("index.html").exists());
    assert!(!app.directory.exists());
    assert!(probe_process_identity(pid).unwrap().is_none());
    assert!(host.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn source_sequence_credentials_custom_node_and_preview_switch() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "prepare.cjs", "const fs=require('fs');if(process.env.NOMIFUN_LOCAL_TRUST||process.env.NOMIFUN_HOST_URL||process.env.NOMIFUN_AGENT_BINDING_ID)process.exit(9);fs.appendFileSync('steps.txt',process.argv[2]+'\\n');");
    write(&source, "server.cjs", "const fs=require('fs');fs.writeFileSync('connection.json',JSON.stringify({host:process.env.NOMIFUN_HOST_URL,trust:process.env.NOMIFUN_LOCAL_TRUST,binding:process.env.NOMIFUN_AGENT_BINDING_ID,mode:process.argv[2]}));fs.appendFileSync('steps.txt',process.argv[2]+'\\n');console.log(process.env.NOMIFUN_LOCAL_TRUST);setInterval(()=>{},1000);");
    write(&source, "package.json", r#"{"scripts":{"dev":"node server.cjs preview","build":"node prepare.cjs build","start":"node server.cjs use"}}"#);
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, Some("source".into()), ImportKind::Source).await.unwrap();
    assert_eq!(app.settings.preview_command.as_deref(), Some("npm run dev"));
    let environment = detect_environment(None).await;
    if resolve_command_path("npm").is_some() { assert!(environment.npm.error.is_none(), "real system npm detection failed: {:?}", environment.npm); }
    if resolve_command_path("bun").is_some() { assert!(environment.bun.error.is_none(), "real system Bun detection failed: {:?}", environment.bun); }
    assert!(environment.node.version.as_ref().unwrap().starts_with('v'));
    let node_path = environment.node.path.unwrap();
    let custom = detect_environment(Some(node_path.to_str().unwrap())).await;
    assert!(custom.node.error.is_none(), "{:?}", custom.node);
    app.settings.node_path = Some(node_path.to_string_lossy().into_owned());
    app.settings.install_command = Some("node prepare.cjs install".into());
    app.settings.build_command = Some("node prepare.cjs build".into());
    app.settings.preview_command = Some("node server.cjs preview".into());
    app.settings.start_command = Some("node server.cjs use".into());
    app.settings.agent_binding_id = Some("existing-agent-binding".into());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    let preview = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    let preview_pid = preview.pid.unwrap();
    assert!(preview.preview);
    let steps = fs::read_to_string(app.directory.join("steps.txt")).unwrap();
    assert_eq!(steps, "install\npreview\n");
    let connection: serde_json::Value = serde_json::from_slice(&fs::read(app.directory.join("connection.json")).unwrap()).unwrap();
    assert_eq!(connection["trust"], "exact-test-host-secret-123456789");
    assert_eq!(connection["binding"], "existing-agent-binding");
    assert!(!preview.output.contains("exact-test-host-secret"));
    let registry = temp.path().join("hosting/registry").join(format!("{}.json", app.id));
    assert!(!fs::read_to_string(&registry).unwrap().contains("exact-test-host-secret"));
    let used = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert!(!used.preview);
    assert_ne!(used.pid, Some(preview_pid));
    assert!(probe_process_identity(preview_pid).unwrap().is_none());
    assert_eq!(fs::read_to_string(app.directory.join("steps.txt")).unwrap(), "install\npreview\ninstall\nbuild\nuse\n");
    host.shutdown().await.unwrap();
    assert_eq!(host.get(&app.id).await.unwrap().state, AppState::Stopped);
    assert!(source.join("server.cjs").exists());
}

#[tokio::test]
async fn no_ui_process_tree_is_stopped_and_unexpected_successful_exit_is_failed() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "server.cjs", "const cp=require('child_process'),fs=require('fs');const child=cp.spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});fs.writeFileSync('child.pid',String(child.pid));setInterval(()=>{},1000);");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert!(running.settings.web_url.is_none());
    wait_for(&app.directory.join("child.pid")).await;
    let child_pid: u32 = fs::read_to_string(app.directory.join("child.pid")).unwrap().parse().unwrap();
    assert!(probe_process_identity(child_pid).unwrap().is_some());
    host.operate(&app.id, AppOperation::Stop).await.unwrap();
    assert!(probe_process_identity(child_pid).unwrap().is_none());
    assert!(probe_process_identity(running.pid.unwrap()).unwrap().is_none());
    write(&app.directory, "exit.cjs", "setTimeout(()=>process.exit(0),600)");
    app.settings.start_command = Some("node exit.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    host.operate(&app.id, AppOperation::Start).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop { if host.get(&app.id).await.unwrap().state == AppState::Failed { break; } tokio::time::sleep(Duration::from_millis(50)).await; }
    }).await.unwrap();
    assert!(host.get(&app.id).await.unwrap().last_error.unwrap().contains("exited unexpectedly"));
}

#[tokio::test]
async fn a_slow_install_does_not_lock_other_apps_and_stop_cancels_it() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "slow.cjs", "require('fs').writeFileSync('installing','yes');setInterval(()=>{},1000)");
    write(&source, "server.cjs", "setInterval(()=>{},1000)");
    let host = owner(&temp.path().join("hosting")).await;
    let mut slow = host.import(&source, Some("slow".into()), ImportKind::Source).await.unwrap();
    slow.settings.install_command = Some("node slow.cjs".into());
    slow.settings.start_command = Some("node server.cjs".into());
    host.update(&slow.id, slow.name.clone(), slow.settings).await.unwrap();
    let mut fast = host.import(&source, Some("fast".into()), ImportKind::Artifact).await.unwrap();
    fast.settings.start_command = Some("node server.cjs".into());
    host.update(&fast.id, fast.name.clone(), fast.settings).await.unwrap();
    let worker_host = host.clone();
    let slow_id = slow.id.clone();
    let installing = tokio::spawn(async move { worker_host.operate(&slow_id, AppOperation::Use).await });
    wait_for(&slow.directory.join("installing")).await;
    let fast = tokio::time::timeout(Duration::from_secs(3), host.operate(&fast.id, AppOperation::Use)).await.unwrap().unwrap();
    assert_eq!(fast.state, AppState::Running);
    tokio::time::timeout(Duration::from_secs(3), host.operate(&slow.id, AppOperation::Stop)).await.unwrap().unwrap();
    assert!(matches!(installing.await.unwrap(), Err(nomifun_app_hosting::AppError::Cancelled)));
    assert_eq!(host.get(&slow.id).await.unwrap().state, AppState::Stopped);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_command_stops_sequence_and_overwrite_preserves_current_data() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "server.cjs", "require('fs').writeFileSync('started.txt','old');setInterval(()=>{},1000)");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Source).await.unwrap();
    app.settings.install_command = Some("nomifun-command-that-does-not-exist-123".into());
    app.settings.build_command = Some("node -e \"require('fs').writeFileSync('built','yes')\"".into());
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    assert!(host.operate(&app.id, AppOperation::Use).await.is_err());
    assert_eq!(host.get(&app.id).await.unwrap().state, AppState::Failed);
    let failed_output = host.get(&app.id).await.unwrap().output;
    assert!(failed_output.contains("nomifun-command-that-does-not-exist-123"));
    assert!(!failed_output.contains('\u{fffd}'), "native shell diagnostics must use the shared process decoder: {failed_output}");
    assert!(!app.directory.join("built").exists());
    assert!(!app.directory.join("started.txt").exists());
    app.settings.install_command = None;
    app.settings.build_command = None;
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    host.operate(&app.id, AppOperation::Use).await.unwrap();
    write(&app.directory, "business-data.json", "user data must survive replacement");
    let replacement = temp.path().join("replacement");
    write(&replacement, "server.cjs", "require('fs').writeFileSync('started.txt','new');setInterval(()=>{},1000)");
    host.overwrite(&app.id, &replacement).await.unwrap();
    assert_eq!(fs::read_to_string(app.directory.join("business-data.json")).unwrap(), "user data must survive replacement");
    host.operate(&app.id, AppOperation::Rebuild).await.unwrap();
    assert_eq!(fs::read_to_string(app.directory.join("started.txt")).unwrap(), "new");
    assert!(source.join("server.cjs").exists());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_reconciles_exact_identity_and_never_kills_a_reused_pid() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "server.cjs", "setInterval(()=>{},1000)");
    let root = temp.path().join("hosting");
    let host = owner(&root).await;
    let app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    host.shutdown().await.unwrap();
    let mut command = ChildProcessBuilder::clean_cli("node");
    command.args(["-e", "setInterval(()=>{},1000)"]);
    let mut unrelated = command.spawn_managed().unwrap();
    let identity = capture_child_identity(unrelated.child()).unwrap();
    let registry = root.join("registry").join(format!("{}.json", app.id));
    let mut stored: serde_json::Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    let mut recycled = identity.clone();
    recycled.platform_start_key += 1;
    stored["identity"] = serde_json::to_value(recycled).unwrap();
    stored["record"]["state"] = "running".into();
    stored["record"]["pid"] = identity.pid.into();
    fs::write(&registry, serde_json::to_vec(&stored).unwrap()).unwrap();
    let restarted = owner(&root).await;
    assert_eq!(restarted.get(&app.id).await.unwrap().state, AppState::Failed);
    assert!(probe_process_identity(identity.pid).unwrap().is_some(), "a mismatched process identity must never be killed");
    #[cfg(target_os = "macos")]
    {
        assert_eq!(restarted.get(&app.id).await.unwrap().pid, Some(identity.pid), "root-only proof must retain cleanup evidence");
        let error = restarted.operate(&app.id, AppOperation::Stop).await.unwrap_err();
        assert!(error.to_string().contains("cleanup remains unproven"));
        assert_eq!(restarted.get(&app.id).await.unwrap().state, AppState::Failed);
        unrelated.shutdown().await.unwrap();
        restarted.operate(&app.id, AppOperation::Stop).await.unwrap();
    }
    restarted.shutdown().await.unwrap();
    unrelated.shutdown().await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn restart_retains_gone_root_with_live_group_as_unknown_and_stop_cannot_clear_it() {
    use std::process::Stdio;
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "index.html", "<p>recovery proof fixture</p>");
    let registry_root = temp.path().join("hosting");
    let host = owner(&registry_root).await;
    let app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    host.shutdown().await.unwrap();
    let marker = temp.path().join("child.pid");
    let exit_gate = temp.path().join("root.exit");
    // Model the older persisted root whose parent ownership no longer exists.
    // The child expires by itself; no fixture cleanup uses a recycled PID.
    let mut command = tokio::process::Command::new("/bin/sh");
    command.args(["-c", "/bin/sleep 10 & printf '%s\\n' \"$!\" > \"$1\"; while [ ! -f \"$2\" ]; do /bin/sleep 0.01; done", "hosting-root"])
        .arg(&marker).arg(&exit_gate).process_group(0)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
    let mut root = command.spawn().unwrap();
    let identity = capture_child_identity(&root).unwrap();
    wait_for(&marker).await;
    let descendant = fs::read_to_string(&marker).unwrap().trim().parse::<u32>().unwrap();
    let registry = registry_root.join("registry").join(format!("{}.json", app.id));
    let mut stored: serde_json::Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    stored["identity"] = serde_json::to_value(&identity).unwrap();
    stored["record"]["state"] = "running".into();
    stored["record"]["pid"] = identity.pid.into();
    fs::write(&registry, serde_json::to_vec(&stored).unwrap()).unwrap();
    fs::write(&exit_gate, b"exit").unwrap();
    root.wait().await.unwrap();
    assert!(probe_process_identity(descendant).unwrap().is_some(), "fixture descendant must survive its root");

    let restarted = owner(&registry_root).await;
    let state = restarted.get(&app.id).await.unwrap();
    assert_eq!(state.state, AppState::Failed);
    assert_eq!(state.pid, Some(identity.pid));
    assert!(state.last_error.as_deref().unwrap().contains("cleanup could not be confirmed"));
    let error = restarted.operate(&app.id, AppOperation::Stop).await.unwrap_err();
    assert!(error.to_string().contains("cleanup remains unproven"));
    assert_eq!(restarted.get(&app.id).await.unwrap().state, AppState::Failed);
    let retained: serde_json::Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    assert_eq!(retained["identity"], serde_json::to_value(identity).unwrap());
    assert!(probe_process_identity(descendant).unwrap().is_some(), "root-only proof must not signal or clear the surviving group");
}

#[tokio::test]
async fn zip_import_supports_enclosing_folder_and_rejects_escape() {
    use std::io::Write;
    let temp = TempDir::new().unwrap();
    let archive_path = temp.path().join("app.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&archive_path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("app/index.html", options).unwrap();
    zip.write_all(b"zip artifact").unwrap();
    zip.finish().unwrap();
    let host = owner(&temp.path().join("hosting")).await;
    let app = host.import(&archive_path, None, ImportKind::Artifact).await.unwrap();
    assert_eq!(app.settings.project_directory, "app");
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(reqwest::get(running.settings.web_url.as_ref().unwrap()).await.unwrap().text().await.unwrap(), "zip artifact");
    host.shutdown().await.unwrap();
    let evil_path = temp.path().join("evil.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&evil_path).unwrap());
    zip.start_file("../escaped.txt", options).unwrap();
    zip.write_all(b"escape").unwrap();
    zip.finish().unwrap();
    let another = owner(&temp.path().join("another")).await;
    assert!(another.import(&evil_path, None, ImportKind::Artifact).await.is_err());
    assert!(!temp.path().join("escaped.txt").exists());
    assert!(another.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn reserved_connection_environment_and_project_escape_are_rejected() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("original");
    write(&source, "index.html", "static");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.environment.insert("NOMIFUN_LOCAL_TRUST".into(), "user override".into());
    assert!(host.update(&app.id, app.name.clone(), app.settings.clone()).await.is_err());
    app.settings.environment.clear();
    app.settings.project_directory = "..".into();
    assert!(host.update(&app.id, app.name.clone(), app.settings).await.is_err());
}

#[tokio::test]
async fn preview_without_preview_command_runs_install_build_start_and_bounds_output() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write(&source, "prepare.cjs", "require('fs').appendFileSync('steps',process.argv[2]+'\\n');");
    write(&source, "server.cjs", "require('fs').appendFileSync('steps','start\\n');console.log('x'.repeat(40000));process.stdout.write(process.env.NOMIFUN_LOCAL_TRUST.slice(0,15));require('fs').writeFileSync('server-ready','yes');setInterval(()=>{},1000);");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Source).await.unwrap();
    app.settings.install_command = Some("node prepare.cjs install".into());
    app.settings.build_command = Some("node prepare.cjs build".into());
    app.settings.start_command = Some("node server.cjs".into());
    app.settings.preview_command = None;
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    host.operate(&app.id, AppOperation::Preview).await.unwrap();
    wait_for(&app.directory.join("server-ready")).await;
    let preview = host.get(&app.id).await.unwrap();
    assert_eq!(fs::read_to_string(app.directory.join("steps")).unwrap(), "install\nbuild\nstart\n");
    assert!(preview.preview);
    assert!(preview.output.len() <= 32 * 1024);
    assert!(!preview.output.contains("exact-test-host"), "a partial credential must also be withheld");
    host.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "subprocess fixture, executed only by the real Host crash test"]
async fn crash_fixture() {
    let root = std::env::var_os("NOMIFUN_HOSTING_TEST_ROOT").expect("fixture needs isolated root");
    let root = std::path::PathBuf::from(root);
    let source = root.join("source");
    write(&source, "server.cjs", "setInterval(()=>{},1000)");
    let host = owner(&root.join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    fs::write(root.join("ready.json"), serde_json::to_vec(&running).unwrap()).unwrap();
    std::future::pending::<()>().await;
}

#[tokio::test]
async fn actual_host_crash_then_restart_reports_failed_without_relaunch() {
    let temp = TempDir::new().unwrap();
    let mut command = ChildProcessBuilder::clean_cli(std::env::current_exe().unwrap());
    command.args(["--exact", "crash_fixture", "--ignored", "--nocapture"])
        .env("NOMIFUN_HOSTING_TEST_ROOT", temp.path());
    let mut host_process = command.spawn_managed().unwrap();
    wait_for(&temp.path().join("ready.json")).await;
    let app: nomifun_app_hosting::AppRecord = serde_json::from_slice(&fs::read(temp.path().join("ready.json")).unwrap()).unwrap();
    assert!(probe_process_identity(app.pid.unwrap()).unwrap().is_some());
    host_process.start_kill().unwrap();
    host_process.wait().await.unwrap();
    let restarted = owner(&temp.path().join("hosting")).await;
    let state = restarted.get(&app.id).await.unwrap();
    assert_eq!(state.state, AppState::Failed);
    if state.pid.is_some() {
        // Windows Job-close teardown can race the recovery lineage probe.
        // This is a truthful failed observation with retained exact evidence,
        // not permission to claim stopped or discard its cleanup authority.
        assert!(state.last_error.as_deref().unwrap().contains("cleanup could not be confirmed"));
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if restarted.operate(&app.id, AppOperation::Stop).await.is_ok() { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.unwrap();
        assert!(restarted.get(&app.id).await.unwrap().pid.is_none());
    }
    assert!(probe_process_identity(app.pid.unwrap()).unwrap().is_none());
    assert!(state.last_error.unwrap().contains("Host restarted"));
    restarted.shutdown().await.unwrap();
    host_process.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoning_the_waiter_retains_preparation_until_real_terminal_state() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write(&source, "prepare.cjs", "require('fs').writeFileSync('installing','yes');setTimeout(()=>process.exit(0),600)");
    write(&source, "server.cjs", "setInterval(()=>{},1000)");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Source).await.unwrap();
    app.settings.install_command = Some("node prepare.cjs".into());
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let waiter_host = host.clone();
    let id = app.id.clone();
    let waiter = tokio::spawn(async move { waiter_host.operate(&id, AppOperation::Use).await });
    wait_for(&app.directory.join("installing")).await;
    waiter.abort();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = host.get(&app.id).await.unwrap();
            if state.state == AppState::Running { assert!(state.operation.is_none()); break; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.unwrap();
    host.shutdown().await.unwrap();
    assert_eq!(host.get(&app.id).await.unwrap().state, AppState::Stopped);
}

#[tokio::test]
async fn cleanup_callback_failure_keeps_files_and_registration_for_retry() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write(&source, "server.cjs", "setInterval(()=>{},1000)");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    let pid = running.pid.unwrap();
    let result = host.delete_with(&app.id, move |record| async move {
        assert!(record.pid.is_none());
        assert!(probe_process_identity(pid).unwrap().is_none(), "process tree must be gone before domain cleanup");
        Err(nomifun_app_hosting::AppError::Invalid("domain cleanup is incomplete".into()))
    }).await;
    assert!(result.is_err());
    assert!(app.directory.exists());
    assert_eq!(host.get(&app.id).await.unwrap().state, AppState::Failed);
    host.delete_with(&app.id, |_| async { Ok(()) }).await.unwrap();
    assert!(!app.directory.exists());
    assert!(source.exists());
}

#[tokio::test]
async fn explicit_static_http_preset_serves_nonstandard_output_directory() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write(&source, "README.txt", "ordinary compiled artifacts");
    write(&source.join("web-files/pages"), "index.html", "nonstandard static output");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    assert!(app.settings.start_command.is_none(), "arbitrary artifacts must not be silently treated as static");
    app.settings.output_directory = Some("web-files/pages".into());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    let inferred = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(reqwest::get(inferred.settings.web_url.unwrap()).await.unwrap().text().await.unwrap(), "nonstandard static output");
    host.operate(&app.id, AppOperation::Stop).await.unwrap();
    app.settings.start_command = Some("node .nomifun-static-server.cjs".into());
    let configured = host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    assert!(configured.settings.web_url.is_some());
    assert!(configured.settings.environment.contains_key("PORT"));
    assert!(app.directory.join(".nomifun-static-server.cjs").exists());
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(reqwest::get(running.settings.web_url.unwrap()).await.unwrap().text().await.unwrap(), "nonstandard static output");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn settings_filesystem_failure_compensates_before_a_queued_start_can_run() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    write(&source, "server.cjs", "const fs=require('fs');fs.writeFileSync('started-domain',fs.readFileSync('domain-state','utf8'));setInterval(()=>{},1000)");
    let root = temp.path().join("hosting");
    let host = owner(&root).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    fs::write(app.directory.join("domain-state"), "old").unwrap();
    let registry = root.join("registry").join(format!("{}.json", app.id));
    let prior_bytes = fs::read(&registry).unwrap();
    fs::remove_file(&registry).unwrap();
    fs::create_dir(&registry).unwrap(); // a real atomic-rename failure
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let change_host = host.clone();
    let app_id = app.id.clone();
    let change_settings = app.settings.clone();
    let domain = app.directory.join("domain-state");
    let rollback_domain = domain.clone();
    let rollback_entered = entered.clone();
    let rollback_release = release.clone();
    let change = tokio::spawn(async move {
        change_host.update_with_rollback(&app_id, "changed name".into(), change_settings,
            move |_, settings| async move { fs::write(domain, "new").unwrap(); Ok(settings) },
            move || async move {
                rollback_entered.notify_one();
                rollback_release.notified().await;
                fs::write(rollback_domain, "old").unwrap();
                fs::remove_dir(&registry).unwrap();
                fs::write(registry, prior_bytes).unwrap();
                Ok(())
            }).await
    });
    entered.notified().await;
    let start_host = host.clone();
    let start_id = app.id.clone();
    let mut start = tokio::spawn(async move { start_host.operate(&start_id, AppOperation::Start).await });
    assert!(tokio::time::timeout(Duration::from_millis(80), &mut start).await.is_err(), "start must wait for exact compensation under the application gate");
    assert!(!app.directory.join("started-domain").exists());
    release.notify_one();
    assert!(change.await.unwrap().is_err());
    start.await.unwrap().unwrap();
    wait_for(&app.directory.join("started-domain")).await;
    assert_eq!(fs::read_to_string(app.directory.join("started-domain")).unwrap(), "old");
    assert_eq!(host.get(&app.id).await.unwrap().name, app.name);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn compiled_server_artifact_uses_real_start_and_preserves_external_data_on_delete() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("compiled-original");
    write(&source, "package.json", r#"{"scripts":{"start":"node dist/server.cjs","build":"node absent-build.cjs","preinstall":"node absent-install.cjs"}}"#);
    write(&source.join("dist"), "index.html", "STATIC_OUTPUT_MUST_NOT_REPLACE_SERVER");
    write(&source.join("dist"), "server.cjs", r#"
const http = require('node:http'), fs = require('node:fs');
fs.writeFileSync(process.env.EXTERNAL_DATA_FILE, 'external application data');
http.createServer((req, res) => {
  res.setHeader('Content-Type', 'text/plain');
  res.end('DYNAMIC_COMPILED_SERVER_RESPONSE');
}).listen(Number(process.env.PORT), '127.0.0.1', () => fs.writeFileSync('server-ready', 'yes'));
"#);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let external = temp.path().join("external-data.json");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, Some("compiled server".into()), ImportKind::Artifact).await.unwrap();
    assert_eq!(app.settings.start_command.as_deref(), Some("npm run start"));
    assert!(app.settings.install_command.is_none());
    assert!(app.settings.build_command.is_none());
    assert!(app.settings.output_directory.is_none(), "a server's dist/index.html must not select static hosting");
    assert!(!app.directory.join(".nomifun-static-server.cjs").exists());
    app.settings.environment.insert("PORT".into(), port.to_string());
    app.settings.environment.insert("EXTERNAL_DATA_FILE".into(), external.to_string_lossy().into_owned());
    app.settings.web_url = Some(format!("http://127.0.0.1:{port}"));
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    wait_for(&app.directory.join("server-ready")).await;
    let body = reqwest::get(running.settings.web_url.unwrap()).await.unwrap().text().await.unwrap();
    assert_eq!(body, "DYNAMIC_COMPILED_SERVER_RESPONSE");
    assert_eq!(fs::read_to_string(&external).unwrap(), "external application data");
    assert_eq!(host.operate(&app.id, AppOperation::Stop).await.unwrap().state, AppState::Stopped);
    assert!(probe_process_identity(running.pid.unwrap()).unwrap().is_none());
    host.delete(&app.id).await.unwrap();
    assert!(!app.directory.exists());
    assert!(source.join("dist/server.cjs").exists());
    assert!(source.join("dist/index.html").exists());
    assert_eq!(fs::read_to_string(external).unwrap(), "external application data");
}

fn available_local_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn local_start_waits_for_delayed_listen_before_the_first_http_request() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("delayed-original");
    write(&source, "delayed.cjs", r#"
const fs=require('node:fs'),http=require('node:http');
fs.writeFileSync('starting',String(process.pid));
setTimeout(()=>http.createServer((req,res)=>{
  fs.writeFileSync('http-request-seen','yes');res.end('DELAYED_REAL_HTTP');
}).listen(Number(process.env.PORT),'127.0.0.1',()=>fs.writeFileSync('server-ready','yes')),400);
"#);
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    let port = available_local_port();
    app.settings.start_command = Some("node delayed.cjs".into());
    app.settings.preview_command = app.settings.start_command.clone();
    app.settings.environment.insert("PORT".into(), port.to_string());
    app.settings.web_url = Some(format!("http://127.0.0.1:{port}"));
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(running.state, AppState::Running);
    assert!(app.directory.join("starting").exists());
    assert!(!app.directory.join("http-request-seen").exists(), "readiness must not send HTTP, TLS or any credentials");
    // A single immediate request exposes the first-preview race; no HTTP retry
    // helper can turn a failed first request into a passing startup assertion.
    let response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(1)).build().unwrap()
        .get(running.settings.web_url.unwrap()).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "DELAYED_REAL_HTTP");
    host.shutdown().await.unwrap();
    assert!(probe_process_identity(running.pid.unwrap()).unwrap().is_none());
    assert!(source.join("delayed.cjs").exists());
}

#[tokio::test]
async fn local_listen_wait_is_cancelled_by_stop_and_host_shutdown() {
    for shutdown in [false, true] {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("never-listen-original");
        write(&source, "never-listen.cjs", "require('fs').writeFileSync('starting',String(process.pid));setInterval(()=>{},1000);");
        let host = owner(&temp.path().join("hosting")).await;
        let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
        app.settings.start_command = Some("node never-listen.cjs".into());
        app.settings.web_url = Some(format!("http://127.0.0.1:{}", available_local_port()));
        host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
        let worker_host = host.clone();
        let id = app.id.clone();
        let startup = tokio::spawn(async move { worker_host.operate(&id, AppOperation::Preview).await });
        wait_for(&app.directory.join("starting")).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(!startup.is_finished(), "the operation is in its added local listen wait");
        let pid = host.get(&app.id).await.unwrap().pid.unwrap();
        let node_pid: u32 = fs::read_to_string(app.directory.join("starting")).unwrap().parse().unwrap();
        if shutdown {
            tokio::time::timeout(Duration::from_millis(1500), host.shutdown()).await.unwrap().unwrap();
        } else {
            tokio::time::timeout(Duration::from_millis(1500), host.operate(&app.id, AppOperation::Stop)).await.unwrap().unwrap();
        }
        assert!(matches!(startup.await.unwrap(), Err(nomifun_app_hosting::AppError::Cancelled)));
        let stopped = host.get(&app.id).await.unwrap();
        assert_eq!(stopped.state, AppState::Stopped);
        assert!(stopped.pid.is_none() && stopped.operation.is_none());
        assert!(probe_process_identity(pid).unwrap().is_none());
        assert!(probe_process_identity(node_pid).unwrap().is_none());
        if !shutdown { host.shutdown().await.unwrap(); }
    }
}

#[tokio::test]
async fn startup_without_local_web_url_does_not_wait_or_probe_remote_hosts() {
    for web_url in [None, Some("http://example.invalid:54321"), Some("https://192.0.2.1:54321")] {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("ordinary-process");
        write(&source, "server.cjs", "setInterval(()=>{},1000);");
        let host = owner(&temp.path().join("hosting")).await;
        let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
        app.settings.start_command = Some("node server.cjs".into());
        app.settings.web_url = web_url.map(str::to_owned);
        host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
        let running = tokio::time::timeout(Duration::from_millis(1500), host.operate(&app.id, AppOperation::Start)).await.unwrap().unwrap();
        assert_eq!(running.state, AppState::Running);
        host.shutdown().await.unwrap();
        assert!(probe_process_identity(running.pid.unwrap()).unwrap().is_none());
    }
}

#[tokio::test]
async fn local_listen_timeout_preserves_process_running_semantics() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("no-web-service");
    write(&source, "server.cjs", "setInterval(()=>{},1000);");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    app.settings.web_url = Some(format!("http://127.0.0.1:{}", available_local_port()));
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let began = tokio::time::Instant::now();
    let running = tokio::time::timeout(Duration::from_secs(5), host.operate(&app.id, AppOperation::Start)).await.unwrap().unwrap();
    assert!(began.elapsed() >= Duration::from_millis(2000));
    assert_eq!(running.state, AppState::Running, "a listen timeout is not an invented command failure or health result");
    assert!(running.last_error.is_none());
    host.shutdown().await.unwrap();
    assert!(probe_process_identity(running.pid.unwrap()).unwrap().is_none());
}

#[tokio::test]
async fn foreground_exit_during_local_listen_wait_is_preserved_as_startup_failure() {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("early-exit");
    write(&source, "server.cjs", "setTimeout(()=>process.exit(7),500);");
    let host = owner(&temp.path().join("hosting")).await;
    let mut app = host.import(&source, None, ImportKind::Artifact).await.unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    app.settings.web_url = Some(format!("http://127.0.0.1:{}", available_local_port()));
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), host.operate(&app.id, AppOperation::Start)).await.unwrap().unwrap_err();
    assert!(error.to_string().contains("exited during startup"));
    let failed = host.get(&app.id).await.unwrap();
    assert_eq!(failed.state, AppState::Failed);
    assert!(failed.pid.is_none());
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn application_launch_uses_shared_environment_and_explicit_path_without_runtime_downloads() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = TempDir::new().unwrap();
    let source = temporary.path().join("source");
    write(&source, "readme.txt", "isolated fixture");
    let tools = temporary.path().join("user tools 中文");
    let observed = temporary.path().join("actual-environment");
    write(&tools, "fixture-app", "#!/bin/sh\nprintf '%s\n' \"$CUSTOM_VALUE\" \"$NOMIFUN_USER_SETTING\" \"$PATH\" > \"$OBSERVED_ENV\"\nexec /bin/sleep 30\n");
    fs::set_permissions(tools.join("fixture-app"), fs::Permissions::from_mode(0o755)).unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.import(&source, Some("environment fixture".into()), ImportKind::Source).await.unwrap();
    app.settings.install_command = None;
    app.settings.build_command = None;
    app.settings.start_command = Some("fixture-app".into());
    app.settings.web_url = None;
    app.settings.environment.insert("PATH".into(), tools.to_string_lossy().into_owned());
    app.settings.environment.insert("CUSTOM_VALUE".into(), "space 中文 'quote'".into());
    app.settings.environment.insert("NOMIFUN_USER_SETTING".into(), "explicit-user-value".into());
    app.settings.environment.insert("OBSERVED_ENV".into(), observed.to_string_lossy().into_owned());
    host.update(&app.id, app.name.clone(), app.settings.clone()).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Start).await.unwrap();
    assert_eq!(running.state, AppState::Running);
    wait_for(&observed).await;
    let actual = fs::read_to_string(&observed).unwrap();
    let values = actual.lines().collect::<Vec<_>>();
    assert_eq!(values[0], "space 中文 'quote'");
    assert_eq!(values[1], "explicit-user-value");
    assert!(values[2].starts_with(&tools.to_string_lossy().to_string()));
    host.delete(&app.id).await.unwrap();
}
