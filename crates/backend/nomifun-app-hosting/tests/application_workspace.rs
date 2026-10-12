//! A current application is the same owned working tree throughout creation,
//! import assistance, direct development and ordinary command hosting.
use std::{fs, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use nomifun_app_hosting::{AppDetails, AppMetadata, AppOperation, AppSettingsPatch, AppState, HostingOwner, ImportKind, LocalConnection};

async fn owner(root: &std::path::Path) -> Arc<HostingOwner> {
    HostingOwner::new(root.to_owned(), LocalConnection {
        host_url: "http://127.0.0.1:5177".into(), local_trust: "current-workspace-test".into(),
    }).await.unwrap()
}

#[tokio::test]
async fn current_application_metadata_and_direct_workspace_survive_reopen_without_reimport() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("hosting");
    let host = owner(&root).await;
    let mut app = host.create("  待办通知  ".into(), AppMetadata {
        description: "会话开发的应用".into(), tags: vec!["工具".into(), " 工具 ".into(), "效率".into()],
    }).await.unwrap();
    assert_eq!(app.name, "待办通知");
    assert_eq!(app.tags, ["工具", "效率"]);
    assert_eq!(app.import_kind, ImportKind::Source);
    assert_eq!(fs::read_dir(&app.directory).unwrap().count(), 0);
    let session = uuid::Uuid::now_v7().to_string();
    let verified = session.clone();
    app = host.development_session_with(&app.id, move |current| async move {
        assert!(current.development_session_id.is_none());
        Ok(verified)
    }).await.unwrap();
    fs::write(app.directory.join("server.cjs"), "setInterval(()=>{},1000)").unwrap();
    app.settings.start_command = Some("node server.cjs".into());
    app = host.update_details(&app.id, "通知".into(), AppDetails {
        description: Some("直接在当前应用目录开发".into()), tags: Some(vec!["工具".into()]), import_kind: None,
    }, app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(running.state, AppState::Running);
    assert!(running.pid.is_some());
    assert_eq!(host.list().await.unwrap().len(), 1, "direct development must not import a second record");
    host.shutdown().await.unwrap();
    drop(host);
    let host = owner(&root).await;
    let reopened = host.get(&app.id).await.unwrap();
    assert_eq!(reopened.description, "直接在当前应用目录开发");
    assert_eq!(reopened.development_session_id.as_deref(), Some(session.as_str()));
    assert!(serde_json::to_value(&reopened.settings).unwrap().get("exclusive_session_ids").is_none());
    assert!(reopened.directory.join("server.cjs").exists());
    assert!(host.delete(&app.id).await.is_err(), "a Session reference requires the canonical owner's workspace fence");
    host.delete_with(&app.id, |_| async { Ok(()) }).await.unwrap();
    assert!(host.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn manual_source_and_artifact_import_keep_the_same_application_and_original_files() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("user-data.json"), "{\"keep\":true}").unwrap();
    let session = uuid::Uuid::now_v7().to_string();
    app = host.development_session_with(&app.id, move |_| async move { Ok(session) }).await.unwrap();
    let source = temporary.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("package.json"), r#"{"scripts":{"dev":"node server.cjs","start":"node server.cjs"}}"#).unwrap();
    fs::write(source.join("server.cjs"), "setInterval(()=>{},1000)").unwrap();
    let imported = host.import_into(&app.id, &source, Some("手工导入".into()), ImportKind::Source).await.unwrap();
    assert_eq!(imported.id, app.id);
    assert_eq!(imported.directory, app.directory);
    assert_eq!(imported.development_session_id, app.development_session_id);
    assert_eq!(imported.settings.preview_command.as_deref(), Some("npm run dev"));
    assert!(imported.directory.join("user-data.json").exists());
    let artifact = temporary.path().join("artifact");
    fs::create_dir(&artifact).unwrap();
    fs::write(artifact.join("index.html"), "<h1>Compiled app</h1>").unwrap();
    let imported = host.import_into(&app.id, &artifact, None, ImportKind::Artifact).await.unwrap();
    assert_eq!(imported.import_kind, ImportKind::Artifact);
    assert!(imported.settings.install_command.is_none());
    assert_eq!(host.list().await.unwrap().len(), 1);
    host.delete_with(&app.id, |_| async { Ok(()) }).await.unwrap();
    assert!(source.join("server.cjs").exists());
    assert!(artifact.join("index.html").exists());
}

#[tokio::test]
async fn invalid_metadata_or_session_reference_does_not_partially_save_settings() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("应用".into(), AppMetadata { description: "保留".into(), tags: vec!["工具".into()] }).await.unwrap();
    let mut settings = app.settings.clone();
    settings.start_command = Some("must-not-be-saved".into());
    assert!(host.update_details(&app.id, "changed".into(), AppDetails {
        tags: Some(vec!["invalid\ncategory".into()]), ..Default::default()
    }, settings).await.is_err());
    let unchanged = host.get(&app.id).await.unwrap();
    assert_eq!(unchanged.name, app.name);
    assert_eq!(unchanged.description, app.description);
    assert_eq!(unchanged.tags, app.tags);
    assert!(unchanged.settings.start_command.is_none());
    assert!(host.development_session_with(&app.id, |_| async { Ok("arbitrary-reference".into()) }).await.is_err());
    assert!(host.get(&app.id).await.unwrap().development_session_id.is_none());
    assert!(host.create("invalid".into(), AppMetadata { tags: vec!["".into()], ..Default::default() }).await.is_err());
    assert_eq!(fs::read_dir(temporary.path().join("hosting/apps")).unwrap().count(), 1);
}

async fn real_http(url: &str) -> String {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Ok(response) = client.get(url).send().await {
                if response.status().is_success() { return response.text().await.unwrap(); }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.unwrap()
}

#[tokio::test]
async fn generated_nested_static_project_previews_actual_files_without_import_or_model_configuration() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    let session = uuid::Uuid::now_v7().to_string();
    let retained_session = session.clone();
    host.development_session_with(&app.id, move |_| async move { Ok(session) }).await.unwrap();
    fs::create_dir(app.directory.join("snake-game")).unwrap();
    fs::create_dir(app.directory.join(".git")).unwrap();
    fs::create_dir(app.directory.join("user-data")).unwrap();
    fs::write(app.directory.join("README.md"), "user source workspace").unwrap();
    fs::write(app.directory.join("user-data/score.json"), "{\"score\":42}").unwrap();
    let html = "<!doctype html><html><title>贪吃蛇</title><canvas id=snake></canvas></html>";
    fs::write(app.directory.join("snake-game/index.html"), html).unwrap();
    let unchanged = host.get(&app.id).await.unwrap();
    assert!(unchanged.settings.start_command.is_none(), "reading generated files must not configure or start a process");
    assert!(!app.directory.join("snake-game/.nomifun-static-server.cjs").exists());
    let preview = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(preview.id, app.id);
    assert_eq!(preview.directory, app.directory);
    assert_eq!(preview.development_session_id.as_deref(), Some(retained_session.as_str()));
    assert_eq!(preview.settings.project_directory, "snake-game");
    assert_eq!(preview.state, AppState::Running);
    assert!(preview.preview);
    let url = preview.settings.web_url.as_ref().unwrap();
    assert_eq!(real_http(url).await, html);
    assert_eq!(host.list().await.unwrap().len(), 1);
    let repeated = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(repeated.pid, preview.pid, "one owner must retain the same live preview");
    host.operate(&app.id, AppOperation::Stop).await.unwrap();
    fs::write(app.directory.join("snake-game/index.html"), "<h1>贪吃蛇改进版</h1>").unwrap();
    let used = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert!(!used.preview);
    assert_eq!(used.settings.web_url.as_ref(), Some(url));
    assert_eq!(real_http(url).await, "<h1>贪吃蛇改进版</h1>");
    assert!(app.directory.join("user-data/score.json").exists());
    host.shutdown().await.unwrap();
    assert!(reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(1)).build().unwrap().get(url).send().await.is_err());
}

#[tokio::test]
async fn missing_or_ambiguous_generated_projects_fail_without_inventing_an_address_or_modifying_files() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    let error = host.operate(&app.id, AppOperation::Preview).await.unwrap_err();
    assert!(error.to_string().contains("no runnable project"));
    let failed = host.get(&app.id).await.unwrap();
    assert_eq!(failed.state, AppState::Failed);
    assert!(failed.settings.web_url.is_none());
    assert!(failed.pid.is_none());
    for name in ["snake-game", "other-app"] {
        fs::create_dir(app.directory.join(name)).unwrap();
        fs::write(app.directory.join(name).join("index.html"), name).unwrap();
    }
    let error = host.operate(&app.id, AppOperation::Use).await.unwrap_err();
    assert!(error.to_string().contains("multiple runnable projects"));
    assert!(host.get(&app.id).await.unwrap().settings.start_command.is_none());
    assert!(!app.directory.join("snake-game/.nomifun-static-server.cjs").exists());
    let mut settings = app.settings;
    settings.project_directory = "snake-game".into();
    host.update(&app.id, app.name, settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(real_http(running.settings.web_url.as_ref().unwrap()).await, "snake-game");
    assert_eq!(fs::read_to_string(app.directory.join("other-app/index.html")).unwrap(), "other-app");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn generated_static_defaults_preserve_explicit_environment_and_preparation_commands() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.create("应用".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("index.html"), "configured static source").unwrap();
    fs::write(app.directory.join("install.cjs"), "require('fs').writeFileSync('prepared.txt',process.env.CUSTOM_VALUE)").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    app.settings.install_command = Some("node install.cjs".into());
    app.settings.environment.insert("PORT".into(), port.to_string());
    app.settings.environment.insert("CUSTOM_VALUE".into(), "preserved".into());
    let environment = app.settings.environment.clone();
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Use).await.unwrap();
    assert_eq!(running.settings.environment, environment);
    assert_eq!(running.settings.install_command.as_deref(), Some("node install.cjs"));
    assert_eq!(running.settings.web_url.as_deref(), Some(format!("http://127.0.0.1:{port}").as_str()));
    assert_eq!(fs::read_to_string(app.directory.join("prepared.txt")).unwrap(), "preserved");
    assert_eq!(real_http(running.settings.web_url.as_ref().unwrap()).await, "configured static source");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn configured_foreground_commands_are_never_replaced_by_generated_file_detection() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.create("后台应用".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("index.html"), "unrelated HTML").unwrap();
    // These detected scripts must not replace an already configured project,
    // including its deliberately null install/build preparation commands.
    fs::write(app.directory.join("package.json"), r#"{"scripts":{"dev":"must-not-run","build":"must-not-run","start":"must-not-run"}}"#).unwrap();
    fs::write(app.directory.join("worker.cjs"), "setInterval(()=>{},1000)").unwrap();
    app.settings.preview_command = Some("node worker.cjs".into());
    app.settings.start_command = Some("node worker.cjs".into());
    app.settings.environment.insert("CUSTOM_VALUE".into(), "unchanged".into());
    let original = serde_json::to_value(&app.settings).unwrap();
    host.update(&app.id, app.name, app.settings).await.unwrap();
    let running = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(serde_json::to_value(&running.settings).unwrap(), original);
    assert!(running.settings.web_url.is_none(), "a backend process cannot imply an invented preview URL");
    assert!(!app.directory.join(".nomifun-static-server.cjs").exists());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn configured_python_static_server_recovers_missing_preview_url_without_replacing_process_or_commands() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    fs::create_dir(app.directory.join("snake-game")).unwrap();
    let html = "<html><title>贪吃蛇</title><canvas></canvas></html>";
    fs::write(app.directory.join("snake-game/index.html"), html).unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    app.settings.project_directory = "snake-game".into();
    app.settings.preview_command = Some(format!("python -m http.server {port}"));
    app.settings.start_command = app.settings.preview_command.clone();
    let original_command = app.settings.start_command.clone();
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let preview = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    assert_eq!(preview.settings.web_url.as_deref(), Some(url.as_str()));
    assert_eq!(real_http(&url).await, html);
    let mut settings = preview.settings;
    settings.web_url = None;
    host.update(&app.id, app.name.clone(), settings).await.unwrap();
    assert!(host.get(&app.id).await.unwrap().settings.web_url.is_none());
    let recovered = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(recovered.settings.web_url.as_deref(), Some(url.as_str()));
    assert_eq!(recovered.pid, preview.pid, "a live same-command preview needs only URL recovery");
    assert_eq!(recovered.settings.start_command, original_command);
    assert!(recovered.settings.environment.is_empty());
    assert!(!app.directory.join("snake-game/.nomifun-static-server.cjs").exists());
    assert_eq!(real_http(&url).await, html);
    host.operate(&app.id, AppOperation::Stop).await.unwrap();
    let mut explicit = recovered.settings;
    explicit.web_url = Some("http://localhost:8123/custom".into());
    host.update(&app.id, app.name, explicit).await.unwrap();
    let restarted = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(restarted.settings.web_url.as_deref(), Some("http://localhost:8123/custom"));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn running_python_preview_rejects_execution_edits_before_preparation_and_accepts_them_after_stop() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let mut app = host.create("原应用".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("index.html"), "real Python preview").unwrap();
    fs::create_dir(app.directory.join("other-project")).unwrap();
    fs::write(app.directory.join("other-project/index.html"), "another project").unwrap();
    let old_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let new_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let old_port = old_listener.local_addr().unwrap().port();
    let new_port = new_listener.local_addr().unwrap().port();
    drop((old_listener, new_listener));
    app.settings.preview_command = Some(format!("python -m http.server {old_port} --bind 127.0.0.1"));
    app.settings.start_command = app.settings.preview_command.clone();
    host.update(&app.id, app.name.clone(), app.settings).await.unwrap();
    let live = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    let old_url = format!("http://127.0.0.1:{old_port}/");
    assert_eq!(real_http(&old_url).await, "real Python preview");
    let preparation_count = Arc::new(AtomicUsize::new(0));
    let mut changes = Vec::new();
    for field in 0..10 {
        let mut changed = live.settings.clone();
        changed.web_url = None;
        match field {
            0 => changed.project_directory = "other-project".into(),
            1 => changed.install_command = Some("must-not-run".into()),
            2 => changed.preview_command = Some(format!("python -m http.server {new_port}")),
            3 => changed.build_command = Some("must-not-run".into()),
            4 => changed.output_directory = Some("other-project".into()),
            5 => changed.start_command = Some(format!("python -m http.server {new_port}")),
            6 => changed.node_path = Some("must-not-select-node".into()),
            7 => { changed.environment.insert("PORT".into(), new_port.to_string()); }
            8 => changed.agent_id = Some(uuid::Uuid::now_v7().to_string()),
            9 => changed.agent_binding_id = Some(uuid::Uuid::now_v7().to_string()),
            _ => unreachable!(),
        }
        changes.push(changed);
    }
    for changed in changes {
        let preparation_count = preparation_count.clone();
        let error = host.update_details_with_rollback(&app.id, "must not save".into(), AppDetails {
            description: Some("must not save".into()), tags: Some(vec!["must not save".into()]), ..Default::default()
        }, changed, move |_, settings| async move {
            preparation_count.fetch_add(1, Ordering::SeqCst);
            Ok(settings)
        }, || async { Ok(()) }).await.unwrap_err();
        assert!(error.to_string().contains("stop the application"));
        let unchanged = host.get(&app.id).await.unwrap();
        assert_eq!(unchanged.pid, live.pid);
        assert_eq!(unchanged.state, AppState::Running);
        assert_eq!(unchanged.name, live.name);
        assert_eq!(unchanged.description, live.description);
        assert_eq!(unchanged.tags, live.tags);
        assert_eq!(serde_json::to_value(unchanged.settings).unwrap(), serde_json::to_value(&live.settings).unwrap());
    }
    assert_eq!(preparation_count.load(Ordering::SeqCst), 0, "rejected edits cannot create a connection or write a preset");
    assert!(host.update_details(&app.id, app.name.clone(), AppDetails {
        import_kind: Some(ImportKind::Artifact), ..Default::default()
    }, live.settings.clone()).await.unwrap_err().to_string().contains("stop the application"));
    assert_eq!(real_http(&old_url).await, "real Python preview");

    // Metadata and a cleared display URL do not change the running command.
    let mut display = live.settings.clone();
    display.web_url = None;
    display.mcp_url = Some(format!("http://127.0.0.1:{old_port}/mcp"));
    let metadata = host.update_details(&app.id, "新名称".into(), AppDetails {
        description: Some("更新描述".into()), tags: Some(vec!["游戏".into()]), ..Default::default()
    }, display).await.unwrap();
    assert_eq!(metadata.pid, live.pid);
    assert!(metadata.settings.web_url.is_none());
    assert_eq!(metadata.tags, ["游戏"]);
    let recovered = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(recovered.pid, live.pid);
    assert_eq!(recovered.settings.web_url.as_deref(), Some(old_url.as_str()));

    host.operate(&app.id, AppOperation::Stop).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(1)).build().unwrap();
    assert!(client.get(&old_url).send().await.is_err());
    let mut stopped = recovered.settings;
    stopped.preview_command = Some(format!("python -m http.server {new_port} --bind 127.0.0.1"));
    stopped.start_command = stopped.preview_command.clone();
    stopped.web_url = None;
    host.update(&app.id, recovered.name, stopped).await.unwrap();
    let restarted = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    let new_url = format!("http://127.0.0.1:{new_port}/");
    assert_eq!(restarted.settings.web_url.as_deref(), Some(new_url.as_str()));
    assert_ne!(restarted.pid, live.pid);
    assert_eq!(real_http(&new_url).await, "real Python preview");
    assert!(client.get(&old_url).send().await.is_err());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn generated_project_configuration_is_saved_before_any_process_runs() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("hosting");
    let host = owner(&root).await;
    let app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    let reference = uuid::Uuid::now_v7().to_string();
    let session = reference.clone();
    host.development_session_with(&app.id, move |_| async move { Ok(session) }).await.unwrap();
    fs::create_dir(app.directory.join("snake-game")).unwrap();
    fs::create_dir(app.directory.join(".git")).unwrap();
    fs::write(app.directory.join("README.md"), "ordinary development workspace").unwrap();
    fs::write(app.directory.join("user-data.json"), "KEEP").unwrap();
    fs::write(app.directory.join("snake-game/index.html"), "<title>贪吃蛇</title><canvas></canvas>").unwrap();
    let saved = host.update_patch_details(&app.id, Some("贪吃蛇".into()), AppDetails {
        description: Some("Agent 填写的实际应用说明".into()), tags: Some(vec!["游戏".into()]), ..Default::default()
    }, AppSettingsPatch::default(), true).await.unwrap();
    assert_eq!(saved.id, app.id);
    assert_eq!(saved.directory, app.directory);
    assert_eq!(saved.development_session_id.as_deref(), Some(reference.as_str()));
    assert_eq!(saved.settings.project_directory, "snake-game");
    assert_eq!(saved.settings.output_directory.as_deref(), Some("."));
    assert_eq!(saved.settings.start_command.as_deref(), Some("node \".nomifun-static-server.cjs\""));
    assert!(saved.settings.web_url.is_some());
    assert_eq!(saved.state, AppState::Stopped);
    assert!(saved.pid.is_none());
    assert!(!saved.prepared);
    assert!(saved.output.is_empty());
    assert_eq!(fs::read_to_string(app.directory.join("user-data.json")).unwrap(), "KEEP");
    host.shutdown().await.unwrap();
    drop(host);
    let host = owner(&root).await;
    let reopened = host.get(&app.id).await.unwrap();
    assert_eq!(reopened.name, "贪吃蛇");
    assert_eq!(reopened.tags, ["游戏"]);
    assert_eq!(serde_json::to_value(reopened.settings).unwrap(), serde_json::to_value(saved.settings).unwrap());
    assert_eq!(host.list().await.unwrap().len(), 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn sparse_updates_merge_current_fields_and_preserve_explicit_null_preparation_commands() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("新应用".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("package.json"), r#"{"scripts":{"dev":"vite","build":"vite build"}}"#).unwrap();
    fs::write(app.directory.join("bun.lock"), "").unwrap();
    let patch: AppSettingsPatch = serde_json::from_value(serde_json::json!({"install_command":null,"build_command":null})).unwrap();
    let detected = host.update_patch_details(&app.id, Some("前端项目".into()), AppDetails::default(), patch, true).await.unwrap();
    assert!(detected.settings.preview_command.as_deref().unwrap().starts_with("bun run dev"));
    assert!(detected.settings.install_command.is_none());
    assert!(detected.settings.build_command.is_none());
    assert!(detected.pid.is_none());
    assert!(!app.directory.join("node_modules").exists(), "configuration must not install dependencies");
    let first = host.update_patch_details(&app.id, Some("用户自己的名称".into()), AppDetails::default(), AppSettingsPatch::default(), false);
    let second = host.update_patch_details(&app.id, None, AppDetails { description: Some("并发元数据保存".into()), ..Default::default() },
        serde_json::from_value(serde_json::json!({"node_path":"custom-node"})).unwrap(), false);
    let (first, second) = tokio::join!(first, second);
    first.unwrap(); second.unwrap();
    let merged = host.get(&app.id).await.unwrap();
    assert_eq!(merged.name, "用户自己的名称");
    assert_eq!(merged.description, "并发元数据保存");
    assert_eq!(merged.settings.node_path.as_deref(), Some("custom-node"));
    assert_eq!(merged.settings.preview_command, detected.settings.preview_command);
    assert_eq!(merged.settings.web_url, detected.settings.web_url);
    assert!(merged.settings.install_command.is_none());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn detection_preserves_unknown_backend_configuration_and_checks_fence_before_writing_preset() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("后台任务".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("index.html"), "unrelated generated asset").unwrap();
    let failed = host.update_guarded_patch_details_with_rollback(&app.id, Some("must not save".into()), AppDetails::default(),
        AppSettingsPatch::default(), true, |_| Err(nomifun_app_hosting::AppError::Invalid("development reference changed".into())),
        |_, settings| async { Ok(settings) }, || async { Ok(()) }).await.unwrap_err();
    assert!(failed.to_string().contains("reference changed"));
    assert!(!app.directory.join(".nomifun-static-server.cjs").exists());
    let custom: AppSettingsPatch = serde_json::from_value(serde_json::json!({"start_command":"node worker.cjs",
        "preview_command":"node worker.cjs", "install_command":null,"build_command":null,"environment":{"CUSTOM":"preserved"}})).unwrap();
    let detected = host.update_patch_details(&app.id, None, AppDetails::default(), custom, true).await.unwrap();
    assert_eq!(detected.name, "后台任务");
    assert_eq!(detected.settings.start_command.as_deref(), Some("node worker.cjs"));
    assert!(detected.settings.web_url.is_none());
    assert!(detected.settings.install_command.is_none());
    assert!(detected.settings.build_command.is_none());
    assert_eq!(detected.settings.environment["CUSTOM"], "preserved");
    assert!(!app.directory.join(".nomifun-static-server.cjs").exists());
    assert!(serde_json::from_value::<AppSettingsPatch>(serde_json::json!({"agent_binding_id":"not-authorable"})).is_err());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn configuration_detection_can_repair_a_live_python_url_without_restarting_its_process() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("网页".into(), AppMetadata::default()).await.unwrap();
    fs::write(app.directory.join("index.html"), "python current configuration").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let command = format!("python -m http.server {port} --bind 127.0.0.1");
    let detected = host.update_patch_details(&app.id, None, AppDetails::default(),
        serde_json::from_value(serde_json::json!({"preview_command":command,"start_command":command})).unwrap(), true).await.unwrap();
    assert!(detected.pid.is_none());
    let url = format!("http://127.0.0.1:{port}/");
    assert_eq!(detected.settings.web_url.as_deref(), Some(url.as_str()));
    let running = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(real_http(&url).await, "python current configuration");
    host.update_patch_details(&app.id, None, AppDetails::default(),
        serde_json::from_value(serde_json::json!({"web_url":null})).unwrap(), false).await.unwrap();
    let restored = host.update_patch_details(&app.id, None, AppDetails::default(), AppSettingsPatch::default(), true).await.unwrap();
    assert_eq!(restored.pid, running.pid);
    assert_eq!(restored.settings.web_url.as_deref(), Some(url.as_str()));
    assert_eq!(restored.settings.start_command.as_deref(), Some(command.as_str()));
    assert!(restored.settings.environment.is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn ambiguous_detection_rejects_the_entire_update_without_partial_metadata_or_configuration() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("保留名称".into(), AppMetadata::default()).await.unwrap();
    for name in ["first", "second"] {
        fs::create_dir(app.directory.join(name)).unwrap();
        fs::write(app.directory.join(name).join("index.html"), name).unwrap();
    }
    let error = host.update_patch_details(&app.id, Some("must not save".into()), AppDetails {
        description: Some("must not save".into()), ..Default::default()
    }, AppSettingsPatch::default(), true).await.unwrap_err();
    assert!(error.to_string().contains("multiple runnable projects"));
    let unchanged = host.get(&app.id).await.unwrap();
    assert_eq!(unchanged.name, "保留名称");
    assert!(unchanged.description.is_empty());
    assert!(unchanged.settings.start_command.is_none());
    assert!(unchanged.settings.web_url.is_none());
    assert_eq!(unchanged.state, AppState::Stopped);
    assert!(!app.directory.join("first/.nomifun-static-server.cjs").exists());
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn python_at_the_workspace_root_previews_the_single_nested_html_without_changing_its_command_cwd() {
    let temporary = tempfile::tempdir().unwrap();
    let host = owner(&temporary.path().join("hosting")).await;
    let app = host.create("贪吃蛇".into(), AppMetadata::default()).await.unwrap();
    std::fs::create_dir(app.directory.join("snake game")).unwrap();
    std::fs::write(app.directory.join("snake game/index.html"), "REAL_NESTED_GAME").unwrap();
    std::fs::write(app.directory.join("business-data.json"), "KEEP").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let command = format!("python -m http.server {port} --bind 127.0.0.1");
    let configured = host.update_patch_details(&app.id, None, AppDetails::default(),
        serde_json::from_value(serde_json::json!({"preview_command":command,"start_command":command})).unwrap(), true).await.unwrap();
    assert_eq!(configured.settings.project_directory, ".");
    assert_eq!(configured.settings.web_url.as_deref(), Some(format!("http://127.0.0.1:{port}/snake%20game/").as_str()));
    let running = host.operate(&app.id, AppOperation::Preview).await.unwrap();
    assert_eq!(real_http(running.settings.web_url.as_deref().unwrap()).await, "REAL_NESTED_GAME");
    assert_eq!(running.settings.start_command.as_deref(), Some(command.as_str()));
    assert_eq!(std::fs::read_to_string(app.directory.join("business-data.json")).unwrap(), "KEEP");
    host.shutdown().await.unwrap();
}
