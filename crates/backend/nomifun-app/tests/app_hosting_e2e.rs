//! Real HTTP Host and real child processes. No simulated hosting success.
use std::{sync::Arc, time::Duration};
use nomifun_app::{AppConfig, AuthPolicy, compatibility::{AppServices, create_router}};
use nomifun_app_hosting::{AppRecord};
use serde_json::{Value, json};

async fn host(config: &AppConfig) -> (AppServices, tokio::task::JoinHandle<()>, String) {
    let database=nomifun_db::init_database(&config.database_path()).await.unwrap();
    let services=AppServices::from_config(database, config).await.unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}",listener.local_addr().unwrap());
    services.app_hosting.set_local_endpoint(url.clone());
    let router=create_router(&services).await;
    let task=tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    (services,task,url)
}

async fn wait_result(path: &std::path::Path) -> Value {
    for _ in 0..100 {
        if let Ok(bytes)=tokio::fs::read(path).await {
            if let Ok(value)=serde_json::from_slice(&bytes) { return value; }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("real hosted application did not finish its Host request");
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn desktop_and_headless_children_connect_without_tokens_and_old_boot_proof_expires() {
    for policy in [AuthPolicy::TrustLocalToken,AuthPolicy::Required] {
        let temp=tempfile::tempdir().unwrap();
        let source=temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("server.cjs"),r#"
const fs=require('node:fs');
const http=require('node:http');
const target=new URL('/api/apps',process.env.NOMIFUN_HOST_URL);
http.get(target,{headers:{'x-nomi-local-trust':process.env.NOMIFUN_LOCAL_TRUST}},res=>{
 let body='';res.on('data',part=>body+=part);res.on('end',()=>{
  fs.writeFileSync('connection-result.json',JSON.stringify({status:res.statusCode,body:JSON.parse(body)}));
 });
}).on('error',error=>{fs.writeFileSync('connection-result.json',JSON.stringify({error:error.message}));});
setInterval(()=>{},1000);
"#).unwrap();
        let config=AppConfig { data_dir:temp.path().join("data"),work_dir:temp.path().join("work"),auth_policy:policy,
            local_trust_secret: if policy==AuthPolicy::TrustLocalToken { Some(Arc::from("desktop-current-boot-proof")) } else { None }, ..Default::default() };
        let (services,server,url)=host(&config).await;
        let proof=services.local_trust_secret.as_ref().unwrap().to_string();
        let client=reqwest::Client::new();
        assert_eq!(client.get(format!("{url}/api/apps")).send().await.unwrap().status(),reqwest::StatusCode::FORBIDDEN);
        let imported:Value=client.post(format!("{url}/api/apps/import")).header("x-nomi-local-trust",&proof)
            .json(&json!({"source":source,"import_kind":"source","name":"Backend"})).send().await.unwrap().json().await.unwrap();
        assert_eq!(imported["success"],true,"{imported}");
        let app:AppRecord=serde_json::from_value(imported["data"].clone()).unwrap();
        let mut settings=app.settings.clone();settings.start_command=Some("node server.cjs".into());
        services.app_hosting.update(&app.id,app.name,settings).await.unwrap();
        let result:Value=client.post(format!("{url}/api/apps/{}/operate",app.id)).header("x-nomi-local-trust",&proof)
            .json(&json!({"operation":"use"})).send().await.unwrap().json().await.unwrap();
        assert_eq!(result["success"],true,"{result}");
        let connected=wait_result(&app.directory.join("connection-result.json")).await;
        assert_eq!(connected["status"],200,"{connected}");
        assert_eq!(connected["body"]["data"][0]["id"],app.id);
        assert!(!serde_json::to_string(&connected).unwrap().contains(&proof));
        assert_eq!(services.app_hosting.get(&app.id).await.unwrap().state,nomifun_app_hosting::AppState::Running);
        // The production Host cleanup stops this process tree before closing DB.
        server.abort();let _=server.await;
        services.cleanup_after_startup_failure(anyhow::anyhow!("e2e requested shutdown")).await;
        assert_eq!(services.app_hosting.get(&app.id).await.unwrap().state,nomifun_app_hosting::AppState::Stopped);
        drop(services);
        let next_config=AppConfig { local_trust_secret: if policy==AuthPolicy::TrustLocalToken { Some(Arc::from("desktop-next-boot-proof")) } else { None }, ..config };
        let (services,server,url)=host(&next_config).await;
        assert_eq!(client.get(format!("{url}/api/apps")).header("x-nomi-local-trust",&proof).send().await.unwrap().status(),reqwest::StatusCode::FORBIDDEN);
        let fresh=services.local_trust_secret.as_ref().unwrap();
        assert_ne!(proof,fresh.as_ref());
        assert_eq!(client.get(format!("{url}/api/apps")).header("x-nomi-local-trust",fresh.as_ref()).send().await.unwrap().status(),reqwest::StatusCode::OK);
        services.app_hosting.delete(&app.id).await.unwrap();
        assert!(source.join("server.cjs").exists());
        server.abort();let _=server.await;
        services.cleanup_after_startup_failure(anyhow::anyhow!("e2e requested shutdown")).await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn hosted_application_accepts_official_agents_without_authoring_their_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let config = AppConfig { data_dir: temp.path().join("data"), work_dir: temp.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from("application-official-agent-fixture")), ..Default::default() };
    let (services, server, url) = host(&config).await;
    let proof = services.local_trust_secret.as_ref().unwrap();
    let client = reqwest::Client::new();
    let library: Value = client.get(format!("{url}/api/agent-preset-templates"))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    let official = library["data"]["official_presets"].as_object().unwrap();
    let chat = official["chat.minimal"]["preset_id"].as_str().unwrap();
    let assistant = official["assistant.general"]["preset_id"].as_str().unwrap();
    assert!(official["chat.minimal"]["current_stable_revision"].is_null());
    assert_eq!(library["data"]["user_presets"], json!([]));
    let app: Value = client.post(format!("{url}/api/apps"))
        .header("x-nomi-local-trust", proof.as_ref()).json(&json!({"name":"Official runtime Agent"}))
        .send().await.unwrap().json().await.unwrap();
    let app_id = app["data"]["id"].as_str().unwrap();
    let endpoint = format!("{url}/api/apps/{app_id}");

    // Listing official options does not authorize an unknown selection or
    // persist a half-prepared RemoteBinding when normal admission rejects it.
    let rejected = client.put(&endpoint).header("x-nomi-local-trust", proof.as_ref())
        .json(&json!({"settings":{"agent_id":uuid::Uuid::now_v7().to_string()}})).send().await.unwrap();
    assert!(!rejected.status().is_success());
    let untouched = services.app_hosting.get(app_id).await.unwrap();
    assert!(untouched.settings.agent_id.is_none() && untouched.settings.agent_binding_id.is_none());

    let provider: Value = client.post(format!("{url}/api/providers"))
        .header("x-nomi-local-trust", proof.as_ref()).json(&json!({
            "platform":"openai", "name":"No-request official Agent model", "base_url":"https://example.invalid/v1",
            "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true, "sort_order":0,
            "initial_model":{"model":"official-app-fixture", "enabled":true, "capabilities":[{
                "task":"chat", "traits":[], "protocol":"openai.responses", "connection_role":"default",
                "endpoint":"/responses", "provider_params":{}, "context_limit":128000
            }]}
        })).send().await.unwrap().json().await.unwrap();
    assert_eq!(provider["success"], true, "{provider}");

    let mut binding_id: Option<String> = None;
    for (index, selected) in [chat, assistant].into_iter().enumerate() {
        let response = client.put(&endpoint).header("x-nomi-local-trust", proof.as_ref())
            .json(&json!({"settings":{"agent_id":selected}})).send().await.unwrap();
        let status = response.status();
        let saved: Value = response.json().await.unwrap();
        assert!(status.is_success(), "official selection must materialize through the existing launch path: {saved}");
        let current = &saved["data"]["settings"];
        assert_eq!(current["agent_id"], selected, "retain the public owner-authored selection");
        let prepared_id = current["agent_binding_id"].as_str().unwrap();
        assert_ne!(prepared_id, selected, "the managed environment receives a RemoteBinding reference");
        if let Some(previous) = binding_id.as_deref() { assert_eq!(prepared_id, previous); }
        else { binding_id = Some(prepared_id.to_owned()); }
        let remotes: Value = client.get(format!("{url}/api/remote-bindings"))
            .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
        assert_eq!(remotes["success"], true, "{remotes}");
        let remote = remotes["data"].as_array().unwrap().iter()
            .find(|binding| binding["remote_binding_id"] == prepared_id).unwrap();
        assert_eq!(remote["agent_binding"]["binding_version"], (index + 1) as u64);
        assert!(remote["agent_binding"]["preset_revision_ref"]["revision"].as_u64().unwrap() > 0);
    }
    let after: Value = client.get(format!("{url}/api/agent-preset-templates"))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    for key in ["chat.minimal", "assistant.general"] {
        assert!(after["data"]["official_presets"][key]["current_stable_revision"].is_null(),
            "launch preparation cannot author an official Agent default");
    }
    assert_eq!(after["data"]["user_presets"], json!([]), "hidden execution variants are not personal Agents");

    // Existing personal selections and clearing still use the same app-owned
    // RemoteBinding; this UI fix must not install a parallel binding scheme.
    let personal: Value = client.post(format!("{url}/api/agent-presets/from-template/chat.minimal"))
        .header("x-nomi-local-trust", proof.as_ref()).json(&json!({
            "display_name":"Personal text Agent", "reuse_existing":false,
            "model":{"provider_id":provider["data"]["provider_id"],"model":"official-app-fixture"}
        })).send().await.unwrap().json().await.unwrap();
    assert_eq!(personal["success"], true, "{personal}");
    let personal_id = personal["data"]["preset"]["preset_id"].as_str().unwrap();
    // Application settings carry a Preset identity, including a saved official
    // customization. They must not reinterpret it as a current Template.
    let editor: Value = client.get(format!("{url}/api/agent-presets/{chat}/editor"))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    let mut draft = editor["data"]["draft"].clone();
    draft["document"] = personal["data"]["draft"]["document"].clone();
    draft["document"]["instructions"] = json!("APPLICATION_EXPLICIT_SAVED_OFFICIAL");
    let saved: Value = client.post(format!("{url}/api/agent-presets/{chat}/revisions"))
        .header("x-nomi-local-trust", proof.as_ref()).json(&json!({
            "expected_current_revision":draft["current_revision"], "draft":draft,
            "reason":"Explicit current-generation application Agent customization"
        })).send().await.unwrap().json().await.unwrap();
    assert_eq!(saved["success"], true, "{saved}");
    let selected: Value = client.put(&endpoint).header("x-nomi-local-trust", proof.as_ref())
        .json(&json!({"settings":{"agent_id":chat}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(selected["success"], true, "{selected}");
    assert_eq!(selected["data"]["settings"]["agent_id"], chat);
    assert_eq!(selected["data"]["settings"]["agent_binding_id"], binding_id.as_deref().unwrap());
    let remotes: Value = client.get(format!("{url}/api/remote-bindings"))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    let remote = remotes["data"].as_array().unwrap().iter()
        .find(|binding| binding["remote_binding_id"] == binding_id.as_deref().unwrap()).unwrap();
    assert!(!remote["agent_binding"]["typed_resource_bindings"].as_array().unwrap().is_empty());
    let reference = &remote["agent_binding"]["preset_revision_ref"];
    let revision: Value = client.get(format!("{url}/api/agent-presets/{}/revisions/{}",
        reference["preset_id"].as_str().unwrap(), reference["revision"].as_u64().unwrap()))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    assert_eq!(revision["data"]["document"]["instructions"], "APPLICATION_EXPLICIT_SAVED_OFFICIAL");
    let selected: Value = client.put(&endpoint).header("x-nomi-local-trust", proof.as_ref())
        .json(&json!({"settings":{"agent_id":personal_id}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(selected["success"], true, "{selected}");
    assert_eq!(selected["data"]["settings"]["agent_id"], personal_id);
    assert_eq!(selected["data"]["settings"]["agent_binding_id"], binding_id.as_deref().unwrap());
    let cleared: Value = client.put(&endpoint).header("x-nomi-local-trust", proof.as_ref())
        .json(&json!({"settings":{"agent_id":null}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(cleared["success"], true, "{cleared}");
    let cleared = services.app_hosting.get(app_id).await.unwrap();
    assert!(cleared.settings.agent_id.is_none() && cleared.settings.agent_binding_id.is_none());
    let remotes: Value = client.get(format!("{url}/api/remote-bindings"))
        .header("x-nomi-local-trust", proof.as_ref()).send().await.unwrap().json().await.unwrap();
    assert!(remotes["data"].as_array().unwrap().iter()
        .all(|binding| binding["remote_binding_id"] != binding_id.as_deref().unwrap()));
    server.abort();
    let _ = server.await;
    services.cleanup_after_startup_failure(anyhow::anyhow!("official Agent selection fixture ended")).await;
}

async fn remote_fixture_request(client: &reqwest::Client, base: &str, proof: &str,
    method: &str, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
    let response = client.request(method.parse().unwrap(), format!("{base}{path}"))
        .header("x-nomi-local-trust", proof).json(&body).send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap())
}

async fn wait_remote_event(client: &reqwest::Client, base: &str, proof: &str, session: &str, expected: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (status, observed) = remote_fixture_request(client, base, proof, "GET",
            &format!("/api/remote/observe?agent_session_id={session}&after_seq=0&limit=100"), json!({})).await;
        assert!(status.is_success(), "{observed}");
        let events = observed["events"].as_array().unwrap();
        assert!(!events.iter().any(|event| matches!(event["kind"].as_str(),
            Some("turn/failed" | "turn/unknown" | "session/open-failed"))), "{observed}");
        if events.iter().any(|event| event["kind"] == expected) { return observed; }
        assert!(tokio::time::Instant::now() < deadline, "Remote did not reach {expected}: {observed}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn remote_domain_turn_and_initial_input_execute_without_opening_ordinary_intake() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
    let model = MockServer::start().await;
    let chunk = json!({"id":"remote-fixture", "object":"chat.completion.chunk", "created":1,
        "model":"remote-fixture", "choices":[{"index":0,
            "delta":{"role":"assistant","content":"REMOTE_FIXTURE_ANSWER"},"finish_reason":null}]});
    let end = json!({"id":"remote-fixture", "object":"chat.completion.chunk", "created":1,
        "model":"remote-fixture", "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")))
        .expect(2).mount(&model).await;
    let temp = tempfile::tempdir().unwrap();
    let config = AppConfig { data_dir: temp.path().join("data"), work_dir: temp.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(Arc::from("remote-domain-boundary-fixture")), ..Default::default() };
    let (services, server, url) = host(&config).await;
    let proof = services.local_trust_secret.as_ref().unwrap();
    let client = reqwest::Client::new();
    let (status, provider) = remote_fixture_request(&client, &url, proof, "POST", "/api/providers", json!({
        "platform":"openai", "name":"Isolated Remote contract model", "base_url":format!("{}/v1",model.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true, "sort_order":0,
        "initial_model":{"model":"remote-fixture","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096
        }]}
    })).await;
    assert!(status.is_success(), "{provider}");
    let (_, library) = remote_fixture_request(&client, &url, proof, "GET", "/api/agent-preset-templates", json!({})).await;
    let agent = library["data"]["official_presets"]["chat.minimal"]["preset_id"].as_str().unwrap();
    let (_, app) = remote_fixture_request(&client, &url, proof, "POST", "/api/apps", json!({"name":"Remote boundary fixture"})).await;
    let app_id = app["data"]["id"].as_str().unwrap();
    let (status, saved) = remote_fixture_request(&client, &url, proof, "PUT", &format!("/api/apps/{app_id}"),
        json!({"settings":{"agent_id":agent}})).await;
    assert!(status.is_success(), "{saved}");
    let binding = saved["data"]["settings"]["agent_binding_id"].as_str().unwrap();
    let session_count_before: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM nomi_remote_sessions")
        .fetch_one(services.database.pool()).await.unwrap();
    let (status, reserved_initial) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/open",
        json!({"binding_id":binding,"idempotency_key":"reserved-channel-origin-open",
            "initial_input":{"content":"Cannot claim a trusted Channel source","origin":"channel_guest"}})).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{reserved_initial}");
    assert_eq!(reserved_initial["code"], "NOMI_CORE_INVALID_REQUEST");
    let session_count_after: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM nomi_remote_sessions")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(session_count_before, session_count_after, "reserved initial origin cannot create a Remote Session");
    let (status, opened) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/open",
        json!({"binding_id":binding,"idempotency_key":"remote-domain-open"})).await;
    assert!(status.is_success(), "{opened}");
    let session = opened["agent_session_id"].as_str().unwrap();
    wait_remote_event(&client, &url, proof, session, "session/ready").await;
    let before_reserved: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=?")
        .bind(session).fetch_one(services.database.pool()).await.unwrap();
    let (status, reserved_turn) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/turn",
        json!({"agent_session_id":session,"idempotency_key":"reserved-channel-origin-turn",
            "input":{"content":"Cannot claim a trusted Channel source","origin":"channel_guest"}})).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "{reserved_turn}");
    assert_eq!(reserved_turn["code"], "NOMI_CORE_INVALID_REQUEST");
    let after_reserved: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=?")
        .bind(session).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(before_reserved, after_reserved, "reserved turn origin cannot cross canonical acceptance");
    let input = json!({"agent_session_id":session,"input":{"content":"Normal exact Remote input"},"idempotency_key":"remote-domain-turn"});
    let (status, submitted) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/turn", input.clone()).await;
    assert!(status.is_success(), "Remote authority cannot be sent through ordinary intake: {submitted}");
    let first = wait_remote_event(&client, &url, proof, session, "turn/completed").await;
    let (status, replayed) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/turn", input).await;
    assert!(status.is_success(), "exact-key replay must retain the original Turn: {replayed}");
    let (status, denied) = remote_fixture_request(&client, &url, proof, "POST",
        &format!("/api/agent-sessions/{session}/messages"),
        json!({"idempotency_key":"ordinary-must-stay-rejected","input":{"content":"No Remote domain authority"}})).await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{denied}");
    let (_, unchanged) = remote_fixture_request(&client, &url, proof, "GET",
        &format!("/api/remote/observe?agent_session_id={session}&after_seq=0&limit=100"), json!({})).await;
    assert_eq!(first["next_cursor"], unchanged["next_cursor"], "replay/denied ordinary input cannot add Remote facts");
    let (status, initial) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/open",
        json!({"binding_id":binding,"idempotency_key":"remote-domain-initial-open",
            "initial_input":{"content":"Initial exact Remote input"}})).await;
    assert!(status.is_success(), "{initial}");
    let second = initial["agent_session_id"].as_str().unwrap();
    assert_ne!(session, second);
    wait_remote_event(&client, &url, proof, second, "turn/completed").await;
    let turn_count: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id IN (?,?)")
        .bind(session).bind(second).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(turn_count, 2, "normal and initial Remote inputs each own one canonical Turn");
    assert_eq!(model.received_requests().await.unwrap().len(), 2, "replay and rejected ordinary input never call the model again");
    let (status, cancelled) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/cancel",
        json!({"agent_session_id":session,"idempotency_key":"remote-domain-exact-cancel"})).await;
    assert!(status.is_success(), "{cancelled}");
    assert_eq!(cancelled["session_status"], "cancelled");
    let (status, closed) = remote_fixture_request(&client, &url, proof, "POST", "/api/remote/turn",
        json!({"agent_session_id":session,"input":{"content":"Cannot reopen cancelled Remote Session"},"idempotency_key":"after-cancel"})).await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{closed}");
    let other: String = nomifun_db::sqlx::query_scalar("SELECT state FROM nomi_remote_sessions WHERE agent_session_id=?")
        .bind(second).fetch_one(services.database.pool()).await.unwrap();
    assert_ne!(other, "cancelled", "cancel authority is restricted to its exact Remote Session");
    remote_fixture_request(&client, &url, proof, "POST", "/api/remote/cancel",
        json!({"agent_session_id":second,"idempotency_key":"remote-domain-second-cleanup"})).await;
    model.verify().await;
    server.abort(); let _ = server.await;
    services.cleanup_after_startup_failure(anyhow::anyhow!("Remote domain fixture ended")).await;
}
