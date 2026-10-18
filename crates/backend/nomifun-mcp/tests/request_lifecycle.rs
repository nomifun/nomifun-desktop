//! Deterministic local protocol peers: no model, external network or credentials.
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{Json, Router, body::Body, extract::State, http::{HeaderValue, StatusCode}, response::{IntoResponse, Response}, routing::post};
use nomifun_mcp::{AnonymousMcpCredentialAuthority, McpCleanupStatus, McpInvocationControl, McpOwner, McpResourceOperation, McpResourceRequest, McpServerBinding, McpServerTransport, McpSetupPolicy, McpToolBinding, McpToolInvocationRequest};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
enum Mode { Silent, Pending, Gated(Arc<Notify>), InitPending, Progress, Pings, Oversized }
#[derive(Clone)]
struct Peer {
    mode: Mode,
    started: Arc<Notify>,
    cancels: Arc<Mutex<Vec<Value>>>,
    session: bool,
    cleanup: StatusCode,
    cleanup_started: Arc<Notify>,
    cleanup_release: Option<Arc<Notify>>,
}
impl Peer {
    fn new(mode: Mode) -> Self {
        Self { mode, started: Arc::new(Notify::new()), cancels: Arc::new(Mutex::new(vec![])), session: false, cleanup: StatusCode::OK,
            cleanup_started:Arc::new(Notify::new()),cleanup_release:None }
    }
}
fn result(id: Value, value: Value) -> Response { Json(json!({"jsonrpc":"2.0","id":id,"result":value})).into_response() }
async fn handle(State(peer): State<Peer>, Json(request): Json<Value>) -> Response {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            if matches!(peer.mode, Mode::InitPending) {
                peer.started.notify_one();
                std::future::pending::<()>().await;
            }
            let mut response = result(id, json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"fixture","version":"1"}}));
            if peer.session { response.headers_mut().insert("mcp-session-id", HeaderValue::from_static("fixture-session")); }
            response
        }
        Some("notifications/initialized") | None => StatusCode::ACCEPTED.into_response(),
        Some("notifications/cancelled") => {
            peer.cancels.lock().unwrap().push(request["params"]["requestId"].clone());
            StatusCode::ACCEPTED.into_response()
        }
        Some("tools/list") => result(id, json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})),
        Some("tools/call") => {
            peer.started.notify_one();
            let output = json!({"content":[{"type":"text","text":"done"}]});
            match peer.mode {
                Mode::Silent | Mode::InitPending => { tokio::time::sleep(Duration::from_millis(250)).await; result(id, output) }
                Mode::Pending => { std::future::pending::<()>().await; unreachable!() }
                Mode::Gated(ref release) => { release.notified().await; result(id,output) }
                Mode::Progress => {
                    let notification = format!("data: {}\n\n", json!({"jsonrpc":"2.0","method":"notifications/message","params":{"data":"x".repeat(2048)}}));
                    let mut body = notification.repeat(6000);
                    // Invalid/opposite-direction cancellation must not kill our call.
                    body.push_str(&format!("data: {}\n\n", json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}})));
                    body.push_str(&format!("data: {}\n\n", json!({"jsonrpc":"2.0","id":id,"result":output})));
                    Response::builder().header("content-type", "text/event-stream").body(Body::from(body)).unwrap()
                }
                Mode::Pings => {
                    let mut body = String::new();
                    for n in 0..80 { body.push_str(&format!("data: {}\n\n", json!({"jsonrpc":"2.0","id":format!("ping-{n}"),"method":"ping"}))); }
                    body.push_str(&format!("data: {}\n\n", json!({"jsonrpc":"2.0","id":id,"result":output})));
                    Response::builder().header("content-type", "text/event-stream").body(Body::from(body)).unwrap()
                }
                Mode::Oversized => {
                    let body = format!("data: {}\n\n", json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"x".repeat(8*1024*1024)}]}}));
                    Response::builder().header("content-type", "text/event-stream").body(Body::from(body)).unwrap()
                }
            }
        }
        Some("resources/list") => {
            peer.started.notify_one();
            tokio::time::sleep(Duration::from_millis(250)).await;
            result(id, json!({"resources":[{"uri":"fixture://example","name":"fixture"}]}))
        }
        Some("resources/read") => result(id, json!({"contents":[{"uri":"fixture://example","text":"done"}]})),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}
async fn close(State(peer): State<Peer>) -> StatusCode {
    if let Some(release)=peer.cleanup_release {
        peer.cleanup_started.notify_one();
        release.notified().await;
    }
    peer.cleanup
}
async fn fixture(peer: Peer) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/mcp", post(handle).delete(close)).with_state(peer);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    (format!("http://{address}/mcp"), task)
}
fn owner() -> McpOwner {
    McpOwner::new(Arc::new(AnonymousMcpCredentialAuthority), reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(1)).build().unwrap())
        .with_setup_policy(McpSetupPolicy::uniform(Duration::from_millis(150)))
}
fn server(url: String) -> McpServerBinding {
    McpServerBinding { server_id:"fixture".into(), server_owner_id:"owner".into(), enabled:true,
        connection_config_ref:"fixture-config".into(), resource_binding_id:"fixture-binding".into(),
        resource_kind:"mcp_server".into(), resource_id:"fixture".into(), resource_owner_id:"owner".into(),
        granted_operations:BTreeSet::from(["connect".into(),"invoke".into(),"read".into()]),
        resource_connection_config_ref:Some("fixture-config".into()), transport:McpServerTransport::Http{url,headers:HashMap::new()} }
}
fn invocation(url: String) -> McpToolInvocationRequest {
    McpToolInvocationRequest { principal_kind:"user".into(),principal_id:"owner".into(),operation_id:"operation-fixture".into(),server:server(url),
        tool:McpToolBinding::new("fixture","fixture-tool","a".repeat(64),json!({"type":"object"}),"echo").unwrap(),arguments:json!({}) }
}

#[tokio::test]
async fn silent_http_headers_and_business_resources_outlive_setup_budget() {
    let (url, task) = fixture(Peer::new(Mode::Silent)).await;
    let outcome = owner().invoke_controlled(invocation(url.clone()), McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert!(outcome.execution.request_dispatched);
    let resource = owner().resource_controlled(McpResourceRequest { principal_kind:"user".into(),principal_id:"owner".into(),operation_id:"resource-fixture".into(),server:server(url),operation:McpResourceOperation::Read{uri:"fixture://example".into()} }, McpInvocationControl::default()).await;
    assert!(resource.result.is_ok(), "{:?}", resource.result);
    task.abort();
}

#[tokio::test]
async fn explicit_cancel_delivers_exact_request_without_waiting_for_final_response() {
    let peer = Peer::new(Mode::Pending);
    let (url, server_task) = fixture(peer.clone()).await;
    let cancellation = CancellationToken::new();
    let control = McpInvocationControl { cancellation:cancellation.clone(), execution_deadline:None };
    let task = tokio::spawn(async move { owner().invoke_controlled(invocation(url),control).await });
    peer.started.notified().await;
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
    assert_eq!(outcome.result.unwrap_err().code(), "MCP_REQUEST_CANCELLED");
    assert!(outcome.execution.request_dispatched && outcome.execution.cancellation_requested);
    assert_eq!(peer.cancels.lock().unwrap().as_slice(), &[json!(3)]);
    assert_eq!(outcome.cleanup.local, McpCleanupStatus::Confirmed);
    server_task.abort();
}

#[tokio::test]
async fn explicit_execution_deadline_is_not_a_setup_deadline() {
    let peer = Peer::new(Mode::Pending);
    let (url, task) = fixture(peer.clone()).await;
    let owner = owner();
    let control = McpInvocationControl { cancellation:CancellationToken::new(),execution_deadline:Some(tokio::time::Instant::now()+Duration::from_secs(1)) };
    let outcome = owner.invoke_controlled(invocation(url),control).await;
    assert_eq!(outcome.result.unwrap_err().code(), "MCP_EXECUTION_DEADLINE_EXCEEDED");
    assert!(outcome.execution.request_dispatched);
    assert_eq!(peer.cancels.lock().unwrap().as_slice(), &[json!(3)]);
    task.abort();
}

#[tokio::test]
async fn normal_notifications_have_no_transaction_lifetime_byte_or_event_limit() {
    let (url, task) = fixture(Peer::new(Mode::Progress)).await;
    let outcome = owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    task.abort();
}

#[tokio::test]
async fn completed_ping_ids_do_not_fill_a_lifetime_request_set() {
    let (url, task) = fixture(Peer::new(Mode::Pings)).await;
    let outcome = owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    task.abort();
}

#[tokio::test]
async fn oversized_single_sse_message_is_still_rejected() {
    let (url, task) = fixture(Peer::new(Mode::Oversized)).await;
    let outcome = owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert_eq!(outcome.result.unwrap_err().code(), "MCP_PROTOCOL_ERROR");
    assert!(outcome.execution.request_dispatched);
    task.abort();
}

#[tokio::test]
async fn observed_business_result_survives_independent_cleanup_failure() {
    let mut peer = Peer::new(Mode::Silent); peer.session=true; peer.cleanup=StatusCode::INTERNAL_SERVER_ERROR;
    let (url, task) = fixture(peer).await;
    let outcome = owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok());
    assert_eq!(outcome.cleanup.local, McpCleanupStatus::Confirmed);
    assert_eq!(outcome.cleanup.remote, McpCleanupStatus::Unproven);
    assert!(outcome.cleanup.error.is_some());
    task.abort();
}

#[tokio::test]
async fn protocol_unsupported_remote_close_is_separate_from_business_result() {
    let mut peer = Peer::new(Mode::Silent); peer.session=true; peer.cleanup=StatusCode::METHOD_NOT_ALLOWED;
    let (url, task) = fixture(peer).await;
    let outcome = owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok());
    assert_eq!(outcome.cleanup.remote, McpCleanupStatus::Unsupported);
    assert!(outcome.cleanup.error.is_none());
    task.abort();
}

#[cfg(unix)]
fn shell_invocation(script: String, args: Vec<String>) -> McpToolInvocationRequest {
    let mut request = invocation("http://unused.test".into());
    request.server.transport = McpServerTransport::Stdio {
        command:"/bin/sh".into(), args:[vec!["-c".into(),script,"mcp-fixture".into()],args].concat(),env:HashMap::new(),
    };
    request
}
#[cfg(unix)]
fn shell_handshake() -> &'static str {
    r#"*'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}' ;;
*'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}' ;;"#
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_cancel_reaches_same_request_and_reaps_the_owned_tree_without_stop_ack() {
    let root = tempfile::tempdir().unwrap();
    let started = root.path().join("started");
    let cancelled = root.path().join("cancelled");
    let script = format!(r#"while IFS= read -r line; do case "$line" in
{}
*'"method":"tools/call"'*) printf '%s' started > "$1"; sleep 60 & ;;
*'"method":"notifications/cancelled"'*) printf '%s' "$line" > "$2" ;;
esac; done"#,shell_handshake());
    let request = shell_invocation(script, vec![started.to_string_lossy().into(),cancelled.to_string_lossy().into()]);
    let control = McpInvocationControl::default();
    let cancellation = control.cancellation.clone();
    let task = tokio::spawn(async move { owner().with_setup_policy(McpSetupPolicy::uniform(Duration::from_secs(2))).invoke_controlled(request,control).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !started.exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(3),task).await.unwrap().unwrap();
    assert_eq!(outcome.result.unwrap_err().code(), "MCP_REQUEST_CANCELLED");
    assert!(outcome.execution.local_process_started && outcome.execution.request_dispatched);
    assert_eq!(outcome.cleanup.local, McpCleanupStatus::Confirmed);
    let notification: Value = serde_json::from_str(&std::fs::read_to_string(cancelled).unwrap()).unwrap();
    assert_eq!(notification["params"]["requestId"],json!(3));
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_progress_has_no_transaction_frame_or_byte_lifetime_limit() {
    let progress = json!({"jsonrpc":"2.0","method":"notifications/message","params":{"data":"x".repeat(6000)}}).to_string();
    let script = format!(r#"while IFS= read -r line; do case "$line" in
{}
*'"method":"tools/call"'*)
  n=0; while [ "$n" -lt 3000 ]; do printf '%s\n' '{}'; n=$((n+1)); done
  sleep 1
  n=0; while [ "$n" -lt 3000 ]; do printf '%s\n' '{}'; n=$((n+1)); done
  printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{{"content":[{{"type":"text","text":"done"}}]}}}}'
  ;;
esac; done"#,shell_handshake(),progress,progress);
    let outcome = owner().with_setup_policy(McpSetupPolicy::uniform(Duration::from_secs(2)))
        .invoke_controlled(shell_invocation(script,vec![]),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok(),"{:?}",outcome.result);
    assert_eq!(outcome.cleanup.local,McpCleanupStatus::Confirmed);
}

#[tokio::test]
async fn cancellation_during_initialize_only_closes_and_never_sends_cancel_notification() {
    let peer = Peer::new(Mode::InitPending);
    let (url, server_task) = fixture(peer.clone()).await;
    let control = McpInvocationControl::default();
    let cancellation = control.cancellation.clone();
    let task = tokio::spawn(async move { owner().invoke_controlled(invocation(url),control).await });
    peer.started.notified().await;
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1),task).await.unwrap().unwrap();
    assert_eq!(outcome.result.unwrap_err().code(),"MCP_REQUEST_CANCELLED");
    assert!(!outcome.execution.request_dispatched);
    assert!(peer.cancels.lock().unwrap().is_empty());
    server_task.abort();
}

#[tokio::test]
async fn connection_probe_reports_its_real_setup_phase_budget() {
    let peer = Peer::new(Mode::InitPending);
    let (url, task) = fixture(peer).await;
    let service = nomifun_mcp::McpConnectionTestService::new(reqwest::Client::builder().no_proxy().build().unwrap())
        .with_setup_policy(McpSetupPolicy::uniform(Duration::from_millis(150)));
    let result = service.test_connection("fixture",&McpServerTransport::Http{url,headers:HashMap::new()}).await;
    assert!(!result.success);
    assert_eq!(result.code,Some(nomifun_api_types::McpConnectionTestErrorCode::Timeout));
    let details = result.details.unwrap();
    assert_eq!(details["phase"],"protocol_setup");
    let seconds=details["timeout_seconds"].as_f64().unwrap();
    assert!(seconds>0.0 && seconds<=0.15,"{details}");
    task.abort();
}

#[tokio::test]
async fn expired_explicit_deadline_is_rejected_before_any_transport_start() {
    let control=McpInvocationControl{cancellation:CancellationToken::new(),execution_deadline:Some(tokio::time::Instant::now())};
    let outcome=owner().invoke_controlled(invocation("http://127.0.0.1:1".into()),control).await;
    assert_eq!(outcome.result.unwrap_err().code(),"MCP_EXECUTION_DEADLINE_EXCEEDED");
    assert!(!outcome.execution.request_dispatched && !outcome.execution.local_process_started);
    assert!(outcome.execution.cancellation_requested);
}

#[tokio::test]
async fn actual_owner_wire_waits_for_a_silent_http_result_after_two_virtual_hours() {
    let release=Arc::new(Notify::new());
    let peer=Peer::new(Mode::Gated(release.clone()));
    let (url,server_task)=fixture(peer.clone()).await;
    let task=tokio::spawn(async move { owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await });
    peer.started.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2*60*60)).await;
    for _ in 0..4 { tokio::task::yield_now().await; }
    assert!(!task.is_finished(),"an implicit owner or HTTP response deadline truncated the tool");
    release.notify_one();
    tokio::time::resume();
    let outcome=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap();
    assert!(outcome.result.is_ok(),"{:?}",outcome.result);
    server_task.abort();
}

#[tokio::test]
async fn cleanup_of_the_exact_expired_session_is_confirmed_without_reinitializing() {
    let mut peer=Peer::new(Mode::Silent);peer.session=true;peer.cleanup=StatusCode::NOT_FOUND;
    let (url,task)=fixture(peer).await;
    let outcome=owner().invoke_controlled(invocation(url),McpInvocationControl::default()).await;
    assert!(outcome.result.is_ok());
    assert_eq!(outcome.cleanup.remote,McpCleanupStatus::Confirmed);
    assert!(outcome.cleanup.error.is_none());
    task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_cancel_of_partial_request_write_closes_without_appending_a_corrupt_frame() {
    let root=tempfile::tempdir().unwrap();
    let started=root.path().join("partial-frame");
    let script=format!(r#"while IFS= read -r line; do case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-03-26","capabilities":{{}},"serverInfo":{{"name":"fixture","version":"1"}}}}}}' ;;
*'"method":"tools/list"'*)
  printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"echo","inputSchema":{{"type":"object"}}}}]}}}}'
  dd bs=1 count=1 > /dev/null 2>&1
  printf '%s' partial > "$1"
  sleep 60
  ;;
esac; done"#);
    let mut request=shell_invocation(script,vec![started.to_string_lossy().into()]);
    request.arguments=json!({"payload":"x".repeat(700*1024)});
    let control=McpInvocationControl::default();let cancellation=control.cancellation.clone();
    let task=tokio::spawn(async move {owner().with_setup_policy(McpSetupPolicy::uniform(Duration::from_secs(2))).invoke_controlled(request,control).await});
    tokio::time::timeout(Duration::from_secs(3),async{while !started.exists(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    cancellation.cancel();
    let outcome=tokio::time::timeout(Duration::from_millis(1500),task).await.expect("partial write must skip the notification delivery wait").unwrap();
    assert_eq!(outcome.result.unwrap_err().code(),"MCP_REQUEST_CANCELLED");
    assert!(outcome.execution.request_dispatched);
    assert_eq!(outcome.cleanup.local,McpCleanupStatus::Confirmed);
}

#[tokio::test]
async fn observed_result_wins_over_cancellation_requested_during_cleanup() {
    let release=Arc::new(Notify::new());
    let mut peer=Peer::new(Mode::Silent);peer.session=true;peer.cleanup_release=Some(release.clone());
    let (url,server_task)=fixture(peer.clone()).await;
    let control=McpInvocationControl::default();let cancellation=control.cancellation.clone();
    let task=tokio::spawn(async move { owner().invoke_controlled(invocation(url),control).await });
    peer.cleanup_started.notified().await;
    cancellation.cancel();
    release.notify_one();
    let outcome=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap().unwrap();
    assert!(outcome.result.is_ok(),"late cancellation erased an observed business result");
    assert!(outcome.execution.cancellation_requested);
    assert_eq!(outcome.cleanup.remote,McpCleanupStatus::Confirmed);
    assert!(peer.cancels.lock().unwrap().is_empty(),"the completed request must not be cancelled");
    server_task.abort();
}
