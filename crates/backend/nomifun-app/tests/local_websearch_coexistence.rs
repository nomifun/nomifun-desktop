//! Real local search and a loopback Responses protocol fixture in one AgentSession.
//! This verifies Nomi routing/citations, not the remote vendor's search service.
use super::*;

const NATIVE_URL: &str = "https://example.com/native-search";
#[derive(Default)]
struct Coexistence {
    calls: AtomicUsize,
    native_calls: AtomicUsize,
    routing_calls: AtomicUsize,
    local: Mutex<Option<Value>>,
    native: Mutex<Option<Value>>,
    failure: Mutex<Option<String>>,
}

fn result(body: &Value, call: &str) -> Result<Value, String> {
    let row = body["input"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .rev()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
        })
        .ok_or("Responses tool output missing")?;
    serde_json::from_str(
        row["output"]
            .as_str()
            .ok_or("Responses tool output not text")?,
    )
    .map_err(|_| "Responses tool output not JSON".into())
}
fn response(
    body: &Value,
    step: usize,
    call: Option<(&str, Value)>,
    text: &str,
) -> Result<Response, String> {
    let store = body["store"]
        .as_bool()
        .ok_or("Responses request omitted store")?;
    let id = format!("resp_coexist_{step}");
    let item = if let Some((name, arguments)) = call {
        json!({"type":"function_call","id":format!("fc_coexist_{step}"),"call_id":format!("coexist_{step}"),"name":name,"arguments":arguments.to_string(),"status":"completed"})
    } else {
        json!({"type":"message","id":"msg_coexist","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})
    };
    let mut added = item.clone();
    added["status"] = json!("in_progress");
    if added["type"] == "function_call" {
        added["arguments"] = json!("");
    } else {
        added["content"] = json!([]);
    }
    let events = [
        json!({"type":"response.created","response":{"id":id,"status":"in_progress","store":store,"output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":added}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":id,"status":"completed","store":store,"output":[item],"usage":null}}),
    ];
    let wire = events
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    Ok(([("content-type", "text/event-stream")], wire).into_response())
}

fn reply(state: &Coexistence, body: &Value) -> Result<Response, String> {
    if body["instructions"]
        .as_str()
        .is_some_and(|text| text.starts_with("You are a routing classifier, not an assistant."))
    {
        if !body["tools"].is_null()
            || body["max_output_tokens"] != 96
            || body["store"] != false
            || state.routing_calls.fetch_add(1, Ordering::SeqCst) != 0
        {
            return Err("Unexpected image-intent routing request".into());
        }
        return response(body, 99, None, r#"{"intent":"none"}"#);
    }
    if body["tools"] == json!([{"type":"web_search"}]) {
        if state.native_calls.fetch_add(1, Ordering::SeqCst) != 0
            || body["input"] != QUERY
            || body["tool_choice"] != json!({"type":"web_search"})
            || body["include"] != json!(["web_search_call.action.sources"])
            || body["store"] != false
        {
            return Err("Vendor search did not use its exact native request contract".into());
        }
        return Ok(Json(json!({"id":"native_search_fixture","status":"completed","output":[
            {"type":"web_search_call","id":"native_call","status":"completed","action":{"type":"search","query":QUERY,"sources":[{"type":"url","id":"native_source","title":"Native source fixture","url":NATIVE_URL}]}},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"Native protocol fixture answer","annotations":[]}]}
        ]})).into_response());
    }
    let tools = body["tools"].as_array().ok_or("Model tools missing")?;
    if tools.iter().any(|tool| tool["type"] != "function") {
        return Err("The normal model request implicitly enabled a native tool".into());
    }
    let names: Vec<_> = tools
        .iter()
        .filter_map(|tool| {
            if tool["type"] == "function" {
                tool["name"].as_str()
            } else {
                None
            }
        })
        .collect();
    for name in ["web_search", "nomi_local_websearch", "citation_render"] {
        if names.iter().filter(|candidate| **candidate == name).count() != 1 {
            return Err(format!("Tool identity is absent or duplicated: {name}"));
        }
    }
    if names
        .iter()
        .any(|name| matches!(*name, "Browser" | "nomi_system_browser"))
    {
        return Err("Searches implicitly enabled browser interaction".into());
    }
    let step = state.calls.fetch_add(1, Ordering::SeqCst);
    let call = match step {
        0 => Some(("nomi_local_websearch", json!({"query":QUERY,"limit":3}))),
        1 => {
            if state.native_calls.load(Ordering::SeqCst) != 0 {
                return Err("Local search invoked the vendor provider".into());
            }
            let local = result(body, "coexist_0")?;
            let rows = local["results"]
                .as_array()
                .ok_or_else(|| format!("Local search failed: {local}"))?;
            if local["provider"]["id"] != "nomi.local.browser"
                || rows.is_empty()
                || rows.len() > 3
                || rows.iter().any(|row| {
                    !row["citation_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("nomi-local-search-"))
                })
            {
                return Err("Local results lost their provider or citation namespace".into());
            }
            *state.local.lock().unwrap() = Some(local);
            Some(("web_search", json!({"query":QUERY,"limit":1})))
        }
        2 => {
            let native = result(body, "coexist_1")?;
            if state.native_calls.load(Ordering::SeqCst) != 1
                || native["provider"] != "openai.responses.web_search"
                || native["results"][0]["url"] != NATIVE_URL
                || !native["results"][0]["citation_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("web-search-"))
            {
                return Err(
                    "Native search was replaced by local search or returned a wrong citation"
                        .into(),
                );
            }
            let local = state
                .local
                .lock()
                .unwrap()
                .clone()
                .ok_or("Local result missing")?;
            let ids = json!([
                local["results"][0]["citation_id"],
                native["results"][0]["citation_id"]
            ]);
            *state.native.lock().unwrap() = Some(native);
            Some(("citation_render", json!({"citation_ids":ids})))
        }
        3 => {
            let rendered = result(body, "coexist_2")?;
            let local = state.local.lock().unwrap();
            let native = state.native.lock().unwrap();
            let expected = [
                &local.as_ref().unwrap()["results"][0],
                &native.as_ref().unwrap()["results"][0],
            ];
            if rendered["citations"].as_array().map(Vec::len) != Some(2) {
                return Err("Mixed citations were not rendered".into());
            }
            for (index, source) in expected.iter().enumerate() {
                if rendered["citations"][index]["citation_id"] != source["citation_id"]
                    || rendered["citations"][index]["url"] != source["url"]
                {
                    return Err("Mixed citations crossed provider identities".into());
                }
            }
            None
        }
        _ => return Err("Unexpected extra model request".into()),
    };
    response(body, step, call, "BOTH_SEARCHES_AND_CITATIONS_DONE")
}
async fn completion(State(state): State<Arc<Coexistence>>, Json(body): Json<Value>) -> Response {
    match reply(&state, &body) {
        Ok(reply) => reply,
        Err(error) => {
            *state.failure.lock().unwrap() = Some(error);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[tokio::test]
#[ignore = "requires NOMIFUN_CHROME_BINARY and public local search; vendor Responses is loopback-only"]
async fn both_searches_execute_and_render_separate_citations_in_one_session() {
    let chrome: std::path::PathBuf = std::env::var_os("NOMIFUN_CHROME_BINARY")
        .expect("explicit test Chrome")
        .into();
    let product = nomi_browser_engine::headless_page::probe_runtime(chrome.clone())
        .await
        .unwrap();
    let mut host = DesktopHostServices::default();
    host.set_browser_release(chrome, product).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(Coexistence::default());
    let stop = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/v1/responses", post(completion))
        .with_state(state.clone());
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
    let result=std::panic::AssertUnwindSafe(async{
        let provider=api(&app,"POST","/api/providers",json!({"platform":"openai","name":"Coexistence protocol fixture","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"search-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming","web_search"],"protocol":"openai.responses","connection_role":"default","output_limit":4096}]}})).await?;
        let provider=provider["provider_id"].as_str().ok_or("Provider missing")?;
        let editor=api(&app,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Search coexistence","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"search-fixture"}})).await?;
        let preset=editor["preset"]["preset_id"].as_str().ok_or("Preset missing")?;
        let catalog=api(&app,"GET","/api/capabilities",Value::Null).await?; let catalog=catalog.as_array().ok_or("Catalog missing")?;
        let mut selected=vec![];
        for id in ["nomi_local_websearch","web.search","citation.render"] {
            let item=catalog.iter().find(|item|item["capability"]["id"]==id&&item["materialization_state"]=="materialized").ok_or_else(||format!("Unavailable capability {id}"))?;
            selected.push(json!({"capability":item["capability"],"action_allowlist":[]}));
        }
        let mut draft=editor["draft"].clone(); draft["document"]["enabled_capabilities"]=json!(selected);draft["document"]["skill_bindings"]=json!([]);
        api(&app,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"search coexistence conformance"})).await?;
        let session=api(&app,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"Search coexistence","model":{"provider_id":provider,"model":"search-fixture"}})).await?;
        let id=session["agent_session_id"].as_str().ok_or("Session missing")?;
        api(&app,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"input":{"content":"Use both selected search providers, then render one citation from each."},"idempotency_key":uuid::Uuid::new_v4().to_string()})).await?;
        tokio::time::timeout(Duration::from_secs(75),async{
            loop {
                if let Some(error)=state.failure.lock().unwrap().clone(){return Err(error);}
                let conversation=api(&app,"GET",&format!("/api/conversations/{id}"),Value::Null).await?;
                if conversation["status"]=="finished" {break;}
                if matches!(conversation["status"].as_str(),Some("error"|"cancelled")){return Err("Search coexistence turn failed".into());}
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if state.calls.load(Ordering::SeqCst)!=4||state.native_calls.load(Ordering::SeqCst)!=1{return Err("Incomplete provider roundtrip".into());}
            Ok::<(),String>(())
        }).await.map_err(|_|"Search coexistence timeout".to_owned())?
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
        "SEARCH_COEXISTENCE_MAIN_APP_PASS model_calls=4 routing_calls={} local_browser=real native_provider=loopback_protocol separate_tools=true mixed_citations=true cleanup=true",
        state.routing_calls.load(Ordering::SeqCst)
    );
}
