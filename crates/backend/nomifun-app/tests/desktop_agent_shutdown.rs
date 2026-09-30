//! Formal desktop host shutdown while a model stream follows a committed write.
//! The local provider controls timing; this is independent of renderer acceptance.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;

use axum::{Json, Router, routing::post};
use clap::Parser as _;
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

struct WaitingStream(Arc<AtomicUsize>);
impl Drop for WaitingStream {
    fn drop(&mut self) { self.0.fetch_sub(1,Ordering::SeqCst); }
}

fn frame(delta:Value,finish:Option<&str>)->String {
    format!("data: {}\n\n",json!({"id":"shutdown-fixture","choices":[{
        "index":0,"delta":delta,"finish_reason":finish}]}))
}

async fn api(server:&DesktopServer,method:&str,path:&str,body:Value)->Value {
    let response=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(15)).build().unwrap()
        .request(method.parse().unwrap(),format!("http://127.0.0.1:{}{path}",server.loopback_port()))
        .header("x-nomi-local-trust",server.local_trust_secret()).json(&body).send().await.unwrap();
    let status=response.status();
    let value:Value=response.json().await.unwrap();
    assert!(status.is_success(),"{path}: {status}: {value}");
    value["data"].clone()
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn desktop_shutdown_drains_a_waiting_model_before_storage_close_and_preserves_one_write() {
    let directory=tempfile::tempdir().unwrap();
    let root=std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR")
        .map(std::path::PathBuf::from).unwrap_or_else(||directory.path().to_path_buf());
    let data=root.join("data");
    let work=root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let calls=Arc::new(AtomicUsize::new(0));
    let active=Arc::new(AtomicUsize::new(0));
    let waiting=Arc::new(tokio::sync::Notify::new());
    let handler_calls=Arc::clone(&calls);
    let handler_active=Arc::clone(&active);
    let handler_waiting=Arc::clone(&waiting);
    let provider=Router::new().route("/v1/chat/completions",post(move |Json(body):Json<Value>| {
        let calls=Arc::clone(&handler_calls);
        let active=Arc::clone(&handler_active);
        let waiting=Arc::clone(&handler_waiting);
        async move {
            let call=calls.fetch_add(1,Ordering::SeqCst);
            if call==0 {
                let data=format!("{}{}data: [DONE]\n\n",frame(json!({"tool_calls":[{
                    "index":0,"id":"write-once","type":"function","function":{
                    "name":"write_file","arguments":json!({"path":"result.txt","content":"SHUTDOWN_WRITE_ONCE"}).to_string()}
                }]}),None),frame(json!({}),Some("tool_calls")));
                return axum::response::Response::builder().header("content-type","text/event-stream")
                    .body(axum::body::Body::from(data)).unwrap();
            }
            assert_eq!(call,1,"shutdown must not retry the model or physical write");
            assert!(body["messages"].as_array().unwrap().iter().any(|message|message["role"]=="tool"));
            active.fetch_add(1,Ordering::SeqCst);
            let stream=futures_util::stream::unfold((WaitingStream(active),true),|(guard,first)|async move {
                let data=if first {frame(json!({"role":"assistant","content":"Waiting after the committed write"}),None)}
                    else {tokio::time::sleep(Duration::from_millis(25)).await; ": keepalive\n\n".into()};
                Some((Ok::<_,std::io::Error>(data),(guard,false)))
            });
            waiting.notify_one();
            axum::response::Response::builder().header("content-type","text/event-stream")
                .body(axum::body::Body::from_stream(stream)).unwrap()
        }
    }));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address=listener.local_addr().unwrap();
    let stop=CancellationToken::new();
    let provider_stop=stop.clone();
    let provider_task=tokio::spawn(async move {axum::serve(listener,provider)
        .with_graceful_shutdown(provider_stop.cancelled_owned()).await.unwrap()});
    let cli=nomifun_app::cli::Cli::parse_from(["desktop-shutdown-test","--data-dir",data.to_str().unwrap(),
        "--work-dir",work.to_str().unwrap()]);
    let (server,keep_alive)=tokio::time::timeout(Duration::from_secs(30),
        DesktopServer::start_with_outcome(&cli,"",None,None,None,
            DesktopHostServices::default())).await.expect("desktop startup must be bounded").unwrap();
    let provider=api(&server,"POST","/api/providers",json!({"platform":"custom","name":"shutdown fixture",
        "base_url":format!("http://{address}/v1"),"auth_scheme":"bearer",
        "credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,
        "initial_model":{"model":"shutdown-fixture","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await;
    let model=json!({"provider_id":provider["provider_id"],"model":"shutdown-fixture"});
    let editor=api(&server,"POST","/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Shutdown fixture","model":model})).await;
    let preset=editor["preset"]["preset_id"].as_str().unwrap();
    let mut draft=editor["draft"].clone();
    draft["document"]["enabled_capabilities"]=json!([{"capability":{"id":"workspace.files"},
        "action_allowlist":["workspace.files/read","workspace.files/write"]}]);
    api(&server,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({
        "expected_current_revision":draft["current_revision"],"draft":draft,"reason":"bounded shutdown regression"})).await;
    let session=api(&server,"POST","/api/agent-sessions",json!({"preset_id":preset,"model":model,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await;
    let id=session["agent_session_id"].as_str().unwrap();
    api(&server,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"one-shutdown-turn",
        "input":{"content":"Create result.txt containing SHUTDOWN_WRITE_ONCE, then wait for the next check. Write only once."}})).await;
    tokio::time::timeout(Duration::from_secs(15),waiting.notified()).await.unwrap();
    assert_eq!(active.load(Ordering::SeqCst),1);
    assert_eq!(calls.load(Ordering::SeqCst),2);
    let observer=sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(data.join("nomifun-backend.db"))
            .read_only(true).busy_timeout(Duration::from_secs(5))).await.unwrap();
    let before:String=sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
        .bind(id).fetch_one(&observer).await.unwrap();
    assert_eq!(before,"running");
    let file=work.join("conversations").join(id).join("result.txt");
    assert_eq!(std::fs::read(&file).unwrap(),b"SHUTDOWN_WRITE_ONCE");
    tokio::time::timeout(Duration::from_secs(20),server.shutdown_all()).await
        .expect("formal desktop shutdown must be bounded").unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        while active.load(Ordering::SeqCst)!=0 {tokio::time::sleep(Duration::from_millis(10)).await;}
    }).await.expect("the remote model body must be dropped after shutdown");
    let turn:String=sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
        .bind(id).fetch_one(&observer).await.unwrap();
    assert_eq!(turn,"cancelled");
    let terminal_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'")
        .bind(id).fetch_one(&observer).await.unwrap();
    assert_eq!(terminal_count,1);
    let effects:Vec<String>=sqlx::query_scalar("SELECT state FROM agent_effects WHERE session_id=?")
        .bind(id).fetch_all(&observer).await.unwrap();
    assert_eq!(effects,vec!["returned"]);
    assert_eq!(std::fs::read(&file).unwrap(),b"SHUTDOWN_WRITE_ONCE");
    assert_eq!(calls.load(Ordering::SeqCst),2);
    let last_seq:i64=sqlx::query_scalar("SELECT last_seq FROM agent_session_heads WHERE session_id=?")
        .bind(id).fetch_one(&observer).await.unwrap();
    server.shutdown_all().await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT last_seq FROM agent_session_heads WHERE session_id=?")
        .bind(id).fetch_one(&observer).await.unwrap(),last_seq);
    assert!(reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap().get(format!(
        "http://127.0.0.1:{}/api/system/version",server.loopback_port())).send().await.is_err());
    observer.close().await;
    drop(server);drop(keep_alive);
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(5),provider_task).await.unwrap().unwrap();
}
