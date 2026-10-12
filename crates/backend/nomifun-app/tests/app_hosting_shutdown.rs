//! Real Desktop HTTP drain during a long-lived preparation command.
use std::time::Duration;
use clap::Parser as _;
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};

async fn api(server: &DesktopServer, method: &str, path: &str, body: Value) -> Value {
    let response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().unwrap()
        .request(method.parse().unwrap(), format!("http://127.0.0.1:{}{path}", server.loopback_port()))
        .header("x-nomi-local-trust", server.local_trust_secret()).json(&body).send().await.unwrap();
    let status = response.status();
    let value: Value = response.json().await.unwrap();
    assert!(status.is_success(), "{path}: {status}: {value}");
    value["data"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desktop_shutdown_cancels_install_before_waiting_for_its_http_request() {
    assert!(nomi_process_runtime::resolve_command_path("node").is_some(), "real Node is required");
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let work = root.path().join("work");
    let source = root.path().join("original");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("install.cjs"), "require('fs').writeFileSync('preparation.pid',String(process.pid));setInterval(()=>{},1000)").unwrap();
    std::fs::write(source.join("server.cjs"), "require('fs').writeFileSync('must-not-start','yes');setInterval(()=>{},1000)").unwrap();
    let cli = nomifun_app::cli::Cli::parse_from(["hosting-shutdown-test", "--data-dir", data.to_str().unwrap(), "--work-dir", work.to_str().unwrap()]);
    let merged_path = std::env::var("PATH").unwrap_or_default();
    let (server, keep_alive) = tokio::time::timeout(Duration::from_secs(45),
        DesktopServer::start_with_outcome(&cli, &merged_path, None, None, None, DesktopHostServices::default())).await.unwrap().unwrap();
    let app = api(&server, "POST", "/api/apps/import", json!({"source":source,"name":"slow application","import_kind":"source"})).await;
    let id = app["id"].as_str().unwrap();
    let directory = std::path::PathBuf::from(app["directory"].as_str().unwrap());
    let mut settings = app["settings"].clone();
    settings["install_command"] = "node install.cjs".into();
    settings["build_command"] = Value::Null;
    settings["start_command"] = "node server.cjs".into();
    api(&server, "PUT", &format!("/api/apps/{id}"), json!({"name":"slow application","settings":settings})).await;
    let request_url = format!("http://127.0.0.1:{}/api/apps/{id}/operate", server.loopback_port());
    let trust = server.local_trust_secret().to_owned();
    let pending_request = tokio::spawn(async move {
        reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build().unwrap()
            .post(request_url).header("x-nomi-local-trust", trust).json(&json!({"operation":"use"})).send().await
    });
    tokio::time::timeout(Duration::from_secs(8), async {
        while !directory.join("preparation.pid").exists() { tokio::time::sleep(Duration::from_millis(25)).await; }
    }).await.unwrap();
    let pid: u32 = std::fs::read_to_string(directory.join("preparation.pid")).unwrap().parse().unwrap();
    assert!(nomi_process_runtime::probe_process_identity(pid).unwrap().is_some());
    tokio::time::timeout(Duration::from_secs(20), server.shutdown_all()).await
        .expect("HTTP drain must not wait for the 20-minute install timeout").unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(3), pending_request).await.unwrap().unwrap();
    if let Ok(response) = outcome { assert!(!response.status().is_success(), "cancelled preparation must not report success"); }
    assert!(nomi_process_runtime::probe_process_identity(pid).unwrap().is_none());
    assert!(!directory.join("must-not-start").exists());
    let stored: Value = serde_json::from_slice(&std::fs::read(data.join("app-hosting/registry").join(format!("{id}.json"))).unwrap()).unwrap();
    assert_eq!(stored["record"]["state"], "stopped");
    assert!(stored["record"]["operation"].is_null());
    assert!(source.join("install.cjs").exists());
    drop(keep_alive);
}
