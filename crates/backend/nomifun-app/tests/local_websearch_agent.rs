//! Workbench HTTP selection -> ordinary Chat model -> real isolated search.
//! No native-browser host or system-browser connection is supplied. The model
//! endpoint is deterministic; the search provider and public results are real.
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::FutureExt;
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const QUERY: &str = "Tauri WebView2 documentation";
#[path = "local_websearch_coexistence.rs"]
mod coexistence;
#[derive(Default)]
struct Model {
    calls: AtomicUsize,
    result: Mutex<Option<Value>>,
    failure: Mutex<Option<String>>,
}

fn model_reply(model: &Model, body: &Value, step: usize) -> Result<(Value, &'static str), String> {
    let tools = body["tools"].as_array().ok_or("model tools missing")?;
    let names: Vec<_> = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    if names
        .iter()
        .filter(|name| **name == "nomi_local_websearch")
        .count()
        != 1
    {
        return Err("the exact local search tool was not registered once".into());
    }
    if names
        .iter()
        .any(|name| matches!(*name, "web_search" | "Browser" | "nomi_system_browser"))
    {
        return Err("local search selection enabled another browser/search tool".into());
    }
    match step {
        0 => Ok((
            json!({"role":"assistant","tool_calls":[{"index":0,"id":"local-search-1","type":"function","function":{"name":"nomi_local_websearch","arguments":json!({"query":QUERY,"limit":3}).to_string()}}]}),
            "tool_calls",
        )),
        1 => {
            let message = body["messages"]
                .as_array()
                .ok_or("messages missing")?
                .iter()
                .rev()
                .find(|message| message["role"] == "tool")
                .ok_or("real tool result missing")?;
            let result: Value =
                serde_json::from_str(message["content"].as_str().ok_or("tool content missing")?)
                    .map_err(|_| "tool result is not JSON")?;
            if result["query"] != QUERY || result["provider"]["id"] != "nomi.local.browser" {
                return Err(format!("real local search did not succeed: {result}"));
            }
            let rows = result["results"]
                .as_array()
                .ok_or("search results missing")?;
            if rows.is_empty() || rows.len() > 3 {
                return Err("public search did not return bounded nonempty results".into());
            }
            for row in rows {
                let url = url::Url::parse(row["url"].as_str().ok_or("source URL missing")?)
                    .map_err(|_| "invalid source URL")?;
                if !matches!(url.scheme(), "http" | "https")
                    || row["title"].as_str().is_none_or(str::is_empty)
                {
                    return Err("source title or URL invalid".into());
                }
                let citation = row["citation_id"].as_str().ok_or("citation missing")?;
                let digest = citation
                    .strip_prefix("nomi-local-search-")
                    .ok_or("wrong citation namespace")?;
                if digest.len() != 32 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err("invalid citation ID".into());
                }
            }
            let reply = format!(
                "LOCAL_WEBSEARCH_DONE {}",
                rows[0]["citation_id"].as_str().unwrap()
            );
            *model.result.lock().unwrap() = Some(result);
            Ok((json!({"role":"assistant","content":reply}), "stop"))
        }
        _ => Err("unexpected extra model call".into()),
    }
}

async fn completion(State(model): State<Arc<Model>>, Json(body): Json<Value>) -> Response {
    let step = model.calls.fetch_add(1, Ordering::SeqCst);
    let (delta, finish) = match model_reply(&model, &body, step) {
        Ok(reply) => reply,
        Err(error) => {
            *model.failure.lock().unwrap() = Some(error);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({"id":format!("local-search-{step}"),"object":"chat.completion.chunk","created":1,"model":"search-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
    };
    (
        [("content-type", "text/event-stream")],
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(delta, None),
            chunk(json!({}), Some(finish))
        ),
    )
        .into_response()
}

async fn api(app: &DesktopServer, method: &str, path: &str, body: Value) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
        .request(
            method.parse().unwrap(),
            format!("http://127.0.0.1:{}{path}", app.loopback_port()),
        )
        .header("x-nomi-local-trust", app.local_trust_secret())
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("{method} {path}: {status} {value}"));
    }
    Ok(value["data"].clone())
}

#[tokio::test]
#[ignore = "requires NOMIFUN_CHROME_BINARY and public network; owns an isolated backend and search browser"]
async fn model_without_native_search_uses_selected_local_browser_search() {
    let chrome = std::env::var_os("NOMIFUN_CHROME_BINARY")
        .expect("explicit test Chrome")
        .into();
    let product = nomi_browser_engine::headless_page::probe_runtime(chrome)
        .await
        .unwrap();
    let mut host = DesktopHostServices::default();
    host.set_browser_release(
        std::env::var_os("NOMIFUN_CHROME_BINARY").unwrap().into(),
        product,
    )
    .await
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let model = Arc::new(Model::default());
    let stop = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/v1/chat/completions", post(completion))
        .with_state(model.clone());
    let server_stop = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
    });
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".into(),
        port: 0,
        data_dir: root.path().join("data"),
        work_dir: None,
        app_version: env!("CARGO_PKG_VERSION").into(),
        local: true,
        log_dir: Some(root.path().join("logs")),
        log_level: Some("off".into()),
        command: None,
    };
    let (app, keep_alive) = DesktopServer::start_with_outcome(&cli, "", None, None, None, host)
        .await
        .unwrap();
    let result = std::panic::AssertUnwindSafe(async {
        // Deliberately no native web-search trait and no Responses protocol.
        let provider = api(&app, "POST", "/api/providers", json!({"platform":"custom","name":"Local search fixture","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"search-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or("provider id missing")?;
        let editor = api(&app, "POST", "/api/agent-presets/from-template/chat.minimal", json!({"reuse_existing":false,"display_name":"Local search fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"search-fixture"}})).await?;
        let preset = editor["preset"]["preset_id"].as_str().ok_or("preset id missing")?;
        let catalog = api(&app, "GET", "/api/capabilities", Value::Null).await?;
        let entries = catalog.as_array().ok_or("catalog shape")?;
        let capability = entries.iter().find(|item| item["capability"]["id"] == "nomi_local_websearch").ok_or("local search missing")?;
        if capability["materialization_state"] != "materialized" || !entries.iter().any(|item| item["capability"]["id"] == "web.search") {
            return Err("local and vendor search catalog identities are not independently available".into());
        }
        let mut draft = editor["draft"].clone();
        draft["document"]["enabled_capabilities"] = json!([{"capability":capability["capability"],"action_allowlist":[]}]);
        draft["document"]["skill_bindings"] = json!([]);
        api(&app, "POST", &format!("/api/agent-presets/{preset}/revisions"), json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"local search conformance"})).await?;
        let session = api(&app, "POST", "/api/agent-sessions", json!({"preset_id":preset,"title":"Local search conformance","model":{"provider_id":provider,"model":"search-fixture"}})).await?;
        let id = session["agent_session_id"].as_str().ok_or("session id missing")?;
        api(&app, "POST", &format!("/api/agent-sessions/{id}/turns"), json!({"input":{"content":"Search public documentation with the selected local search tool."},"idempotency_key":uuid::Uuid::new_v4().to_string()})).await?;
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if let Some(error) = model.failure.lock().unwrap().clone() { return Err(error); }
                let conversation = api(&app, "GET", &format!("/api/conversations/{id}"), Value::Null).await?;
                if conversation["status"] == "finished" { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if model.calls.load(Ordering::SeqCst) != 2 || model.result.lock().unwrap().is_none() {
                return Err("conversation ended without the real search/model roundtrip".into());
            }
            Ok::<(), String>(())
        }).await.map_err(|_| "local search conversation timeout".to_owned())?
    }).catch_unwind().await;
    let cleanup = app.shutdown_all().await;
    stop.cancel();
    server.await.unwrap().unwrap();
    drop(app);
    drop(keep_alive);
    cleanup.unwrap();
    root.close().unwrap();
    match result {
        Ok(result) => result.unwrap(),
        Err(panic) => std::panic::resume_unwind(panic),
    }
    println!(
        "LOCAL_WEBSEARCH_MAIN_APP_PASS model_calls=2 native_search_trait=false exact_tool=true public_results=true citations=true temporary_backend_cleanup=true"
    );
}
