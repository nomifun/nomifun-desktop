//! Explicit local reproduction using the exact configured route of a reported
//! Session. Source storage is read-only; only an isolated fixture receives a
//! copy of that provider connection. Never print credential material, model
//! reasoning, source transcripts or the source database. No automatic opt-in.
use super::*;
use sqlx::{Connection, Row};

#[tokio::test(flavor="multi_thread", worker_threads=4)]
#[ignore = "uses an explicitly selected local provider and incurs model usage"]
async fn configured_provider_completes_original_task_in_isolated_workspace() {
    run_source_case(false).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
#[ignore = "read-only model protocol probes using the selected local provider"]
async fn configured_provider_wire_shapes_without_executing_tools() {
    run_source_case(true).await;
}

async fn run_source_case(probe_only: bool) {
    let source = std::path::PathBuf::from(std::env::var("NOMIFUN_RELIABILITY_SOURCE_DATA_DIR").expect("explicit source data directory"));
    let source_session = std::env::var("NOMIFUN_RELIABILITY_SOURCE_SESSION").expect("explicit source Session");
    let artifact_root = std::path::PathBuf::from(std::env::var("NOMIFUN_RELIABILITY_OUTPUT_DIR").expect("new artifact directory"));
    std::fs::create_dir(&artifact_root).expect("artifact directory must not already exist");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(source.join("nomifun-backend.db")).read_only(true);
    let mut original = sqlx::SqliteConnection::connect_with(&options).await.expect("read-only source connection");
    let binding: String = sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=?")
        .bind(&source_session).fetch_one(&mut original).await.unwrap();
    let binding: Value = serde_json::from_str(&binding).unwrap();
    let revision: String = sqlx::query_scalar("SELECT payload_json FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(binding["preset_revision_ref"]["preset_id"].as_str().unwrap())
        .bind(binding["preset_revision_ref"]["revision"].as_i64().unwrap()).fetch_one(&mut original).await.unwrap();
    let revision: Value = serde_json::from_str(&revision).unwrap();
    let candidate = &revision["chat_route_records"]["agent_chat"]["primary"];
    let provider_id = candidate["provider_id"].as_str().unwrap();
    let source_model = candidate["model"].as_str().unwrap().to_owned();
    let role = candidate["connection_config_ref"].as_str().unwrap();
    let platform: String = sqlx::query_scalar("SELECT platform FROM providers WHERE provider_id=?")
        .bind(provider_id).fetch_one(&mut original).await.unwrap();
    let row = if role == "default" {
        sqlx::query("SELECT base_url,auth_scheme,credentials_encrypted FROM providers WHERE provider_id=?")
            .bind(provider_id).fetch_one(&mut original).await.unwrap()
    } else {
        sqlx::query("SELECT base_url,auth_scheme,credentials_encrypted FROM provider_connections WHERE provider_id=? AND role=?")
            .bind(provider_id).bind(role).fetch_one(&mut original).await.unwrap()
    };
    let capability = sqlx::query("SELECT protocol,traits,provider_params,context_limit,output_limit FROM provider_model_capabilities WHERE provider_id=? AND model=? AND task='chat'")
        .bind(provider_id).bind(&source_model).fetch_one(&mut original).await.unwrap();
    // Read only the original accepted user input, not a derived title or the
    // failed assistant's suggested workaround.
    let input: String = sqlx::query_scalar("SELECT json_extract(inline_json,'$.content') FROM agent_events WHERE session_id=? AND kind='message/user-accepted' ORDER BY seq LIMIT 1")
        .bind(&source_session).fetch_one(&mut original).await.unwrap();
    original.close().await.unwrap();
    let encoded_key = std::fs::read_to_string(source.join("encryption_key")).unwrap();
    let encoded_key = encoded_key.trim();
    assert_eq!(encoded_key.len(),64,"source key shape");
    let mut key = [0u8;32];
    for (i,byte) in key.iter_mut().enumerate() { *byte=u8::from_str_radix(&encoded_key[i*2..i*2+2],16).expect("key encoding"); }
    let cipher: String = row.get("credentials_encrypted");
    let plaintext = nomifun_common::decrypt_string(&cipher,&key).expect("credential decryption");
    key.fill(0);
    let credentials: Value = serde_json::from_str(&plaintext).expect("credential material shape");
    drop(plaintext);
    let probe_key = probe_only.then(|| credentials["api_keys"][0].as_str().expect("bearer API key for probe").to_owned());
    let captured = Arc::new(std::sync::Mutex::new(None::<Value>));
    let capture_server = if probe_only {
        let server=wiremock::MockServer::start().await;
        let capture=captured.clone();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                *capture.lock().unwrap()=Some(serde_json::from_slice(&request.body).unwrap());
                stream(None,"Diagnostic request captured; no task or tool was executed.")
            }).mount(&server).await;
        Some(server)
    } else { None };

    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("game project"); std::fs::create_dir(&project).unwrap();
    let config = AppConfig { data_dir:root.path().join("data"),work_dir:root.path().join("work"),
        auth_policy:AuthPolicy::TrustLocalToken,local_trust_secret:Some(TRUST.into()),..Default::default() };
    std::fs::create_dir(&config.data_dir).unwrap();
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let mut app = AppServices::from_config(db,&config).await.unwrap();
    // Match the reported sessions: browser role installed, no browser resource
    // selected for this coding task. Any actual browser creation makes this
    // fixture insufficient rather than pretending a browser action succeeded.
    let browser_starts = Arc::new(AtomicUsize::new(0));
    app.browser_resources = Some(Arc::new(nomifun_browser_platform::workspace::BrowserResourceService::new(
        Arc::new(super::BindingOnlyBrowser(browser_starts.clone())),
    )));
    let router = create_router(&app).await;
    let request = json!({"platform":platform,"name":"isolated reliability reproduction",
        "base_url":capture_server.as_ref().map(|server| format!("{}/v1",server.uri())).unwrap_or_else(|| row.get::<String,_>("base_url")),
        "auth_scheme":row.get::<String,_>("auth_scheme"),"credentials":if probe_only { json!({"api_keys":["capture-only"]}) } else { credentials },"enabled":true,
        "initial_model":{"model":source_model,"enabled":true,"capabilities":[{"task":"chat",
            "traits":serde_json::from_str::<Value>(&capability.get::<String,_>("traits")).unwrap(),
            "protocol":capability.get::<String,_>("protocol"),"connection_role":"default",
            "provider_params":serde_json::from_str::<Value>(&capability.get::<String,_>("provider_params")).unwrap(),
            "context_limit":capability.get::<Option<i64>,_>("context_limit"),"output_limit":capability.get::<Option<i64>,_>("output_limit")}]}});
    // Do not use an assertion helper that could echo an upstream credential
    // validation body. Only the status is reported at this write-only boundary.
    let response = router.clone().oneshot(Request::builder().method("POST").uri("/api/providers")
        .header("x-nomi-local-trust",TRUST).header("content-type","application/json")
        .body(Body::from(request.to_string())).unwrap()).await.unwrap();
    assert!(response.status().is_success(),"isolated provider setup status={}",response.status());
    let provider: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap()).unwrap();
    let model = json!({"provider_id":provider["data"]["provider_id"],"model":source_model});
    let preset = call(&router,"POST","/api/agent-presets/from-template/assistant.general",json!({"reuse_existing":false,"display_name":"Isolated general reproduction","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":project,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"},{"resource_kind":"computer","resource_id":"local-desktop"},{"resource_kind":"scheduler","resource_id":"installation-scheduler"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let started = std::time::Instant::now();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"original-task","input":{"content":input}})).await;
    let state = loop {
        let state = call(&router,"GET",&format!("/api/agent-sessions/{id}/execution"),Value::Null).await;
        let steps = state["model_progress"]["used_model_steps"].as_u64().unwrap_or(0);
        if state["state"] != "running" || started.elapsed() > Duration::from_secs(420) || steps >= 30 { break state; }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    if probe_only {
        let body=captured.lock().unwrap().clone().expect("captured production wire body");
        let shapes=probe_wire(&row.get::<String,_>("base_url"),probe_key.as_deref().unwrap(),body).await;
        std::fs::write(artifact_root.join("wire-shapes.json"),serde_json::to_vec_pretty(&shapes).unwrap()).unwrap();
        drop(router); app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
        eprintln!("LIVE_WIRE_SHAPES {shapes}");
        return;
    }
    let mut counts = std::collections::BTreeMap::<String,u64>::new();
    let mut build_digest = Value::Null;
    let mut trace = Vec::new();
    let events: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
        .bind(id).fetch_all(app.database.pool()).await.unwrap();
    for raw in events {
        let event: Value = serde_json::from_str(&raw).unwrap(); let event = &event["event"];
        let kind = event["event"].as_str().unwrap_or("");
        if kind=="turn_started" { build_digest=event["binding"]["build_digest"].clone(); }
        if kind=="tool_call_completed" && event["step"].as_u64().unwrap_or(0)>0 {
            let mut call = event["call"].clone();
            if let Some(args) = call["arguments"].as_object_mut() {
                for field in ["content","patch","input","env"] {
                    if let Some(value)=args.get_mut(field) { *value=json!({"omitted_bytes":value.to_string().len()}); }
                }
                if let Some(files)=args.get_mut("files").and_then(Value::as_array_mut) {
                    for file in files { if let Some(hunks)=file.as_object_mut().and_then(|file| file.remove("hunks")) { file["hunks_omitted"]=json!(hunks.to_string().len()); } }
                }
            }
            trace.push(json!({"event":kind,"step":event["step"],"call":call}));
        }
        if kind=="tool_completed" && event["step"].as_u64().unwrap_or(0)>0 && event["result"]["is_error"]==true {
            trace.push(json!({"event":kind,"step":event["step"],"result":event["result"]}));
        }
        if matches!(kind,"model_output_truncated"|"model_response_rejected") { trace.push(event.clone()); }
        if kind=="usage" {
            for name in ["input_tokens","output_tokens","cache_read_tokens"] {
                *counts.entry(name.into()).or_default()+=event["usage"][name].as_u64().unwrap_or(0);
            }
        }
        if matches!(kind,"model_step_started"|"model_output_truncated"|"model_response_rejected"|"compaction_started") {
            *counts.entry(kind.into()).or_default()+=1;
        }
        if kind=="tool_completed" && event["step"].as_u64().unwrap_or(0)>0 {
            *counts.entry("model_tool_results".into()).or_default()+=1;
            if event["result"]["is_error"]==true { *counts.entry("model_tool_errors".into()).or_default()+=1; }
        }
    }
    let completed = state["state"] == "completed";
    let summary = json!({"source_session":source_session,"session_id":id,"model":source_model,
        "build_digest":build_digest,"elapsed_ms":started.elapsed().as_millis(),"completed":completed,"state":state,"counts":counts,
        "browser_starts":browser_starts.load(Ordering::SeqCst)});
    std::fs::write(artifact_root.join("summary.json"),serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    std::fs::write(artifact_root.join("tool-trace.json"),serde_json::to_vec_pretty(&trace).unwrap()).unwrap();
    let mut budget = 20*1024*1024;
    copy_artifacts(&project,&artifact_root.join("workspace"),&mut budget,0);
    if !completed && matches!(state["state"].as_str(),Some("running"|"paused"|"reconciliation")) {
        call(&router,"POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({"idempotency_key":"bounded-reproduction-stop"})).await;
    }
    drop(router); app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
    eprintln!("LIVE_TOOL_RELIABILITY {}",summary);
    assert!(completed,"original task did not complete; sanitized evidence retained at {}",artifact_root.display());
    assert_eq!(browser_starts.load(Ordering::SeqCst),0,"live browser validation requires a native browser fixture");
}

async fn probe_wire(base: &str, key: &str, mut body: Value) -> Value {
    use futures_util::StreamExt;
    let http=nomifun_net::http_client_no_redirect().unwrap();
    if let Ok(name)=std::env::var("NOMIFUN_RELIABILITY_PROBE_FUNCTION") {
        let tools=body["tools"].as_array_mut().expect("captured tools");
        tools.retain(|tool| tool["function"]["name"]==name);
        assert_eq!(tools.len(),1,"probe can only select one exact advertised function");
        body["tool_choice"]=json!({"type":"function","function":{"name":name}});
    }
    let mut shapes=Vec::new();
    for streaming in [true,false] {
        let mut request=body.clone(); request["stream"]=json!(streaming);
        if !streaming { request.as_object_mut().unwrap().remove("stream_options"); }
        let started=std::time::Instant::now();
        let response=http.post(format!("{}/chat/completions",base.trim_end_matches('/')))
            .bearer_auth(key).timeout(Duration::from_secs(180)).json(&request).send().await.expect("probe transport");
        let status=response.status().as_u16();
        let mut bytes=Vec::new(); let mut stream=response.bytes_stream();
        while let Some(chunk)=stream.next().await {
            let chunk=chunk.expect("probe response stream");
            assert!(bytes.len()+chunk.len() <= 4*1024*1024,"probe response exceeded bound"); bytes.extend_from_slice(&chunk);
        }
        let wire=String::from_utf8(bytes).expect("probe UTF-8");
        let frames: Vec<Value> = if streaming {
            wire.lines().filter_map(|line| line.strip_prefix("data:").and_then(|data|serde_json::from_str(data.trim()).ok())).collect()
        } else { serde_json::from_str(&wire).ok().into_iter().collect() };
        let mut text=String::new(); let mut finish=Value::Null;
        let mut native=std::collections::BTreeMap::<usize,(String,String)>::new();
        for frame in frames {
            if let Some(reason)=frame.pointer("/choices/0/finish_reason").filter(|value| !value.is_null()) { finish=reason.clone(); }
            let item=if streaming { &frame["choices"][0]["delta"] } else { &frame["choices"][0]["message"] };
            if let Some(content)=item["content"].as_str() { text.push_str(content); }
            if let Some(calls)=item["tool_calls"].as_array() {
                for (ordinal,call) in calls.iter().enumerate() {
                    let entry=native.entry(call["index"].as_u64().unwrap_or(ordinal as u64) as usize).or_default();
                    if let Some(name)=call["function"]["name"].as_str() { entry.0.push_str(name); }
                    if let Some(args)=call["function"]["arguments"].as_str() { entry.1.push_str(args); }
                }
            }
        }
        shapes.push(json!({"streaming":streaming,"status":status,"elapsed_ms":started.elapsed().as_millis(),
            "finish_reason":finish,"native_calls":native.len(),"native_names":native.values().map(|item|&item.0).collect::<Vec<_>>(),
            "all_native_arguments_json":native.values().all(|item|serde_json::from_str::<Value>(&item.1).is_ok()),
            "text_tool_markup":text.contains("<tool_call>") && text.contains("<function="),"content_bytes":text.len(),
            "tools_executed":0}));
    }
    json!(shapes)
}

fn copy_artifacts(source: &std::path::Path, target: &std::path::Path, budget: &mut u64, depth: u8) {
    if depth>4 { return; }
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap().flatten() {
        let name=entry.file_name(); let label=name.to_string_lossy();
        if label.starts_with('.') || label=="node_modules" { continue; }
        let kind=entry.file_type().unwrap();
        if kind.is_dir() { copy_artifacts(&entry.path(),&target.join(name),budget,depth+1); }
        else if kind.is_file() {
            let bytes=entry.metadata().unwrap().len();
            if bytes<=*budget { std::fs::copy(entry.path(),target.join(name)).unwrap(); *budget-=bytes; }
        }
    }
}
