//! End-to-end evidence for the production Unified Plugin Core HTTP boundary.

mod common;

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use base64::Engine as _;
use http_body_util::BodyExt as _;
use nomifun_agent_contracts::PluginId;
use nomifun_plugin_platform::PluginServiceObservation;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

use common::{body_json, get_with_token, json_with_token, setup_and_login};

const LOCAL_TRUST: &str = "plugin-e2e-local-desktop";

#[tokio::test]
async fn plugin_creation_preflight_checks_the_whole_module_without_creating_a_draft() {
    let harness = Harness::new().await;
    let (status, response) = harness.json("POST", "/api/conversations/plugin-preflight", json!({
        "selection":{"kind":"template","templateKey":"assistant.general"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{response}");
    assert_eq!(response["data"]["status"],"ready");
    assert_eq!(response["data"]["reason"],"");
    assert!(response["data"]["required_actions"].as_array().unwrap().iter()
        .any(|action|action=="plugin.development/install"));
    assert_eq!(response["data"]["owner_user_id"].as_str().unwrap().len(),36);
    let (status, disabled) = harness.json("POST", "/api/conversations/plugin-preflight", json!({
        "selection":{"kind":"template","templateKey":"chat.minimal"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{disabled}");
    assert_eq!(disabled["data"]["status"],"configure_agent");
    assert_eq!(disabled["data"]["reason"],"PLUGIN_MODULE_DISABLED");
    let (_,drafts)=harness.get_json("/api/plugin-drafts").await;
    assert_eq!(drafts["data"]["drafts"].as_array().unwrap().len(),0);
}

#[tokio::test]
async fn plugin_preflight_requires_all_creation_actions_and_rechecks_session_admission() {
    let harness=Harness::new().await;
    let provider=create_chat_provider(&harness,"http://127.0.0.1:9/v1","Module admission","module-fixture").await;
    let (status,created)=harness.json("POST","/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Plugin developer","model_route_refs":{},"chat_route_records":{},
        "model":{"provider_id":provider,"model":"module-fixture"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{created}");
    let preset=created["data"]["preset"]["preset_id"].as_str().unwrap().to_owned();
    let mut draft=created["data"]["draft"].clone();
    let mut reference=created["data"]["revision"]["reference"].clone();
    for (actions,expected) in [
        (json!(["plugin.development/list","plugin.development/read"]),"configure_agent"),
        (json!(nomifun_plugin_development::CREATE_ACTIONS),"ready"),
    ] {
        draft["document"]["enabled_capabilities"]=json!([{"capability":{"id":"plugin.development"},"action_allowlist":actions}]);
        let (status,saved)=harness.json("POST",&format!("/api/agent-presets/{preset}/revisions"),json!({
            "expected_current_revision":reference,"draft":draft
        })).await;
        assert_eq!(status,StatusCode::OK,"{saved}");
        reference=saved["data"]["revision"]["reference"].clone();
        draft["current_revision"]=reference.clone();
        let (status,check)=harness.json("POST","/api/conversations/plugin-preflight",json!({
            "selection":{"kind":"preset","presetId":preset}
        })).await;
        assert_eq!(status,StatusCode::OK,"{check}");
        assert_eq!(check["data"]["status"],expected,"{check}");
    }
    // A successful preflight does not grant authority to a later stable revision.
    draft["document"]["enabled_capabilities"]=json!([]);
    let (status,_)=harness.json("POST",&format!("/api/agent-presets/{preset}/revisions"),json!({
        "expected_current_revision":reference,"draft":draft
    })).await;
    assert_eq!(status,StatusCode::OK);
    let (status,blocked)=harness.json("POST","/api/agent-sessions",json!({
        "preset_id":preset,"required_modules":["plugin.development"],
        "model":{"provider_id":provider,"model":"module-fixture"}
    })).await;
    assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY,"{blocked}");
    assert_eq!(blocked["code"],"AGENT_LAUNCH_MODULE_REQUIRED");
}

#[tokio::test]
async fn ordinary_conversation_cannot_complete_plugin_delivery_with_prose_only() {
    let harness=Harness::new().await;
    let upstream=wiremock::MockServer::start().await;
    let reply_seen=Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observer=reply_seen.clone();
    let stream=format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"id":"delivery-fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"The plugin is ready."},"finish_reason":null}]}),
        json!({"id":"delivery-fixture","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":6,"total_tokens":16}})
    );
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request:&wiremock::Request| {
            let body:Value=request.body_json().unwrap();
            if body["messages"].to_string().contains("OWNER_PLUGIN_REPLY_284") {
                observer.store(true,std::sync::atomic::Ordering::SeqCst);
            }
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type","text/event-stream").set_body_string(stream.clone())
        })
        .mount(&upstream).await;
    let provider=create_chat_provider(&harness,&format!("{}/v1",upstream.uri()),"Prose-only delivery","delivery-fixture").await;
    let (status,created)=harness.json("POST","/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Plugin delivery fixture","model_route_refs":{},"chat_route_records":{},
        "model":{"provider_id":provider,"model":"delivery-fixture"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{created}");
    let preset=created["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft=created["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"]=json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status,saved)=harness.json("POST",&format!("/api/agent-presets/{preset}/revisions"),json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status,StatusCode::OK,"{saved}");
    let (status,session)=harness.json("POST","/api/agent-sessions",json!({
        "preset_id":preset,"required_modules":["plugin.development"],
        "model":{"provider_id":provider,"model":"delivery-fixture"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{session}");
    let id=session["data"]["agent_session_id"].as_str().unwrap();
    let (status,accepted)=harness.json("POST",&format!("/api/agent-sessions/{id}/turns"),json!({
        "idempotency_key":"plugin-prose-only",
        "input":{"content":"Create and install a working text-cleanup plugin.","plugin_delivery":{}}
    })).await;
    assert!(status.is_success(),"{accepted}");
    let settled=tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            let paused:Option<String>=nomifun_db::sqlx::query_scalar(
                "SELECT native_pause_json FROM agent_turns WHERE session_id=? AND native_pause_json IS NOT NULL"
            ).bind(id).fetch_optional(harness.services.database.pool()).await.unwrap().flatten();
            if let Some(paused)=paused {
                assert!(paused.contains("PLUGIN_DELIVERY_REQUIRED"),"{paused}");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
    }).await;
    if settled.is_err() {
        let turns:Vec<(String,Option<String>)>=nomifun_db::sqlx::query_as(
            "SELECT state,native_pause_json FROM agent_turns WHERE session_id=?"
        ).bind(id).fetch_all(harness.services.database.pool()).await.unwrap();
        let events:Vec<(String,String)>=nomifun_db::sqlx::query_as(
            "SELECT kind,COALESCE(inline_json,'') FROM agent_events WHERE session_id=? ORDER BY seq DESC LIMIT 12"
        ).bind(id).fetch_all(harness.services.database.pool()).await.unwrap();
        panic!("Host did not reject final text: turns={turns:?}; events={events:?}");
    }
    let completed:i64=nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
    ).bind(id).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!(completed,0);
    let (_, execution)=harness.get_json(&format!("/api/agent-sessions/{id}/execution")).await;
    let execution=&execution["data"];
    let continuation=json!({"request":{
        "operation_id":execution["operation_id"],"idempotency_key":"plugin-owner-reply",
        "expected_pause_revision":execution["pause"]["revision"],
        "expected_checkpoint_revision":execution["checkpoint_revision"],
        "expected_checkpoint_digest":execution["checkpoint_digest"],"budget":{}
    },"input":{"content":"OWNER_PLUGIN_REPLY_284: use uppercase text."}});
    let route=format!("/api/agent-sessions/{id}/plugin-continuation");
    let mut invalid=continuation.clone();
    invalid["request"]["expected_checkpoint_digest"]=json!("f".repeat(64));
    let (status,rejected)=harness.json("POST",&route,invalid).await;
    assert_eq!(status,StatusCode::CONFLICT,"{rejected}");
    let mut increased=continuation.clone();
    increased["request"]["budget"]=json!({"additional_segments":1});
    let (status,rejected)=harness.json("POST",&route,increased).await;
    assert_eq!(status,StatusCode::BAD_REQUEST,"{rejected}");
    let (status,accepted)=harness.json("POST",&route,continuation.clone()).await;
    assert!(status.is_success(),"{accepted}");
    tokio::time::timeout(Duration::from_secs(15),async {
        while !reply_seen.load(std::sync::atomic::Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(25)).await; }
    }).await.expect("recovered provider must receive the persisted owner's reply");
    let (status,replayed)=harness.json("POST",&route,continuation.clone()).await;
    assert!(status.is_success(),"{replayed}");
    assert_eq!(replayed["data"]["duplicate"],true);
    let mut changed=continuation;
    changed["input"]["content"]=json!("Different reply using the same operation key");
    let (status,rejected)=harness.json("POST",&route,changed).await;
    assert_eq!(status,StatusCode::CONFLICT,"{rejected}");
    let (roots,inputs):(i64,i64)=nomifun_db::sqlx::query_as(
        "SELECT SUM(kind='turn/started'),SUM(kind='turn/steer-accepted') FROM agent_events WHERE session_id=?"
    ).bind(id).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!((roots,inputs),(1,1),"reply/retry must preserve the original task and one input receipt");
    let (status,cancelled)=harness.json("POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({
        "idempotency_key":"plugin-reply-fixture-stop"
    })).await;
    assert!(status.is_success(),"{cancelled}");
}

/// Script only the provider's decisions. Session admission, tool schemas,
/// Kernel calls, human approvals, Node execution, repairs and installation all
/// use production owners. This does not stand in for live-model acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_conversation_repairs_and_delivers_a_real_headless_plugin() {
    run_headless_workflow(1,false,true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_delivered_plugin_cannot_complete_a_two_output_request() {
    run_headless_workflow(2,false,true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn newly_installed_tool_is_used_by_the_original_conversation_after_safe_resume() {
    run_headless_workflow(1,true,true).await;
}

/// Once durable host facts prove delivery (installed, observed and consumed
/// in this conversation), the host owns the outcome: the turn settles on a
/// plain final answer without the incidentally active ledger demanding a
/// completion account.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_conversation_completes_on_host_verified_use_without_a_completion_account() {
    run_headless_workflow(1,true,false).await;
}

/// Real Node failures drive the production PluginDeliveryCheck. A third
/// planned failure pauses without a model final, and an owner reply resumes
/// the same canonical Turn with its original cumulative limits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn planned_plugin_failures_pause_and_owner_continuation_repairs_the_same_turn() {
    run_planned_plugin_stall_workflow(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_plugin_delivery_marker_planned_failures_pause_and_resume_the_same_turn() {
    run_planned_plugin_stall_workflow(false).await;
}

async fn run_planned_plugin_stall_workflow(explicit_delivery:bool) {
    #[derive(Default)]
    struct Script {
        phase: usize, calls: usize, planned: bool, reply_seen: bool,
        draft: String, revision: Value, digest: Value, plugin: Value,
    }
    let harness=Harness::new().await;
    let upstream=wiremock::MockServer::start().await;
    let script=Arc::new(Mutex::new(Script::default()));
    let decisions=script.clone();
    const BROKEN:&str="export async function activate() { return { async invoke(action,input) { return {text:input.text.toLowerCase()}; } }; }";
    const REPLY:&str="STALLED_PLUGIN_REPLY_29: repair the existing Service to return uppercase, retain the plan, and finish installation.";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request:&wiremock::Request| {
            let body:Value=request.body_json().unwrap();
            if body["tools"].as_array().is_none_or(Vec::is_empty) {
                assert!(body.to_string().contains("Write only a compact continuation note"));
                return plugin_workflow_stream(json!({"content":"Continue the existing uppercase plugin; keep its plan, repair its Service and install."}),"stop");
            }
            let last=body["messages"].as_array().unwrap().iter().rev()
                .find(|message|message["role"]=="tool").and_then(|message|message["content"].as_str());
            let result:Value=last.and_then(|text|serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state=decisions.lock().unwrap();
            if let Some(id)=result["summary"]["draft_id"].as_str() { state.draft=id.into(); }
            for revision in [&result["summary"]["revision"],&result["revision"],&result["draft"]["revision"]] {
                if revision.is_number() { state.revision=revision.clone(); }
            }
            if result["verification_digest"].is_string() { state.digest=result["verification_digest"].clone(); }
            if result["delivery"]["plugin_id"].is_string() { state.plugin=result["delivery"]["plugin_id"].clone(); }
            if matches!(state.phase,4|11) && result["waiting_for_user"]==true {
                // Preview already requests a native safe-point pause. After
                // approval, retry preview rather than offering another final.
                state.phase-=1;
            }
            if state.phase==7 {
                assert!(body["messages"].to_string().contains(REPLY),
                    "the watchdog must stop before another model request without an owner reply");
                state.reply_seen=true;
            }
            if state.phase==1 && !state.planned {
                state.planned=true; state.calls+=1;
                let tool=body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("stall-plan-{}",state.calls),"type":"function",
                    "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft,"expected_revision":state.revision,"plan":{
                        "summary":"Create an uppercase Service","output_key":"uppercase","outputs":[{"key":"uppercase","kind":"headless"}],
                        "features":[{"description":"Uppercase the input","case_names":["required-uppercase"]}],
                        "cases":{"required-uppercase":{"kind":"action","action":"normalize","input":{"text":"Hello"},"expected_output":{"text":"HELLO"}}}
                    }}).to_string()}}]}),"tool_calls");
            }
            let (action,input)=match state.phase {
                0=>("open",json!({})),
                1=>{
                    let files=result["files"].as_array().unwrap();
                    let mut manifest:Value=serde_json::from_str(files.iter().find(|file|file["path"]=="nomifun.plugin.json")
                        .unwrap()["text"].as_str().unwrap()).unwrap();
                    manifest["entrypoints"]=json!({"service":"service/main.mjs"});
                    manifest["actions"]=json!({"normalize":{"name":"Uppercase","description":"Uppercase text",
                        "input":{"type":"object","required":["text"],"properties":{"text":{"type":"string"}}},
                        "output":{"type":"object","required":["text"],"properties":{"text":{"type":"string"}}},"effect":"read"}});
                    manifest["bindings"]=json!([{"point":"desktop.command","action":"normalize"}]);
                    let delete=files.iter().filter_map(|file|file["path"].as_str()).filter(|path|path.starts_with("ui/")).collect::<Vec<_>>();
                    ("apply",json!({"draft_id":state.draft,"expected_revision":state.revision,"delete":delete,
                        "files":{"nomifun.plugin.json":manifest.to_string(),"service/main.mjs":BROKEN}}))
                },
                2|9=>("check",json!({"draft_id":state.draft,"expected_revision":state.revision})),
                3|10=>("preview",json!({"draft_id":state.draft,"expected_revision":state.revision})),
                4..=6|11=>{
                    if matches!(state.phase,5|6) { assert_eq!(result["passed"],false,"the real Node case must fail: {last:?}"); }
                    ("test_action",json!({"draft_id":state.draft,"expected_revision":state.revision,"case_name":"required-uppercase",
                        "action":"normalize","input":{"text":"Hello"},"expected_output":{"text":"HELLO"}}))
                },
                7=>("read",json!({"draft_id":state.draft})),
                8=>("apply",json!({"draft_id":state.draft,"expected_revision":state.revision,
                    "files":{"service/main.mjs":BROKEN.replace("toLowerCase","toUpperCase")}})),
                12=>{
                    assert_eq!(result["passed"],true,"the resumed Service must satisfy the same oracle: {last:?}");
                    ("install",json!({"draft_id":state.draft,"expected_revision":state.revision,"verification_digest":state.digest}))
                },
                13=>{
                    assert!(state.plugin.is_string(),"the repaired plugin must be durably delivered: {last:?}");
                    ("inspect",json!({"plugin_id":state.plugin}))
                },
                14=>return plugin_workflow_stream(json!({"content":"The repaired uppercase Service is installed."}),"stop"),
                other=>panic!("unexpected repair phase {other}: {last:?}"),
            };
            let needle=format!("Action: plugin.development/{action}.");
            let tool=body["tools"].as_array().unwrap().iter().find(|tool|
                tool["function"]["description"].as_str().unwrap_or("").contains(&needle)).unwrap();
            state.phase+=1; state.calls+=1;
            plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("stall-work-{}",state.calls),"type":"function",
                "function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}),"tool_calls")
        }).mount(&upstream).await;
    let provider=create_chat_provider(&harness,&format!("{}/v1",upstream.uri()),"Plugin stall","plugin-stall").await;
    let (status,preset)=harness.json("POST","/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Plugin stall","model":{"provider_id":provider,"model":"plugin-stall"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{preset}");
    let id=preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft=preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"]=json!([{"capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS}]);
    let (status,saved)=harness.json("POST",&format!("/api/agent-presets/{id}/revisions"),json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status,StatusCode::OK,"{saved}");
    let (status,session)=harness.json("POST","/api/agent-sessions",json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"plugin-stall"}
    })).await;
    assert_eq!(status,StatusCode::OK,"{session}");
    let session=session["data"]["agent_session_id"].as_str().unwrap();
    let mut input=json!({"content":"Create and install a Service that returns uppercase text."});
    if explicit_delivery { input["plugin_delivery"]=json!({}); }
    let (status,accepted)=harness.json("POST",&format!("/api/agent-sessions/{session}/turns"),json!({
        "idempotency_key":"plugin-stall-start","input":input
    })).await;
    assert!(status.is_success(),"{accepted}");
    let mut confirmation=String::new();
    let paused=tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let (_,execution)=harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
            let execution=&execution["data"];
            if execution["state"]=="paused" && execution["pause"]["reason"]=="PLUGIN_VERIFICATION_REQUIRED" { break execution.clone(); }
            let draft_id=script.lock().unwrap().draft.clone();
            approve_stalled_service_if_requested(&harness,session,&draft_id,execution,&mut confirmation).await;
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await;
    let paused=match paused {
        Ok(paused)=>paused,
        Err(_)=>{
            let (_,execution)=harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
            let rows:Vec<(String,String)>=nomifun_db::sqlx::query_as(
                "SELECT kind,substr(COALESCE(inline_json,''),1,2048) FROM agent_events WHERE session_id=? ORDER BY seq DESC LIMIT 18"
            ).bind(session).fetch_all(harness.services.database.pool()).await.unwrap();
            let events:Vec<_>=rows.into_iter().map(|(kind,payload)|(kind,nomi_redact::redact_secrets_owned(payload))).collect();
            let phase=script.lock().unwrap().phase;
            panic!("three real planned failures must pause before another model request: phase={phase}, execution={execution}, recent canonical events={events:?}");
        },
    };
    assert_eq!(script.lock().unwrap().phase,7);
    assert!(!script.lock().unwrap().reply_seen);
    assert_eq!(paused["checkpoint_retained"],true,"the watchdog uses a resumable native checkpoint");
    let rows:Vec<String>=nomifun_db::sqlx::query_scalar(
        "SELECT inline_json FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event') IN ('tool_completed','execution_budget_exhausted') ORDER BY seq"
    ).bind(session).fetch_all(harness.services.database.pool()).await.unwrap();
    let events:Vec<nomifun_agent_runtime::AgentEngineEvent>=rows.iter().map(|row|{
        let payload:Value=serde_json::from_str(row).unwrap();
        serde_json::from_value(payload["event"].clone()).unwrap()
    }).collect();
    let failures=events.iter().filter(|event|match event {
        nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..}=>{
            let output:Value=serde_json::from_str(&result.output_text()).unwrap_or(Value::Null);
            output["case_name"]=="required-uppercase" && output["planned"]==true && output["passed"]==false
        },
        _=>false,
    }).count();
    assert_eq!(failures,3,"the production watchdog counts actual planned test results");
    assert!(events.iter().any(|event|matches!(event,nomifun_agent_runtime::AgentEngineEvent::ExecutionBudgetExhausted {
        reason:nomifun_agent_runtime::AgentExecutionStopReason::PluginVerificationStalled,.. })));
    let continuation=json!({"request":{"operation_id":paused["operation_id"],"idempotency_key":"plugin-stall-owner-reply",
        "expected_pause_revision":paused["pause"]["revision"],"expected_checkpoint_revision":paused["checkpoint_revision"],
        "expected_checkpoint_digest":paused["checkpoint_digest"],"budget":{}},"input":{"content":REPLY}});
    let route=format!("/api/agent-sessions/{session}/plugin-continuation");
    let mut increased=continuation.clone(); increased["request"]["budget"]=json!({"additional_segments":1});
    let (status,rejected)=harness.json("POST",&route,increased).await;
    assert_eq!(status,StatusCode::BAD_REQUEST,"the plugin reply cannot grant additional budget: {rejected}");
    let (status,resumed)=harness.json("POST",&route,continuation).await;
    assert!(status.is_success(),"the stalled task must support its existing continuation API: {resumed}");
    tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            let completed:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'")
                .bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
            if completed==1 { break; }
            let (_,execution)=harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
            let draft_id=script.lock().unwrap().draft.clone();
            approve_stalled_service_if_requested(&harness,session,&draft_id,&execution["data"],&mut confirmation).await;
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("the original task must repair and deliver after the owner reply");
    assert!(script.lock().unwrap().reply_seen);
    assert_eq!(script.lock().unwrap().phase,14);
    let (roots,inputs,failed):(i64,i64,i64)=nomifun_db::sqlx::query_as(
        "SELECT SUM(kind='turn/started'),SUM(kind='turn/steer-accepted'),SUM(kind='turn/failed') FROM agent_events WHERE session_id=?"
    ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!((roots,inputs,failed),(1,1,0),"continuation belongs to one original Turn and one accepted reply");
    let (_,finished)=harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
    assert_eq!(finished["data"]["operation_id"],paused["operation_id"]);
    // Resume advances the budget record's CAS revision even with an empty
    // grant; the actual allowances must remain exactly unchanged.
    for field in ["journal_bytes","journal_records","session_payload_bytes"] {
        assert_eq!(finished["data"]["budget"][field],paused["budget"][field],
            "recovery must not increase the cumulative {field} allowance");
    }
    // Completed turns may release their checkpoint. Each live generation's
    // canonical prepared budget proves the same total model ceiling instead.
    let model_limits:Vec<i64>=nomifun_db::sqlx::query_scalar(
        "SELECT DISTINCT CAST(json_extract(inline_json,'$.event.max_model_steps') AS INTEGER) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='execution_budget_prepared'"
    ).bind(session).fetch_all(harness.services.database.pool()).await.unwrap();
    let original_limit=paused["model_progress"]["steps_per_segment"].as_u64().unwrap()
        * paused["model_progress"]["maximum_segments"].as_u64().unwrap();
    assert_eq!(model_limits,vec![original_limit as i64],"every resumed generation keeps the original cumulative model allowance");
}

async fn approve_stalled_service_if_requested(harness:&Harness,session:&str,draft:&str,execution:&Value,previous:&mut String) {
    if draft.is_empty() || execution["state"]!="paused"
        || !matches!(execution["pause"]["reason"].as_str(),Some("PLUGIN_AUTHORIZATION_REQUIRED"|"EXECUTION_USER_REQUESTED")) { return; }
    let (_,report)=harness.get_json(&format!("/api/plugin-drafts/{draft}/authoring")).await;
    let confirmation=report["data"]["confirmation"]["confirmation_id"].as_str().unwrap_or("");
    if confirmation.is_empty() || confirmation==previous.as_str() { return; }
    let (status,approved)=harness.json("POST",&format!("/api/plugin-drafts/{draft}/approve"),json!({
        "expected_revision":report["data"]["draft"]["summary"]["revision"],"confirmation_id":confirmation,"approved":true
    })).await;
    assert_eq!(status,StatusCode::OK,"{approved}");
    *previous=confirmation.to_owned();
    let (status,resumed)=harness.json("POST",&format!("/api/agent-sessions/{session}/execution/resume"),json!({
        "operation_id":execution["operation_id"],"idempotency_key":format!("plugin-stall-approve-{confirmation}"),
        "expected_pause_revision":execution["pause"]["revision"],"expected_checkpoint_revision":execution["checkpoint_revision"],
        "expected_checkpoint_digest":execution["checkpoint_digest"],"budget":{}
    })).await;
    assert!(status.is_success(),"{resumed}");
}

async fn run_headless_workflow(expected_count: u8, current_use: bool, account_after_use: bool) {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    #[derive(Default)]
    struct Script {
        phase: usize,
        planned: bool,
        consumed: bool,
        accepted_plan: Value,
        draft_id: String,
        revision: Value,
        verification_digest: Value,
        plugin_id: Value,
        inspected: Value,
        controls: std::collections::BTreeMap<usize, usize>,
    }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let provider_script = script.clone();
    let broken_service = r#"export async function activate(ctx) {
  let calls = 0;
  return { async invoke(action, input) {
    if (action === 'count') return { calls };
    if (action === 'echo') return { echoed: input };
    if (action === 'persist') {
      if ('value' in input) await ctx.storage.kv.set('sample', input.value);
      return { value: await ctx.storage.kv.get('sample') };
    }
    if (action !== 'normalize') throw new Error('unsupported action');
    calls += 1;
    return { text: input.text.trim().toLowerCase() };
  }};
}"#;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let tools = body["tools"].as_array();
            if tools.is_none_or(|tools| tools.is_empty()) {
                assert!(body.to_string().contains("Write only a compact continuation note"));
                return plugin_workflow_stream(json!({"content":"Continue the original text-cleanup Plugin task. Inspect the current existing draft and installed state before more effects. Preserve the required uppercase behavior and do not create another plugin."}), "stop");
            }
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool")
                .and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = provider_script.lock().unwrap();
            if let Some(id) = result["summary"]["draft_id"].as_str() {
                state.draft_id = id.into();
                state.revision = result["summary"]["revision"].clone();
            } else if result["draft_id"].is_string() && result["revision"].is_number() {
                state.revision = result["revision"].clone();
            } else if result["draft"]["draft_id"].is_string() && result["draft"]["revision"].is_number() {
                state.revision = result["draft"]["revision"].clone();
            }
            if result["verification_digest"].is_string() {
                state.verification_digest = result["verification_digest"].clone();
            }
            if result["delivery"]["plugin_id"].is_string() {
                state.plugin_id = result["delivery"]["plugin_id"].clone();
            }
            if result["summary"]["enabled"].is_boolean() {
                state.inspected = result.clone();
            }
            let mut phase = state.phase;
            if phase==1 && !state.planned {
                state.planned=true;
                let mut outputs=json!([{"key":"cleanup","kind":"headless"}]);
                if expected_count>1 { outputs.as_array_mut().unwrap().push(json!({"key":"second","kind":"headless"})); }
                let mut plan=json!({"summary":"Create the requested plugins and preserve a stored sample", "output_key":"cleanup","outputs":outputs,
                    "features":[{"description":"Trim and uppercase text","case_names":["required-uppercase"]},
                        {"description":"Store and recover the sample after Service restart","case_names":["write-sample","read-after-restart"],"requires_persistence":true}],
                    "cases":{"required-uppercase":{"kind":"action","action":"normalize","input":{"text":"  Mixed Case  "},"expected_output":{"text":"MIXED CASE"}},
                        "write-sample":{"kind":"action","action":"persist","input":{"value":"saved"},"expected_output":{"value":"saved"}},
                        "read-after-restart":{"kind":"action","action":"persist","input":{},"expected_output":{"value":"saved"},"restart":true}}});
                if current_use { plan["current_conversation_case"]=json!({"action":"normalize","input":{"text":"  Current call  "},"expected_output":{"text":"CURRENT CALL"}}); }
                state.accepted_plan=plan.clone();
                let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-requirements-plan","type":"function",
                    "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,"plan":plan}).to_string()}}]}),"tool_calls");
            }
            // A repeated bare open for the same request must reopen the
            // existing draft instead of accumulating conflicting drafts.
            if phase==1 && state.planned && expected_count==1
                && state.controls.get(&0).copied().unwrap_or_default()==0 {
                state.controls.insert(0,1);
                let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/open.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-second-open","type":"function",
                    "function":{"name":tool["function"]["name"],"arguments":"{}"}}]}),"tool_calls");
            }
            if phase==21 && current_use && !state.consumed {
                let stable=format!("plugin:{}/normalize",state.plugin_id.as_str().unwrap());
                if let Some(tool)=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains(&format!("Unified Plugin Action {stable}."))) {
                    state.consumed=true;
                    state.controls.insert(21,0);
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-current-consumption","type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":json!({"text":"  Current call  "}).to_string()}}]}),"tool_calls");
                }
            }
            if phase==8 && state.controls.get(&8).copied().unwrap_or_default()==0 {
                assert_eq!(result["passed"],false);
                state.controls.insert(8,1);
                let mut weakened=state.accepted_plan.clone();
                weakened["cases"]["required-uppercase"]["expected_output"]["text"]=json!("mixed case");
                let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-weakened-plan","type":"function",
                    "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,"plan":weakened}).to_string()}}]}),"tool_calls");
            }
            if phase==19 {
                let count=state.controls.entry(19).or_default();
                if *count<2 {
                    let read=*count==1; *count+=1;
                    let mut input=json!({"draft_id":state.draft_id,"expected_revision":state.revision,"action":"persist",
                        "input":if read{json!({})}else{json!({"value":"saved"})},"expected_output":{"value":"saved"},
                        "case_name":if read{"read-after-restart"}else{"write-sample"}});
                    if read { input["restart"]=json!(true); }
                    let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/test_action.")).unwrap();
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":if read{"plugin-persist-read"}else{"plugin-persist-write"},"type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}),"tool_calls");
                }
                // The recorded count oracle only holds while normalize is not
                // called again, so supplementary cases run after its re-run.
                if *count==2 {
                    *count+=1;
                    assert_eq!(result["passed"], true, "repaired restart case must pass: {last:?}");
                    // A supplementary case has no planned value; the action's
                    // declared object schema still coerces a JSON-encoded input.
                    let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/test_action.")).unwrap();
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-supplementary-encoded","type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,
                            "action":"normalize","input":"{\"text\":\"  Extra Call  \"}","expected_output":"{\"text\":\"EXTRA CALL\"}",
                            "case_name":"supplementary-encoded"}).to_string()}}]}),"tool_calls");
                }
                if *count==3 {
                    *count+=1;
                    assert_eq!(result["passed"], true, "schema-guided coercion must satisfy the supplementary oracle: {last:?}");
                    // A string-schema field is never coerced: the encoded text
                    // must reach the action verbatim even though it parses.
                    let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/test_action.")).unwrap();
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-supplementary-string","type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,
                            "action":"echo","input":"[1, 2]","expected_output":{"echoed":"[1, 2]"},
                            "case_name":"supplementary-string-schema"}).to_string()}}]}),"tool_calls");
                }
                if *count==4 {
                    *count+=1;
                    assert_eq!(result["passed"],true,"the raw string probe must pass: {last:?}");
                    let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/test_action.")).unwrap();
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-failed-diagnostic","type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,
                            "action":"normalize","input":{"text":"diagnostic"},"expected_output":{"text":"deliberately incorrect"},
                            "case_name":"failed-diagnostic"}).to_string()}}]}),"tool_calls");
                }
            }
            if matches!(phase, 4 | 14) {
                assert_eq!(result["waiting_for_user"], true, "{last:?}");
                assert!(!last.unwrap().contains("confirmation_id"));
                phase += 1;
                state.phase = phase;
            }
            if phase==21 {
                let post=state.controls.entry(20).or_default();
                if *post==0 {
                    *post+=1;
                    // install revoked the draft preview; a test_action now
                    // must be rejected by the host, not recorded as a failed
                    // case.
                    let tool=tools.unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/test_action.")).unwrap();
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"plugin-post-install-test","type":"function",
                        "function":{"name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft_id,"expected_revision":state.revision,
                            "action":"normalize","input":"{\"text\":\"post install\"}","expected_output":"{}",
                            "case_name":"post-install-supplementary"}).to_string()}}]}),"tool_calls");
                }
                if *post==1 {
                    *post+=1;
                    assert!(last.unwrap().contains("PLUGIN_PREVIEW_REQUIRED") && last.unwrap().contains("After install"), "{last:?}");
                }
            }
            // After host-verified consumption the delivery is proven by
            // durable facts; the incidental ledger may not demand an account.
            if phase==21 && !account_after_use && state.consumed
                && state.controls.get(&20).copied().unwrap_or_default()>=2 {
                return plugin_workflow_stream(json!({"content":"The installed normalize tool answered in this conversation."}), "stop");
            }
            // Recovery activates the shared engine's task ledger. Respect its
            // real planning/completion contract rather than disabling it.
            if matches!(phase, 5 | 15 | 21) && tools.unwrap().iter().any(|tool| tool["function"]["name"] == "update_plan") {
                let count = state.controls.entry(phase).or_default();
                if *count < if phase == 21 { 2 } else { 1 } {
                    let control = *count;
                    *count += 1;
                    let (name, arguments) = if phase == 21 && control == 1 {
                        ("report_completion", json!({"summary":"Verified and installed the exact repaired plugin", "observed_tool_error_count":4,
                            "criteria":[{"step":"Create and deliver text cleanup", "disposition":"supported", "evidence_call_ids":if current_use && state.consumed{json!(["plugin-current-consumption"])}else{json!(["plugin-workflow-20"])}, "requirement_ids":if current_use && state.consumed{json!(["input_0","input_1"])}else{json!(["input_0"])}, "rationale":"Fresh inspection and actual tool results cover the installed artifact and requested behavior."}]}))
                    } else {
                        ("update_plan", json!({"explanation":"Continue the original plugin and retain its required uppercase behavior",
                            "plan":[{"step":"Create and deliver text cleanup","status":if phase == 21 {"completed"}else{"in_progress"}}]}))
                    };
                    return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("plugin-control-{phase}-{control}-{}",u8::from(state.consumed)),"type":"function", "function":{"name":name,"arguments":arguments.to_string()}}]}), "tool_calls");
                }
            }
            state.phase += 1;
            let draft = &state.draft_id;
            let revision = &state.revision;
            let (action, input) = match phase {
                0 => ("open", json!({})),
                1 => {
                    if expected_count==1 {
                        assert_eq!(result["summary"]["draft_id"].as_str(),Some(state.draft_id.as_str()),
                            "a second bare open must reopen the request's draft: {last:?}");
                        assert!(result["notice"].as_str().unwrap_or_default().contains("already has draft"), "{last:?}");
                    }
                    let files = result["files"].as_array().expect("open returns actual files");
                    let manifest = files.iter().find(|file| file["path"] == "nomifun.plugin.json").unwrap();
                    let mut manifest: Value = serde_json::from_str(manifest["text"].as_str().unwrap()).unwrap();
                    manifest["name"] = json!("Conversation text cleanup");
                    manifest["entrypoints"] = json!({"service":"service/main.mjs","serviceMode":"continuous"});
                    manifest["actions"] = json!({
                        "persist":{"name":"Store sample","description":"Persist/read a sample","input":{"type":"object"},"output":{"type":"object"},"effect":"write"},
                        "normalize":{"name":"Normalize text","description":"Trim and uppercase text", "input":{"type":"object","required":["text"],"properties":{"text":{"type":"string"}}},"output":{"type":"object"},"effect":"read"},
                        "count":{"name":"Inspect call count","description":"Read temporary invocation count", "input":{"type":"object"},"output":{"type":"object"},"effect":"read"},
                        "echo":{"name":"Echo input","description":"Return the raw input", "input":{"type":"string"},"output":{"type":"object"},"effect":"read"}
                    });
                    manifest["bindings"] = json!([{"point":"desktop.command","action":"normalize"}]);
                    if current_use { manifest["bindings"].as_array_mut().unwrap().push(json!({"point":"agent.tool","action":"normalize"})); }
                    let remove = files.iter().filter_map(|file| file["path"].as_str())
                        .filter(|path| path.starts_with("ui/")).collect::<Vec<_>>();
                    ("apply", json!({"draft_id":draft,"expected_revision":revision,
                        "files":{"nomifun.plugin.json":manifest.to_string(),"service/main.mjs":broken_service},"delete":remove}))
                }
                2 | 12 => {
                    assert!(result["summary"]["revision"].is_number(), "file apply must succeed at phase {phase}: {last:?}");
                    ("check", json!({"draft_id":draft,"expected_revision":revision}))
                }
                3 | 6 | 13 | 16 => ("preview", json!({"draft_id":draft,"expected_revision":revision})),
                5 | 15 => ("read", json!({"draft_id":draft})),
                // JSON values arriving as encoded strings still satisfy the
                // strict oracle when they decode exactly to the planned value.
                7 | 17 => ("test_action", json!({"draft_id":draft,"expected_revision":revision,
                    "action":"normalize","input":"{\"text\":\"  Mixed Case  \"}","expected_output":"{\"text\":\"MIXED CASE\"}","case_name":"required-uppercase"})),
                8 => {
                    assert!(last.unwrap().contains("PLUGIN_PLAN_CHANGED"),"failed requirements must not be weakened: {last:?}");
                    ("test_action", json!({"draft_id":draft,"expected_revision":revision,
                        "action":"normalize","input":{"text":"  Mixed Case  "},"expected_output":{"text":"mixed case"},"case_name":"required-uppercase"}))
                }
                9 | 18 => {
                    if phase == 9 {
                        let rejection = last.unwrap();
                        assert!(rejection.contains("PLUGIN_ORACLE_CHANGED")
                            && rejection.contains("case 'required-uppercase' must run exactly as planned")
                            && rejection.contains("\"expected_output\":{\"text\":\"MIXED CASE\"}")
                            && rejection.contains("Pass every value as JSON"), "{last:?}");
                    } else {
                        assert_eq!(result["passed"], true, "repair must satisfy the original requirement");
                    }
                    ("test_action", json!({"draft_id":draft,"expected_revision":revision,
                        "action":"count","input":{},"expected_output":{"calls":1},"case_name":"oracle-change-has-no-effect"}))
                }
                10 | 19 => {
                    assert_eq!(result["passed"], phase==10, "the diagnostic may fail without blocking install: {last:?}");
                    if phase==19 { assert_eq!(result["planned"],false,"diagnostics do not become requirements"); }
                    ("install", json!({"draft_id":draft,"expected_revision":revision,"verification_digest":state.verification_digest}))
                }
                11 => {
                    assert!(last.unwrap().contains("PLUGIN_BUSINESS_CHECK_REQUIRED"), "{last:?}");
                    ("apply", json!({"draft_id":draft,"expected_revision":revision,
                        "files":{"service/main.mjs":broken_service.replace("toLowerCase", "toUpperCase")}}))
                }
                20 => {
                    assert!(state.plugin_id.is_string(), "installer must return durable delivery: {last:?}");
                    ("inspect", json!({"plugin_id":state.plugin_id}))
                }
                _ => {
                    // The host settlement check relays the still-missing
                    // second output once; a re-report without new tool work
                    // is not relayed again, so the completion gate pauses the
                    // turn. Each rejection invalidates the recorded account,
                    // so re-report.
                    if expected_count > 1 && last.unwrap().contains("Completion account") {
                        return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("plugin-recheck-{phase}"),"type":"function",
                            "function":{"name":"report_completion","arguments":json!({
                                "summary":"Verified and installed the first requested plugin; the second output remains undelivered",
                                "observed_tool_error_count":4,
                                "criteria":[{"step":"Create and deliver text cleanup","disposition":"supported",
                                    "evidence_call_ids":["plugin-workflow-20"],"requirement_ids":["input_0"],
                                    "rationale":"Fresh inspection and actual tool results cover the installed artifact."}]
                            }).to_string()}}]}), "tool_calls");
                    }
                    panic!("unexpected provider request {phase}: {last:?}");
                }
            };
            let (delta, finish) = if action.is_empty() {
                (json!({"content":input.as_str().unwrap()}), "stop")
            } else {
                let needle = format!("Action: plugin.development/{action}.");
                let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("").contains(&needle))
                    .unwrap_or_else(|| panic!("ordinary Agent is missing {needle}"));
                (json!({"tool_calls":[{"index":0,"id":format!("plugin-workflow-{phase}"),"type":"function",
                    "function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}), "tool_calls")
            };
            plugin_workflow_stream(delta, finish)
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1", upstream.uri()), "Plugin workflow", "plugin-workflow").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"Ordinary Agent workflow fixture",
        "model":{"provider_id":provider,"model":"plugin-workflow"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"plugin-workflow"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap().to_owned();
    let mut input = json!({"content":"Create and install a text-cleanup plugin that trims whitespace, converts text to uppercase and preserves a stored sample after Service restart."});
    if current_use { input["content"]=json!("Create and install the text-cleanup plugin, preserve a stored sample across restart, and use normalize in this same conversation on '  Current call  ' to obtain 'CURRENT CALL'."); }
    if expected_count > 1 {
        input["content"] = json!("Create two separate usable plugins; one trims whitespace and converts text to uppercase.");
    }
    // Ordinary follow-up turns need no UI marker: actual file edits create the
    // Host obligation. Explicit output count additionally prevents partial success.
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"plugin-workflow-initial",
        "input":input
    })).await;
    assert!(status.is_success(), "{accepted}");
    let mut approvals = 0;
    let mut previous_confirmation = String::new();
    let mut resumed_consumption=false;
    let settled = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let completed: i64 = nomifun_db::sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
            ).bind(&session).fetch_one(harness.services.database.pool()).await.unwrap();
            if completed > 0 { break; }
            let draft_id = script.lock().unwrap().draft_id.clone();
            if !draft_id.is_empty() {
                let (_, details) = harness.get_json(&format!("/api/plugin-drafts/{draft_id}/authoring")).await;
                let confirmation = details["data"]["confirmation"]["confirmation_id"].as_str().unwrap_or("");
                let (_, execution) = harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
                let execution = &execution["data"];
                if current_use && !resumed_consumption && execution["state"]=="paused"
                    && execution["pause"]["reason"]=="PLUGIN_CURRENT_CONVERSATION_PENDING" {
                    resumed_consumption=true;
                    let (status,resumed)=harness.json("POST",&format!("/api/agent-sessions/{session}/plugin-continuation"),json!({
                        "request":{"operation_id":execution["operation_id"],"idempotency_key":"plugin-current-use-resume",
                            "expected_pause_revision":execution["pause"]["revision"],"expected_checkpoint_revision":execution["checkpoint_revision"],
                            "expected_checkpoint_digest":execution["checkpoint_digest"],"budget":{}},
                        "input":{"content":"Use the installed normalize tool in this original conversation on the requested input before completing."}
                    })).await;
                    assert!(status.is_success(),"{resumed}");
                }
                if expected_count > 1 && execution["state"] == "paused"
                    && execution["pause"]["reason"] == "PLUGIN_DELIVERY_REQUIRED" { break; }
                if !confirmation.is_empty() && confirmation != previous_confirmation && execution["state"] == "paused" {
                    let (status, approved) = harness.json("POST", &format!("/api/plugin-drafts/{draft_id}/approve"), json!({
                        "expected_revision":details["data"]["draft"]["summary"]["revision"],
                        "confirmation_id":confirmation,"approved":true
                    })).await;
                    assert_eq!(status, StatusCode::OK, "{approved}");
                    previous_confirmation = confirmation.into();
                    approvals += 1;
                    let (status, resumed) = harness.json("POST", &format!("/api/agent-sessions/{session}/execution/resume"), json!({
                        "operation_id":execution["operation_id"],
                        "idempotency_key":format!("plugin-workflow-resume-{approvals}"),
                        "expected_pause_revision":execution["pause"]["revision"],
                        "expected_checkpoint_revision":execution["checkpoint_revision"],
                        "expected_checkpoint_digest":execution["checkpoint_digest"],"budget":{}
                    })).await;
                    assert!(status.is_success(), "{resumed}");
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await;
    if settled.is_err() {
        let (_, execution) = harness.get_json(&format!("/api/agent-sessions/{session}/execution")).await;
        let events: Vec<(String, String)> = nomifun_db::sqlx::query_as(
            "SELECT kind,COALESCE(inline_json,'') FROM agent_events WHERE session_id=? ORDER BY seq DESC LIMIT 12"
        ).bind(&session).fetch_all(harness.services.database.pool()).await.unwrap();
        let state = script.lock().unwrap();
        panic!("workflow did not complete; phase={}, draft={}, approvals={approvals}, execution={execution}, events={events:?}", state.phase, state.draft_id);
    }
    assert_eq!(approvals, 2, "changed Service code needs a fresh exact-artifact approval");
    let completed: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
    ).bind(&session).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!(completed, if expected_count == 1 { 1 } else { 0 }, "partial delivery cannot settle the request");
    let (_, authoring) = harness.get_json(&format!("/api/plugin-drafts/{}/authoring", script.lock().unwrap().draft_id)).await;
    assert!(authoring["data"]["verification"]["cases"]["post-install-supplementary"].is_null(),
        "a rejected post-install test_action must record no case: {authoring}");
    let state = script.lock().unwrap();
    if expected_count == 1 {
        assert_eq!(state.phase, 21, "all development and installed inspection calls must execute");
    } else {
        assert!(state.phase >= 21, "all development and installed inspection calls must execute");
    }
    if account_after_use {
        assert_eq!(state.controls.get(&21), Some(&2), "the native ledger must accept the final evidence report");
    } else {
        assert_eq!(state.controls.get(&21), Some(&0),
            "host-verified use must need no report_completion after the consumption");
    }
    assert_eq!(state.inspected["summary"]["enabled"], true);
    if current_use { assert!(state.consumed,"the original conversation must actually invoke the installed tool"); }
    let plugin_id = state.plugin_id.as_str().unwrap().to_owned();
    let draft_id = state.draft_id.clone();
    drop(state);
    let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{draft_id}/authoring")).await;
    assert_eq!(report["data"]["verification"]["cases"]["required-uppercase"]["passed"], true);
    assert_eq!(report["data"]["verification"]["plan"]["cases"]["required-uppercase"]["expected_output"]["text"], "MIXED CASE");
    assert!(report["data"]["verification"]["acceptance"].is_null(),"requirements have one source: {report}");
    assert_eq!(report["data"]["verification"]["cases"]["failed-diagnostic"]["passed"],false);
    assert_eq!(report["data"]["verification"]["installed_observation"]["read_probes"].as_array().unwrap().len(),1,
        "diagnostic probes must not be replayed against installed storage");
    assert_eq!(report["data"]["verification"]["delivery"]["plugin_id"], plugin_id);
    assert_eq!(report["data"]["verification"]["installed_observation"]["read_probes"][0]["action"], "normalize");
    assert_eq!(report["data"]["verification"]["installed_observation"]["read_probes"][0]["actual_output"]["text"], "MIXED CASE");
    assert_eq!(report["data"]["verification"]["cases"]["read-after-restart"]["persistence_checked"],true);
    let (_, library) = harness.get_json("/api/plugins").await;
    assert_eq!(library["data"]["plugins"].as_array().unwrap().len(), 1);
    let (status, invoked) = harness.json("POST", "/api/plugins/desktop/commands/invoke", json!({
        "action_id":format!("plugin:{plugin_id}/normalize"),"input":{"text":"  Actual delivery  "}
    })).await;
    assert_eq!(status, StatusCode::OK, "{invoked}");
    assert_eq!(invoked["data"]["text"], "ACTUAL DELIVERY");
    let (_, installed) = harness.get_json(&format!("/api/plugins/{plugin_id}")).await;
    let (_, current_draft) = harness.get_json(&format!("/api/plugin-drafts/{draft_id}/authoring")).await;
    let (status, repeated) = harness.json("POST", &format!("/api/plugin-drafts/{draft_id}/save"), json!({
        "expected_revision":current_draft["data"]["draft"]["summary"]["revision"],
        "expected_plugin_revision":installed["data"]["summary"]["revision"],"config":{}
    })).await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    let (_, after_repeat) = harness.get_json(&format!("/api/plugins/{plugin_id}")).await;
    assert_eq!(after_repeat["data"]["summary"]["revision"], installed["data"]["summary"]["revision"],
        "saving already committed bytes must return the existing install without another revision");
    assert_eq!(after_repeat["data"]["summary"]["active"], installed["data"]["summary"]["active"]);
    let (status, trashed) = harness.json("POST", &format!("/api/plugins/{plugin_id}/trash"), json!({
        "expected_revision":installed["data"]["summary"]["revision"]
    })).await;
    assert_eq!(status, StatusCode::OK, "{trashed}");
    let (status, deleted) = harness.json("DELETE", &format!("/api/plugins/{plugin_id}"), json!({
        "expected_revision":trashed["data"]["summary"]["revision"],"acknowledge_permanent_delete":true
    })).await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    let (_, historical) = harness.get_json(&format!("/api/plugin-drafts/{draft_id}/authoring")).await;
    assert!(historical["data"]["draft"]["summary"]["plugin_id"].is_null());
    assert!(historical["data"]["draft"]["summary"]["base_plugin_revision"].is_null());
    assert_eq!(historical["data"]["verification"]["delivery"]["plugin_id"], plugin_id,
        "deleting an installed instance must not erase its historical delivery receipt");
}

/// Fixture reports cover cancellation/ownership, not actual DOM acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_installed_ui_verification_preserves_commit_and_closes_private_surface() {
    #[derive(Default)]
    struct Script { phase: usize, call: usize, planned: bool, draft: String, revision: Value, digest: Value }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let decisions = script.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool").and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = decisions.lock().unwrap();
            if let Some(id) = result["summary"]["draft_id"].as_str() { state.draft = id.into(); }
            if result["summary"]["revision"].is_number() { state.revision = result["summary"]["revision"].clone(); }
            if result["revision"].is_number() { state.revision = result["revision"].clone(); }
            if result["verification_digest"].is_string() { state.digest = result["verification_digest"].clone(); }
            if state.phase == 4 && !result["descriptor"].is_object() { state.phase = 3; }
            if state.phase==1 && !state.planned {
                state.planned=true; state.call+=1;
                let tool=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"ui-cancel-plan","type":"function","function":{
                    "name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft,"expected_revision":state.revision,"plan":{
                        "summary":"Create the count UI","output_key":"counter","outputs":[{"key":"counter","kind":"ui"}],
                        "features":[{"description":"Show zero","case_names":["initial-count"]}],
                        "cases":{"initial-count":{"kind":"ui","steps":[{"operation":"text","selector":"#count","value":"0"}]}}
                    }}).to_string()}}]}),"tool_calls");
            }
            let (action, input) = match state.phase {
                0 => ("open", json!({})),
                1 => {
                    let files = result["files"].as_array().unwrap();
                    let mut manifest: Value = serde_json::from_str(files.iter()
                        .find(|file| file["path"] == "nomifun.plugin.json").unwrap()["text"].as_str().unwrap()).unwrap();
                    manifest["entrypoints"] = json!({"ui":"ui/index.html"});
                    manifest["actions"] = json!({});
                    manifest["bindings"] = json!([]);
                    let delete = files.iter().filter_map(|file| file["path"].as_str())
                        .filter(|path| path.starts_with("service/")).collect::<Vec<_>>();
                    ("apply", json!({"draft_id":state.draft,"expected_revision":state.revision,"delete":delete,
                        "files":{"nomifun.plugin.json":manifest.to_string(),
                            "ui/index.html":"<!doctype html><html><body><span id='count'>0</span></body></html>"}}))
                }
                2 => ("check", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                3 => ("preview", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                4 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.revision,
                    "case_name":"initial-count","steps":[{"operation":"text","selector":"#count","value":"0"}]})),
                5 => ("install", json!({"draft_id":state.draft,"expected_revision":state.revision,"verification_digest":state.digest})),
                _ => panic!("cancel must interrupt pending installed verification: {last:?}"),
            };
            let needle = format!("Action: plugin.development/{action}.");
            let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                tool["function"]["description"].as_str().unwrap_or("").contains(&needle)).unwrap();
            state.phase += 1;
            state.call += 1;
            plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("ui-cancel-{}",state.call),
                "type":"function","function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}), "tool_calls")
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1",upstream.uri()), "UI cancellation", "ui-cancel").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"UI cancellation","model":{"provider_id":provider,"model":"ui-cancel"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"ui-cancel"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"ui-cancel-start","input":{"content":"Create and install a UI showing count zero.","plugin_delivery":{}}
    })).await;
    assert!(status.is_success(), "{accepted}");
    let formal = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let id = script.lock().unwrap().draft.clone();
            if !id.is_empty() {
                let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{id}/authoring")).await;
                if let Some(command) = report["data"]["commands"].as_array().and_then(|commands| commands.first()) {
                    if command["case_name"] == "installed_surface_bridge" { break command["descriptor"].clone(); }
                    let (status, acknowledged) = harness.json("POST", &format!("/api/plugin-drafts/{id}/ui-results"), json!({
                        "test_token":command["test_token"],"descriptor":command["descriptor"],"observations":["0"]
                    })).await;
                    assert_eq!(status, StatusCode::OK, "{acknowledged}");
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("installed verification must remain pending on its actual Surface");
    let (status, _, _) = harness.get_public_bytes(&surface_asset_path(&formal)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, cancelled) = tokio::time::timeout(Duration::from_secs(3), harness.json("POST", &format!("/api/agent-sessions/{session}/turns/cancel"), json!({
        "idempotency_key":"ui-cancel-stop"
    }))).await.expect("owner cancellation must not wait for the 25-second UI timeout");
    assert!(status.is_success(), "{cancelled}");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (status, _, _) = harness.get_public_bytes(&surface_asset_path(&formal)).await;
            if status != StatusCode::OK { break; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("cancelled verification must revoke its private Surface");
    let id = script.lock().unwrap().draft.clone();
    let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{id}/authoring")).await;
    assert!(report["data"]["verification"]["delivery"].is_null());
    assert_eq!(report["data"]["verification"]["installed_observation"]["ui_ready"], false);
    assert!(report["data"]["commands"].as_array().unwrap().is_empty());
    let (_, plugin) = harness.get_json(&format!("/api/plugins/{}",formal["plugin_id"].as_str().unwrap())).await;
    assert_eq!(plugin["data"]["summary"]["enabled"], true, "cancellation after commit must preserve installation");
    let completed: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
    ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!(completed, 0);
}

/// Two consecutive test_ui cases must not share preview storage: each queued
/// command carries its own preview surface_session_id, and a prose-only final
/// answer still pauses after the bounded host settlement check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn consecutive_test_ui_cases_use_fresh_preview_surfaces() {
    #[derive(Default)]
    struct Script {
        phase: usize, call: usize, planned: bool, feedback_seen: bool,
        draft: String, revision: Value, edit_revision: Value,
    }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let decisions = script.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool").and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = decisions.lock().unwrap();
            if body["messages"].to_string().contains("Host completion check") {
                state.feedback_seen = true;
            }
            if let Some(id) = result["summary"]["draft_id"].as_str() { state.draft = id.into(); }
            if result["summary"]["revision"].is_number() { state.revision = result["summary"]["revision"].clone(); }
            if result["revision"].is_number() { state.revision = result["revision"].clone(); }
            if state.phase==2 { state.edit_revision=state.revision.clone(); }
            if state.phase == 4 && !result["descriptor"].is_object() { state.phase = 3; }
            if state.phase == 1 && !state.planned {
                state.planned = true; state.call += 1;
                let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"ui-fresh-plan","type":"function","function":{
                    "name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft,"expected_revision":state.revision,"plan":{
                        "summary":"Create the count UI","output_key":"counter","outputs":[{"key":"counter","kind":"ui"}],
                        "features":[{"description":"Show zero","case_names":["first-count","second-count"]}],
                        "cases":{
                            "first-count":{"kind":"ui","steps":[{"operation":"text","selector":"#count","value":"0"}]},
                            "second-count":{"kind":"ui","steps":[{"operation":"text","selector":"#count","value":"0"}]}
                        }
                    }}).to_string()}}]}),"tool_calls");
            }
            let (action, input) = match state.phase {
                0 => ("open", json!({})),
                1 => {
                    let files = result["files"].as_array().unwrap();
                    let mut manifest: Value = serde_json::from_str(files.iter()
                        .find(|file| file["path"] == "nomifun.plugin.json").unwrap()["text"].as_str().unwrap()).unwrap();
                    manifest["entrypoints"] = json!({"ui":"ui/index.html"});
                    manifest["actions"] = json!({});
                    manifest["bindings"] = json!([]);
                    let delete = files.iter().filter_map(|file| file["path"].as_str())
                        .filter(|path| path.starts_with("service/")).collect::<Vec<_>>();
                    ("apply", json!({"draft_id":state.draft,"expected_revision":state.revision,"delete":delete,
                        "files":{"nomifun.plugin.json":manifest.to_string(),
                            "ui/index.html":"<!doctype html><html><body><span id='count'>0</span></body></html>"}}))
                }
                2 => ("check", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                3 => ("preview", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                4 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.edit_revision,
                    "case_name":"first-count","steps":[{"operation":"text","selector":"#count","value":"0"}]})),
                5 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.edit_revision,
                    "case_name":"second-count","steps":[{"operation":"text","selector":"#count","value":"0"}]})),
                _ => {
                    return plugin_workflow_stream(
                        json!({"role":"assistant","content":"The plugin is delivered."}), "stop");
                }
            };
            let needle = format!("Action: plugin.development/{action}.");
            let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                tool["function"]["description"].as_str().unwrap_or("").contains(&needle)).unwrap();
            state.phase += 1;
            state.call += 1;
            plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("ui-fresh-{}",state.call),
                "type":"function","function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}), "tool_calls")
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1",upstream.uri()), "UI fresh storage", "ui-fresh").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"UI fresh storage","model":{"provider_id":provider,"model":"ui-fresh"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"ui-fresh"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"ui-fresh-start","input":{"content":"Create the count UI.","plugin_delivery":{}}
    })).await;
    assert!(status.is_success(), "{accepted}");
    let descriptors = tokio::time::timeout(Duration::from_secs(30), async {
        let mut captured = std::collections::BTreeMap::<String, Value>::new();
        while captured.len() < 2 {
            let id = script.lock().unwrap().draft.clone();
            if !id.is_empty() {
                let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{id}/authoring")).await;
                if let Some(commands) = report["data"]["commands"].as_array() {
                    for command in commands {
                        let token = command["test_token"].as_str().unwrap_or("");
                        let case = command["case_name"].as_str().unwrap_or("");
                        if !["first-count","second-count"].contains(&case) || captured.contains_key(token) {
                            continue;
                        }
                        captured.insert(token.to_owned(), command["descriptor"].clone());
                        let (status, acknowledged) = harness.json("POST", &format!("/api/plugin-drafts/{id}/ui-results"), json!({
                            "test_token":command["test_token"],"descriptor":command["descriptor"],"observations":["0"]
                        })).await;
                        assert_eq!(status, StatusCode::OK, "{acknowledged}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        captured.into_values().collect::<Vec<_>>()
    }).await.expect("two consecutive test_ui cases must queue their commands");
    assert_ne!(
        descriptors[0]["surface_session_id"], descriptors[1]["surface_session_id"],
        "every UI case starts on fresh preview storage"
    );
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let paused: Option<String> = nomifun_db::sqlx::query_scalar(
                "SELECT native_pause_json FROM agent_turns WHERE session_id=? AND native_pause_json IS NOT NULL"
            ).bind(session).fetch_optional(harness.services.database.pool()).await.unwrap().flatten();
            if let Some(paused) = paused {
                assert!(paused.contains("PLUGIN_DELIVERY_REQUIRED"), "{paused}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("the completion gate still pauses an undelivered plugin");
    assert!(script.lock().unwrap().feedback_seen,
        "the host settlement check must relay the delivery gap into the same turn");
}

/// A follow-up turn in a conversation that owns a draft gets neutral plugin
/// context — the draft id and the authorized functions — instead of a
/// delivery obligation or an unconditional install claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn follow_up_turn_gets_plugin_context_without_a_delivery_obligation() {
    #[derive(Default)]
    struct Script {
        opened: bool,
        draft: String,
        follow_up: Vec<Value>,
    }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let decisions = script.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool").and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = decisions.lock().unwrap();
            if let Some(id) = result["summary"]["draft_id"].as_str() { state.draft = id.into(); }
            if body["messages"].to_string().contains("What can this draft do?") {
                state.follow_up.push(body.clone());
                return plugin_workflow_stream(
                    json!({"role":"assistant","content":"It is still an empty draft."}), "stop");
            }
            if !state.opened {
                state.opened = true;
                let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("")
                        .contains("Action: plugin.development/open.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"ctx-open",
                    "type":"function","function":{"name":tool["function"]["name"],
                        "arguments":"{}"}}]}), "tool_calls");
            }
            plugin_workflow_stream(
                json!({"role":"assistant","content":"Draft opened."}), "stop")
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1",upstream.uri()), "Draft context", "draft-context").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"Draft context","model":{"provider_id":provider,"model":"draft-context"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"draft-context"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"draft-context-open","input":{"content":"Open a plugin draft.","plugin_delivery":{}}
    })).await;
    assert!(status.is_success(), "{accepted}");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let paused: Option<String> = nomifun_db::sqlx::query_scalar(
                "SELECT native_pause_json FROM agent_turns WHERE session_id=? AND native_pause_json IS NOT NULL"
            ).bind(session).fetch_optional(harness.services.database.pool()).await.unwrap().flatten();
            if let Some(paused) = paused {
                assert!(paused.contains("PLUGIN_DELIVERY_REQUIRED"), "{paused}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("the undelivered turn pauses for the post-turn gate");
    let draft_id = script.lock().unwrap().draft.clone();
    assert!(!draft_id.is_empty(), "the open result carries summary.draft_id");
    let (status, cancelled) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns/cancel"), json!({
        "idempotency_key":"draft-context-stop"
    })).await;
    assert!(status.is_success(), "{cancelled}");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let running: i64 = nomifun_db::sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_turns WHERE session_id=? AND state='running'"
            ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
            if running == 0 { break; }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("the cancelled turn leaves the running state");
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"draft-context-followup","input":{"content":"What can this draft do?"}
    })).await;
    assert!(status.is_success(), "{accepted}");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if !script.lock().unwrap().follow_up.is_empty() { break; }
            let completed: i64 = nomifun_db::sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
            ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
            if completed > 0 { break; }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("the follow-up turn produces a model request");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let completed: i64 = nomifun_db::sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
            ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
            if completed == 1 { break; }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("the follow-up turn completes without a delivery obligation");
    let pauses: Vec<Option<String>> = nomifun_db::sqlx::query_scalar(
        "SELECT native_pause_json FROM agent_turns WHERE session_id=? ORDER BY accepted_at"
    ).bind(session).fetch_all(harness.services.database.pool()).await.unwrap();
    assert_eq!(pauses.len(), 2);
    // Turn 1's pause was already observed before cancellation cleared the
    // row; the context-only follow-up must never pause.
    assert!(pauses[1].is_none(), "the context-only follow-up does not pause");
    let follow_up = &script.lock().unwrap().follow_up[0];
    let serialized = serde_json::to_string(follow_up).unwrap();
    // The context is injected as its own system message; the draft id alone
    // could come from turn-1 tool history, so assert on that exact message.
    let context = follow_up["messages"].as_array().unwrap().iter()
        .find_map(|message| message["content"].as_str()
            .filter(|content| content.contains("Plugin development context")))
        .unwrap_or_else(|| panic!("no Plugin development context message in {serialized}"));
    assert!(context.contains(&draft_id), "{context}");
    assert!(context.contains(r#""status":"in_progress""#), "{context}");
    assert!(context.contains("not a new requirement or a delivery obligation"), "{context}");
    assert!(!context.contains("cannot complete until install succeeds"), "{context}");
    let open_tool = follow_up["tools"].as_array().unwrap().iter().any(|tool|
        tool["function"]["description"].as_str().unwrap_or("")
            .contains("Action: plugin.development/open."));
    assert!(open_tool, "the plugin development functions stay bound: {serialized}");
}

/// A failed diagnostic UI case does not become required or overwrite planned
/// readiness. Install still verifies its Surface, and post-install cases are
/// refused without stripping delivery; the delivered turn settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_diagnostic_ui_case_does_not_block_delivery_and_post_install_cases_are_refused() {
    #[derive(Default)]
    struct Script {
        phase: usize, planned: bool, refused: Option<String>,
        draft: String, revision: Value, digest: Value,
    }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let decisions = script.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool").and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = decisions.lock().unwrap();
            if let Some(id) = result["summary"]["draft_id"].as_str() { state.draft = id.into(); }
            if result["summary"]["revision"].is_number() { state.revision = result["summary"]["revision"].clone(); }
            if result["revision"].is_number() { state.revision = result["revision"].clone(); }
            if result["draft"]["revision"].is_number() { state.revision = result["draft"]["revision"].clone(); }
            if result["verification_digest"].is_string() { state.digest = result["verification_digest"].clone(); }
            if state.phase == 4 && !result["descriptor"].is_object() { state.phase = 3; }
            if state.phase == 6 {
                assert_eq!(result["case_name"],"debug-count");
                assert_eq!(result["planned"],false);
                assert_eq!(result["passed"],false);
                assert_eq!(result["diagnostics"][0]["step_index"],0);
                assert_eq!(result["diagnostics"][0]["expected"],"1");
                assert_eq!(result["diagnostics"][0]["actual"],"0");
            }
            if state.phase == 8 {
                state.refused = last.map(str::to_owned);
                return plugin_workflow_stream(
                    json!({"role":"assistant","content":"The count UI is installed."}), "stop");
            }
            if state.phase == 1 && !state.planned {
                state.planned = true;
                let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("").contains("Action: plugin.development/plan.")).unwrap();
                return plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":"ui-after-install-plan","type":"function","function":{
                    "name":tool["function"]["name"],"arguments":json!({"draft_id":state.draft,"expected_revision":state.revision,"plan":{
                        "summary":"Create the count UI","output_key":"counter","outputs":[{"key":"counter","kind":"ui"}],
                        "features":[{"description":"Show zero","case_names":["initial-count"]}],
                        "cases":{"initial-count":{"kind":"ui","steps":[{"operation":"text","selector":"#count","value":"0"}]}}
                    }}).to_string()}}]}),"tool_calls");
            }
            let (action, input) = match state.phase {
                0 => ("open", json!({})),
                1 => {
                    let files = result["files"].as_array().unwrap();
                    let mut manifest: Value = serde_json::from_str(files.iter()
                        .find(|file| file["path"] == "nomifun.plugin.json").unwrap()["text"].as_str().unwrap()).unwrap();
                    manifest["entrypoints"] = json!({"ui":"ui/index.html"});
                    manifest["actions"] = json!({});
                    manifest["bindings"] = json!([]);
                    let delete = files.iter().filter_map(|file| file["path"].as_str())
                        .filter(|path| path.starts_with("service/")).collect::<Vec<_>>();
                    ("apply", json!({"draft_id":state.draft,"expected_revision":state.revision,"delete":delete,
                        "files":{"nomifun.plugin.json":manifest.to_string(),
                            "ui/index.html":"<!doctype html><html><body><span id='count'>0</span></body></html>"}}))
                }
                2 => ("check", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                3 => ("preview", json!({"draft_id":state.draft,"expected_revision":state.revision})),
                4 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.revision,
                    "case_name":"initial-count","steps":[{"operation":"text","selector":"#count","value":"0"}]})),
                5 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.revision,
                    "case_name":"debug-count","steps":[{"operation":"text","selector":"#count","value":"1"}]})),
                6 => ("install", json!({"draft_id":state.draft,"expected_revision":state.revision,"verification_digest":state.digest})),
                7 => ("test_ui", json!({"draft_id":state.draft,"expected_revision":state.revision,
                    "case_name":"after-install","steps":[{"operation":"text","selector":"#count","value":"0"}]})),
                _ => panic!("the delivered turn must settle after the refused case: {last:?}"),
            };
            let needle = format!("Action: plugin.development/{action}.");
            let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                tool["function"]["description"].as_str().unwrap_or("").contains(&needle)).unwrap();
            state.phase += 1;
            plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("ui-after-install-{}",state.phase),
                "type":"function","function":{"name":tool["function"]["name"],"arguments":input.to_string()}}]}), "tool_calls")
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1",upstream.uri()), "Post-install UI case", "ui-after-install").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"Post-install UI case","model":{"provider_id":provider,"model":"ui-after-install"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"ui-after-install"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"ui-after-install-start","input":{"content":"Create and install the count UI, then run the second case.","plugin_delivery":{}}
    })).await;
    assert!(status.is_success(), "{accepted}");
    let leaked = tokio::time::timeout(Duration::from_secs(45), async {
        let mut queued = Vec::new();
        loop {
            let completed: i64 = nomifun_db::sqlx::query_scalar(
                "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/completed'"
            ).bind(session).fetch_one(harness.services.database.pool()).await.unwrap();
            if completed == 1 { break; }
            let id = script.lock().unwrap().draft.clone();
            if !id.is_empty() {
                let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{id}/authoring")).await;
                if let Some(commands) = report["data"]["commands"].as_array() {
                    for command in commands {
                        let case = command["case_name"].as_str().unwrap_or("");
                        let observations = match case {
                            "initial-count" => json!(["0"]),
                            "debug-count" => json!(["0"]),
                            "installed_surface_bridge" => json!([true]),
                            other => { queued.push(other.to_owned()); continue; }
                        };
                        let (status, acknowledged) = harness.json("POST", &format!("/api/plugin-drafts/{id}/ui-results"), json!({
                            "test_token":command["test_token"],"descriptor":command["descriptor"],"observations":observations
                        })).await;
                        assert_eq!(status, StatusCode::OK, "{acknowledged}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        queued
    }).await.expect("the delivered turn must complete after the refused case");
    assert!(leaked.is_empty(), "an after-install case must never queue a UI command: {leaked:?}");
    let refused = script.lock().unwrap().refused.clone().expect("the post-install test_ui result is recorded");
    assert!(refused.contains("PLUGIN_PREVIEW_REQUIRED") && refused.contains("After install"), "{refused}");
    let id = script.lock().unwrap().draft.clone();
    let (_, report) = harness.get_json(&format!("/api/plugin-drafts/{id}/authoring")).await;
    assert!(report["data"]["verification"]["delivery"].is_object(), "{report}");
    assert_eq!(report["data"]["verification"]["cases"]["debug-count"]["passed"],false,"the diagnostic result is retained: {report}");
    assert!(report["data"]["verification"]["plan"]["cases"]["debug-count"].is_null(),"the plan is unchanged: {report}");
    assert!(report["data"]["verification"]["cases"]["after-install"].is_null(),
        "the refused case must not be recorded: {report}");
    assert_eq!(report["data"]["verification"]["installed_observation"]["ui_ready"], true, "{report}");
    let pauses: Vec<Option<String>> = nomifun_db::sqlx::query_scalar(
        "SELECT native_pause_json FROM agent_turns WHERE session_id=?"
    ).bind(session).fetch_all(harness.services.database.pool()).await.unwrap();
    assert!(pauses.iter().all(Option::is_none), "a delivered turn does not pause: {pauses:?}");
}

/// Bare open honors the admitted expected_count before any plan exists, and
/// only an identical request replays into an existing draft.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bare_open_creates_each_admitted_output_and_only_identical_requests_reopen() {
    #[derive(Default)]
    struct Script { calls: usize, opens: Vec<Value> }
    let harness = Harness::new().await;
    let upstream = wiremock::MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::default()));
    let decisions = script.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let last = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "tool").and_then(|message| message["content"].as_str());
            let result: Value = last.and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
            let mut state = decisions.lock().unwrap();
            if let Some(id) = result["summary"]["draft_id"].as_str() {
                // A rejected final answer retries with the same tool history;
                // record each distinct open result only once.
                let open = json!({"draft_id":id,"notice":result["notice"],
                    "files":result["files"].as_array().map(|files|files.iter()
                        .filter_map(|file|file["path"].as_str()).collect::<Vec<_>>()).unwrap_or_default()});
                if state.opens.last() != Some(&open) { state.opens.push(open); }
            }
            let arguments = match state.calls {
                0 | 1 | 4 => "{}".to_owned(),
                2 | 3 => json!({"template":"agent.before_tool"}).to_string(),
                _ => return plugin_workflow_stream(
                    json!({"role":"assistant","content":"Opened the drafts."}), "stop"),
            };
            state.calls += 1;
            let tool = body["tools"].as_array().unwrap().iter().find(|tool|
                tool["function"]["description"].as_str().unwrap_or("")
                    .contains("Action: plugin.development/open.")).unwrap();
            plugin_workflow_stream(json!({"tool_calls":[{"index":0,"id":format!("open-{}",state.calls),
                "type":"function","function":{"name":tool["function"]["name"],"arguments":arguments}}]}), "tool_calls")
        }).mount(&upstream).await;
    let provider = create_chat_provider(&harness, &format!("{}/v1",upstream.uri()), "Open reuse", "open-reuse").await;
    let (status, preset) = harness.json("POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing":false,"display_name":"Open reuse","model":{"provider_id":provider,"model":"open-reuse"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let mut draft = preset["data"]["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([{
        "capability":{"id":"plugin.development"},"action_allowlist":nomifun_plugin_development::CREATE_ACTIONS
    }]);
    let (status, saved) = harness.json("POST", &format!("/api/agent-presets/{id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"],"draft":draft
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, session) = harness.json("POST", "/api/agent-sessions", json!({
        "preset_id":id,"required_modules":["plugin.development"],"model":{"provider_id":provider,"model":"open-reuse"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session = session["data"]["agent_session_id"].as_str().unwrap();
    let (status, accepted) = harness.json("POST", &format!("/api/agent-sessions/{session}/turns"), json!({
        "idempotency_key":"open-reuse-start","input":{"content":"Create two plugins.","plugin_delivery":{"expected_count":2}}
    })).await;
    assert!(status.is_success(), "{accepted}");
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let paused: Option<String> = nomifun_db::sqlx::query_scalar(
                "SELECT native_pause_json FROM agent_turns WHERE session_id=? AND native_pause_json IS NOT NULL"
            ).bind(session).fetch_optional(harness.services.database.pool()).await.unwrap().flatten();
            if let Some(paused) = paused {
                assert!(paused.contains("PLUGIN_DELIVERY_REQUIRED"), "{paused}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }).await.expect("undelivered drafts pause at the completion gate");
    let opens = script.lock().unwrap().opens.clone();
    assert_eq!(opens.len(), 5, "{opens:?}");
    let first = opens[0]["draft_id"].as_str().unwrap();
    let second = opens[1]["draft_id"].as_str().unwrap();
    let template = opens[2]["draft_id"].as_str().unwrap();
    assert_ne!(first, second, "the admitted second output is a new draft, not a replay");
    assert!(!opens[..3].iter().any(|open| !open["notice"].is_null()), "{opens:?}");
    assert!(![first, second].contains(&template));
    assert!(opens[2]["files"].as_array().unwrap().iter().any(|path| path == "service/main.mjs"),
        "the template draft carries its Service entrypoint: {opens:?}");
    assert_eq!(opens[3]["draft_id"].as_str().unwrap(), template,
        "an identical template open reopens its own draft");
    assert!(opens[3]["notice"].as_str().unwrap_or_default().contains("already has draft"), "{opens:?}");
    assert!([first, second].contains(&opens[4]["draft_id"].as_str().unwrap()),
        "a bare open beyond the admitted count reopens a bare draft");
    assert!(opens[4]["notice"].as_str().unwrap_or_default().contains("already has draft"), "{opens:?}");
    let (_, drafts) = harness.get_json("/api/plugin-drafts").await;
    let owned = drafts["data"]["drafts"].as_array().unwrap().iter()
        .filter(|draft| draft["source_conversation_id"] == session).count();
    assert_eq!(owned, 3, "only the three distinct requests create drafts: {drafts}");
}

fn plugin_workflow_stream(delta: Value, finish: &str) -> wiremock::ResponseTemplate {
    let frame = json!({"id":"plugin-workflow","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let done = json!({"id":"plugin-workflow","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
    wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
        .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
}

struct Harness {
    app: axum::Router,
    services: nomifun_app::compatibility::AppServices,
    token: String,
    csrf: String,
    files: tempfile::TempDir,
}

impl Harness {
    async fn new() -> Self {
        let (mut app, services) = common::build_local_trust_app(LOCAL_TRUST).await;
        let (token, csrf) =
            setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
        Self {
            app,
            services,
            token,
            csrf,
            files: tempfile::tempdir().unwrap(),
        }
    }

    async fn json(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut request = json_with_token(
            method,
            uri,
            body,
            &self.token,
            &self.csrf,
        );
        request
            .headers_mut()
            .insert("x-nomi-local-trust", LOCAL_TRUST.parse().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    async fn get_json(&self, uri: &str) -> (StatusCode, Value) {
        let mut request = get_with_token(uri, &self.token);
        request
            .headers_mut()
            .insert("x-nomi-local-trust", LOCAL_TRUST.parse().unwrap());
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    async fn get_public_bytes(&self, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        self.get_public_bytes_with_origin(uri, "null").await
    }

    async fn get_public_bytes_with_origin(
        &self,
        uri: &str,
        origin: &str,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let response = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header("host", "127.0.0.1:5197")
                    .header("origin", origin)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    fn package_path(&self, name: &str) -> PathBuf {
        self.files.path().join(name)
    }

    async fn install_directory(&self, source: &Path) -> Value {
        let source_path = source.to_string_lossy().into_owned();
        let (status, inspection) = self
            .json(
                "POST",
                "/api/plugins/import/inspect",
                json!({"source_path":source_path,"kind":"directory"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "inspection failed: {inspection}");
        let confirmation = inspection["data"]["permission_expansion"]["confirmation_id"]
            .as_str()
            .map(str::to_owned);
        let (status, installed) = self
            .json(
                "POST",
                "/api/plugins/import",
                json!({
                    "source_path": source.to_string_lossy(),
                    "kind":"directory",
                    "create_copy":false,
                    "permission_confirmation_id":confirmation,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "install failed: {installed}");
        assert_eq!(installed["data"]["result"]["outcome"], "installed");
        installed["data"]["result"]["plugin"].clone()
    }

    async fn open_surface(&self, plugin_id: &str, revision: u64) -> Value {
        let (status, response) = self
            .json(
                "POST",
                &format!("/api/plugins/{plugin_id}/surface/open"),
                json!({"expected_revision":revision}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "surface open failed: {response}");
        response["data"].clone()
    }

    async fn close_surface(&self, route: &str, descriptor: &Value) {
        let (status, response) = self
            .json(
                "POST",
                route,
                json!({
                    "surface_session_id":descriptor["surface_session_id"],
                    "surface_generation":descriptor["surface_generation"],
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "surface close failed: {response}");
    }

    async fn bridge(&self, route: &str, descriptor: &Value, target: Value) -> (StatusCode, Value) {
        self.json(
            "POST",
            route,
            json!({
                "plugin_id":descriptor["plugin_id"],
                "draft_id":descriptor["draft_id"],
                "artifact_digest":descriptor["artifact_digest"],
                "surface_session_id":descriptor["surface_session_id"],
                "surface_generation":descriptor["surface_generation"],
                "is_preview":descriptor["is_preview"],
                "request":{
                    "call_id":Uuid::now_v7().to_string(),
                    "target":target,
                }
            }),
        )
        .await
    }
}

async fn create_chat_provider(
    harness: &Harness,
    base_url: &str,
    name: &str,
    model: &str,
) -> String {
    let (status, provider) = harness
        .json(
            "POST",
            "/api/providers",
            json!({
                "platform":"openai",
                "name":name,
                "base_url":base_url,
                "auth_scheme":"bearer",
                "credentials":{"api_keys":["test-only"]},
                "enabled":true,
                "initial_model":{
                    "model":model,
                    "enabled":true,
                    "capabilities":[{
                        "task":"chat",
                        "traits":[],
                        "protocol":"openai.chat_text",
                        "connection_role":"default",
                        "provider_params":{}
                    }]
                }
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "provider create failed: {provider}");
    provider["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned()
}


fn write_ui_package(
    root: &Path,
    package_id: &str,
    secret_slots: &[&str],
    permissions: &[&str],
) {
    write_package_files(
        root,
        json!({
            "schema":"nomifun.plugin/v1",
            "id":package_id,
            "version":"1.0.0",
            "name":"Unified UI fixture",
            "description":"A UI-only Unified Plugin fixture.",
            "hostApi":">=1 <2",
            "entrypoints":{"ui":"ui/index.html"},
            "actions":{},
            "bindings":[],
            "dataVersion":0,
            "migrations":[],
            "configSchema":{"type":"object"},
            "secrets":secret_slots,
            "permissions":permissions,
        }),
        true,
        None,
    );
}

fn write_service_package(root: &Path, package_id: &str, mode: &str, mixed: bool) {
    let action_name = format!("{mode} echo");
    let service_mode = if mode == "headless" { "continuous" } else { "onDemand" };
    write_package_files(
        root,
        json!({
            "schema":"nomifun.plugin/v1",
            "id":package_id,
            "version":"1.0.0",
            "name":action_name,
            "description":"A dedicated Service Action fixture.",
            "hostApi":">=1 <2",
            "entrypoints": if mixed {
                json!({"ui":"ui/index.html","service":"service/main.mjs","serviceMode":service_mode})
            } else {
                json!({"service":"service/main.mjs","serviceMode":service_mode})
            },
            "actions":{
                "echo":{
                    "name":action_name,
                    "description":"Returns its input from the isolated Plugin Service.",
                    "input":{"type":"object"},
                    "output":{"type":"object"},
                    "effect":"read"
                }
            },
            "bindings":[{"point":"desktop.command","action":"echo"}],
            "dataVersion":0,
            "migrations":[],
            "configSchema":{"type":"object"},
            "secrets":[],
            "permissions":[],
        }),
        mixed,
        Some(format!(
            r#"export async function activate(ctx) {{
  return {{
    async invoke(action, input) {{
      if (action !== "echo") throw new Error("unsupported action");
      await ctx.storage.kv.set("last-input", input);
      return {{ mode: {mode:?}, input, pluginId: ctx.pluginId }};
    }}
  }};
}}
"#
        )),
    );
}

fn write_package_files(root: &Path, manifest: Value, with_ui: bool, service: Option<String>) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("nomifun.plugin.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    if with_ui {
        fs::create_dir_all(root.join("ui")).unwrap();
        fs::write(
            root.join("ui/index.html"),
            b"<!doctype html><html><head><title>Unified fixture</title><link rel=\"stylesheet\" href=\"./style.css\"><script type=\"module\" src=\"./app.js\"></script></head><body><main>fixture</main></body></html>",
        )
        .unwrap();
        fs::write(root.join("ui/app.js"), b"document.body.dataset.plugin = 'ready';\n").unwrap();
        fs::write(root.join("ui/style.css"), b"main { display: block; }\n").unwrap();
    }
    if let Some(service) = service {
        fs::create_dir_all(root.join("service")).unwrap();
        fs::write(root.join("service/main.mjs"), service).unwrap();
    }
}

fn installed_identity(detail: &Value) -> (String, u64) {
    (
        detail["summary"]["plugin_id"].as_str().unwrap().to_owned(),
        detail["summary"]["revision"].as_u64().unwrap(),
    )
}

fn assert_bridge_success(response: &Value) -> &Value {
    assert_eq!(response["data"]["outcome"], "success", "bridge failed: {response}");
    &response["data"]["result"]
}

fn surface_asset_path(descriptor: &Value) -> String {
    surface_asset_path_for(descriptor, descriptor["entrypoint"].as_str().unwrap())
}

fn surface_asset_path_for(descriptor: &Value, asset: &str) -> String {
    let owner = if descriptor["is_preview"] == true {
        format!(
            "/api/plugin-drafts/{}",
            descriptor["draft_id"].as_str().unwrap()
        )
    } else {
        format!(
            "/api/plugins/{}",
            descriptor["plugin_id"].as_str().unwrap()
        )
    };
    format!(
        "{owner}/surface/assets/{}/{}/{}/{}",
        descriptor["surface_session_id"].as_str().unwrap(),
        descriptor["surface_generation"].as_u64().unwrap(),
        descriptor["artifact_digest"].as_str().unwrap(),
        asset,
    )
}

async fn seed_surface_data(harness: &Harness, plugin_id: &str, descriptor: &Value) {
    let route = format!("/api/plugins/{plugin_id}/surface/bridge");
    for target in [
        json!({"target":"kv","request":{"operation":"set","key":"counter","value":7}}),
        json!({"target":"db","request":{"operation":"execute","sql":"CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)","parameters":[]}}),
        json!({"target":"db","request":{"operation":"execute","sql":"INSERT INTO notes (body) VALUES (?1)","parameters":["persisted"]}}),
        json!({"target":"files","request":{"operation":"write","path":"notes/state.txt","content_base64":base64::engine::general_purpose::STANDARD.encode(b"persisted-file"),"overwrite":false}}),
    ] {
        let (status, response) = harness.bridge(&route, descriptor, target).await;
        assert_eq!(status, StatusCode::OK);
        assert_bridge_success(&response);
    }
}

async fn assert_surface_data(harness: &Harness, plugin_id: &str, descriptor: &Value) {
    let route = format!("/api/plugins/{plugin_id}/surface/bridge");
    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"kv","request":{"operation":"get","key":"counter"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&response)["result"]["value"], 7);

    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"db","request":{"operation":"query","sql":"SELECT body FROM notes ORDER BY id","parameters":[]}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&response)["result"]["rows"][0]["body"],
        "persisted"
    );

    let (status, response) = harness
        .bridge(
            &route,
            descriptor,
            json!({"target":"files","request":{"operation":"read","path":"notes/state.txt"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let encoded = assert_bridge_success(&response)["result"]["content_base64"]
        .as_str()
        .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
        b"persisted-file"
    );
}

#[tokio::test]
async fn ui_only_surface_uses_one_sdk_persists_storage_and_library_pin_is_cas_guarded() {
    let harness = Harness::new().await;
    let package = harness.package_path("ui-only");
    write_ui_package(&package, "e2e.ui-only", &[], &[]);
    fs::create_dir_all(package.join("source")).unwrap();
    fs::write(package.join("source/original.ts"), "export const privateSource = true;").unwrap();
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    assert_eq!(detail["summary"]["has_service"], false);
    assert_eq!(detail["summary"]["runtime"]["state"], "stopped");

    let descriptor = harness.open_surface(&plugin_id, revision).await;
    let (status, headers, html) = harness
        .get_public_bytes(&surface_asset_path(&descriptor))
        .await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("http://127.0.0.1:5197"));
    assert_eq!(headers["access-control-allow-origin"], "null");
    let (_, hostile_headers, _) = harness
        .get_public_bytes_with_origin(
            &surface_asset_path(&descriptor),
            "https://attacker.invalid",
        )
        .await;
    assert_ne!(
        hostile_headers.get("access-control-allow-origin"),
        Some(&axum::http::HeaderValue::from_static("*"))
    );
    assert_ne!(
        hostile_headers.get("access-control-allow-origin"),
        Some(&axum::http::HeaderValue::from_static(
            "https://attacker.invalid"
        ))
    );
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("Object.defineProperty(window, 'nomi'"));
    let (status, _, script) = harness
        .get_public_bytes(&surface_asset_path_for(&descriptor, "ui/app.js"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(script).unwrap().contains("plugin = 'ready'"));
    assert_eq!(
        harness
            .get_public_bytes(&surface_asset_path_for(&descriptor, "source/original.ts"))
            .await
            .0,
        StatusCode::NOT_FOUND,
        "source/** must remain authoring-only and outside the Runtime asset surface"
    );
    assert_eq!(
        harness
            .get_public_bytes(&surface_asset_path_for(&descriptor, "nomifun.plugin.json"))
            .await
            .0,
        StatusCode::NOT_FOUND,
    );
    let mut wrong_owner = surface_asset_path_for(&descriptor, "ui/app.js");
    wrong_owner = wrong_owner.replacen(&plugin_id, &Uuid::now_v7().to_string(), 1);
    assert_eq!(harness.get_public_bytes(&wrong_owner).await.0, StatusCode::NOT_FOUND);
    seed_surface_data(&harness, &plugin_id, &descriptor).await;
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &descriptor,
        )
        .await;

    let reopened = harness.open_surface(&plugin_id, revision).await;
    assert_surface_data(&harness, &plugin_id, &reopened).await;
    let runtime = harness.services.plugin_service_runtime.get().unwrap();
    assert_eq!(
        runtime
            .observation(&PluginId::from(plugin_id.clone()))
            .await,
        PluginServiceObservation::Stopped,
        "UI-only Plugins must not own a Node process"
    );

    let (status, library) = harness.get_json("/api/plugins/library-state").await;
    assert_eq!(status, StatusCode::OK);
    let library_revision = library["data"]["revision"].as_u64().unwrap();
    assert_eq!(library["data"]["items"].as_array().unwrap().len(), 1);
    let (status, pinned) = harness
        .json(
            "PUT",
            "/api/plugins/library-state",
            json!({
                "expected_revision":library_revision,
                "collections":[],
                "items":[{"plugin_id":plugin_id,"pinned":true}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "pin failed: {pinned}");
    assert_eq!(pinned["data"]["items"][0]["pinned"], true);
    let (status, _) = harness
        .json(
            "PUT",
            "/api/plugins/library-state",
            json!({
                "expected_revision":library_revision,
                "collections":[],
                "items":[{"plugin_id":plugin_id,"pinned":false}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "stale library CAS must fail");
}

#[tokio::test]
async fn package_and_backup_round_trip_data_without_credential_plaintext() {
    const SECRET: &str = "E2E-CREDENTIAL-PLAINTEXT-MUST-NOT-EXPORT";
    let harness = Harness::new().await;
    let package = harness.package_path("backup-source");
    write_ui_package(&package, "e2e.backup", &["api_key"], &["network"]);
    let (status, first_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package.to_string_lossy(),"kind":"directory"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first_inspection["data"]["permission_expansion"]["added_secret_slots"][0],
        "api_key"
    );
    assert_eq!(
        first_inspection["data"]["permission_expansion"]["added_permissions"][0],
        "network"
    );
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    let surface = harness.open_surface(&plugin_id, revision).await;
    seed_surface_data(&harness, &plugin_id, &surface).await;
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &surface,
        )
        .await;

    let provider_id = Uuid::now_v7().to_string();
    let encrypted = nomifun_common::encrypt_string(
        &json!({"api_keys":[SECRET]}).to_string(),
        &harness.services.encryption_key,
    )
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO providers
         (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted,
          enabled, created_at, updated_at)
         VALUES (?, 'openai', 'Plugin E2E', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(&provider_id)
    .bind(encrypted)
    .execute(harness.services.database.pool())
    .await
    .unwrap();
    let (status, credential_references) =
        harness.get_json("/api/plugins/credentials").await;
    assert_eq!(status, StatusCode::OK);
    assert!(credential_references["data"].as_array().unwrap().iter().any(|reference| {
        reference["credential_id"] == format!("provider:{provider_id}")
            && reference["label"] == "Plugin E2E"
            && reference["enabled"] == true
    }));
    assert!(!credential_references.to_string().contains(SECRET));
    let (status, configured) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{plugin_id}/config"),
            json!({
                "expected_revision":revision,
                "config":{"restored":true},
                "credential_bindings":{"api_key":format!("provider:{provider_id}")},
                "grants":{"network":true}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure failed: {configured}");
    let mut configured_revision = configured["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();

    let manifest_path = package.join("nomifun.plugin.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["version"] = json!("1.0.1");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let (status, update_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package.to_string_lossy(),"kind":"directory"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update inspect failed: {update_inspection}");
    assert!(update_inspection["data"]["permission_expansion"].is_null());
    let (status, updated) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package.to_string_lossy(),
                "kind":"directory",
                "expected_plugin_revision":configured_revision,
                "create_copy":false
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "update import failed: {updated}");
    assert_eq!(
        updated["data"]["result"]["plugin"]["config"]["values"]["restored"],
        true,
        "Package updates must preserve the local instance config"
    );
    configured_revision = updated["data"]["result"]["plugin"]["summary"]["revision"]
        .as_u64()
        .unwrap();

    let package_zip = harness.files.path().join("plugin-package.zip");
    let backup_zip = harness.files.path().join("plugin-backup.zip");
    for (route, destination, body) in [
        (
            "export",
            &package_zip,
            json!({"expected_revision":configured_revision,"destination_path":package_zip.to_string_lossy(),"include_source":true}),
        ),
        (
            "backup",
            &backup_zip,
            json!({"expected_revision":configured_revision,"destination_path":backup_zip.to_string_lossy()}),
        ),
    ] {
        let (status, exported) = harness
            .json(
                "POST",
                &format!("/api/plugins/{plugin_id}/{route}"),
                body,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "export failed: {exported}");
        assert!(destination.is_file());
    }
    assert_zip_has_no_secret(&package_zip, SECRET, &provider_id);
    assert_zip_has_no_secret(&backup_zip, SECRET, &provider_id);
    let package_entries = zip_entries(&package_zip);
    assert!(!package_entries.iter().any(|name| name.ends_with("data.sqlite")));
    assert!(zip_entries(&backup_zip).iter().any(|name| name.ends_with("data.sqlite")));

    let (status, package_inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":package_zip.to_string_lossy(),"kind":"zip"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "package inspect failed: {package_inspection}");
    let (status, confirmation_required) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package_zip.to_string_lossy(),
                "kind":"zip",
                "create_copy":true
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        confirmation_required["data"]["result"]["outcome"],
        "confirmation_required"
    );
    let copy_confirmation = confirmation_required["data"]["result"]["confirmation"]
        ["confirmation_id"]
        .as_str()
        .unwrap();
    let (status, package_copy) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":package_zip.to_string_lossy(),
                "kind":"zip",
                "create_copy":true,
                "permission_confirmation_id":copy_confirmation
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "package re-import failed: {package_copy}");
    assert_eq!(package_copy["data"]["result"]["outcome"], "installed");
    assert_eq!(
        package_copy["data"]["result"]["plugin"]["config"]["values"],
        json!({}),
        "Package copies must not inherit another local instance's user config"
    );

    let backup_path = backup_zip.to_string_lossy().into_owned();
    let (status, inspection) = harness
        .json(
            "POST",
            "/api/plugins/import/inspect",
            json!({"source_path":backup_path,"kind":"backup"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "backup inspect failed: {inspection}");
    assert_eq!(inspection["data"]["backup"]["includes_data"], true);
    assert_eq!(
        inspection["data"]["backup"]["credential_slots_to_rebind"][0],
        "api_key"
    );
    let confirmation = inspection["data"]["permission_expansion"]["confirmation_id"]
        .as_str()
        .map(str::to_owned);
    let (status, imported) = harness
        .json(
            "POST",
            "/api/plugins/import",
            json!({
                "source_path":backup_zip.to_string_lossy(),
                "kind":"backup",
                "create_copy":true,
                "permission_confirmation_id":confirmation,
                "credential_bindings":{"api_key":format!("provider:{provider_id}")}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "backup restore failed: {imported}");
    let restored = &imported["data"]["result"]["plugin"];
    let (restored_id, restored_revision) = installed_identity(restored);
    assert_ne!(restored_id, plugin_id);
    assert_eq!(restored["config"]["values"]["restored"], true);
    assert_eq!(restored["credential_bindings"][0]["status"], "bound");
    assert_eq!(
        restored["credential_bindings"][0]["credential_id"],
        format!("provider:{provider_id}")
    );
    let restored_surface = harness.open_surface(&restored_id, restored_revision).await;
    assert_surface_data(&harness, &restored_id, &restored_surface).await;
}

#[tokio::test]
async fn headless_and_mixed_service_actions_publish_desktop_commands_and_lifecycle_revokes_them() {
    let harness = Harness::new().await;
    let headless_path = harness.package_path("headless");
    let mixed_path = harness.package_path("mixed");
    write_service_package(&headless_path, "e2e.headless", "headless", false);
    write_service_package(&mixed_path, "e2e.mixed", "mixed", true);
    let headless = harness.install_directory(&headless_path).await;
    let mixed = harness.install_directory(&mixed_path).await;
    let (headless_id, headless_revision) = installed_identity(&headless);
    let (mixed_id, mixed_revision) = installed_identity(&mixed);
    assert_eq!(headless["summary"]["has_ui"], false);
    assert_eq!(mixed["summary"]["has_ui"], true);
    assert_eq!(headless["summary"]["runtime"]["state"], "running");
    assert_eq!(mixed["summary"]["runtime"]["state"], "stopped");

    let (status, commands) = harness.get_json("/api/plugins/desktop/commands").await;
    assert_eq!(status, StatusCode::OK);
    let command_ids = commands["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["action_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let headless_action = format!("plugin:{headless_id}/echo");
    let mixed_action = format!("plugin:{mixed_id}/echo");
    assert!(command_ids.contains(&headless_action));
    assert!(command_ids.contains(&mixed_action));
    for (action, mode) in [(&headless_action, "headless"), (&mixed_action, "mixed")] {
        let (status, invoked) = harness
            .json(
                "POST",
                "/api/plugins/desktop/commands/invoke",
                json!({"action_id":action,"input":{"value":mode}}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "command failed: {invoked}");
        assert_eq!(invoked["data"]["mode"], mode);
        assert_eq!(invoked["data"]["input"]["value"], mode);
    }

    let mixed_surface = harness.open_surface(&mixed_id, mixed_revision).await;
    let (status, action_result) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"actions","action":"echo","input":{"value":"surface"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&action_result)["result"]["mode"], "mixed");
    let (status, shared_storage) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"kv","request":{"operation":"get","key":"last-input"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&shared_storage)["result"]["value"]["value"],
        "surface",
        "UI and Service must observe the same generation DataRoot"
    );

    let (status, disabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{mixed_id}/enabled"),
            json!({"expected_revision":mixed_revision,"enabled":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "disable failed: {disabled}");
    let (status, _) = harness
        .bridge(
            &format!("/api/plugins/{mixed_id}/surface/bridge"),
            &mixed_surface,
            json!({"target":"config"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "disable must revoke Surface admission");
    assert_command_absent(&harness, &mixed_action).await;

    let (status, enabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{mixed_id}/enabled"),
            json!({"expected_revision":2,"enabled":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "re-enable failed: {enabled}");
    let (status, trashed) = harness
        .json(
            "POST",
            &format!("/api/plugins/{mixed_id}/trash"),
            json!({"expected_revision":3}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "trash failed: {trashed}");
    assert_command_absent(&harness, &mixed_action).await;
    assert_eq!(
        harness
            .services
            .plugin_service_runtime
            .get()
            .unwrap()
            .observation(&PluginId::from(mixed_id.clone()))
            .await,
        PluginServiceObservation::Stopped,
        "Trash must stop the mixed Plugin process"
    );
    let mixed_root = harness.services.plugin_data_roots.root().join(&mixed_id);
    assert!(mixed_root.exists());
    let (status, deleted) = harness
        .json(
            "DELETE",
            &format!("/api/plugins/{mixed_id}"),
            json!({"expected_revision":4,"acknowledge_permanent_delete":true}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "permanent delete failed: {deleted}");
    assert!(!mixed_root.exists());
    assert_eq!(
        harness
            .get_json(&format!("/api/plugins/{mixed_id}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (status, disabled) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{headless_id}/enabled"),
            json!({"expected_revision":headless_revision,"enabled":false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "headless disable failed: {disabled}");
    assert_command_absent(&harness, &headless_action).await;
    assert_eq!(
        harness
            .services
            .plugin_service_runtime
            .get()
            .unwrap()
            .observation(&PluginId::from(headless_id))
            .await,
        PluginServiceObservation::Stopped,
        "Disable must stop the headless Plugin process"
    );
}

async fn assert_command_absent(harness: &Harness, action_id: &str) {
    let (status, commands) = harness.get_json("/api/plugins/desktop/commands").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        commands["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| command["action_id"] != action_id)
    );
    let (status, _) = harness
        .json(
            "POST",
            "/api/plugins/desktop/commands/invoke",
            json!({"action_id":action_id,"input":{}}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "revoked Actions remain fenced tombstones, never invokable"
    );
}

#[tokio::test]
async fn draft_preview_uses_temporary_data_root_without_polluting_installed_data() {
    let harness = Harness::new().await;
    let package = harness.package_path("preview-base");
    write_ui_package(&package, "e2e.preview", &[], &[]);
    let detail = harness.install_directory(&package).await;
    let (plugin_id, revision) = installed_identity(&detail);
    let production = harness.open_surface(&plugin_id, revision).await;
    let (status, response) = harness
        .bridge(
            &format!("/api/plugins/{plugin_id}/surface/bridge"),
            &production,
            json!({"target":"kv","request":{"operation":"set","key":"scope","value":"production"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_bridge_success(&response);
    harness
        .close_surface(
            &format!("/api/plugins/{plugin_id}/surface/close"),
            &production,
        )
        .await;

    let (status, draft) = harness
        .json(
            "POST",
            "/api/plugin-drafts",
            json!({"plugin_id":plugin_id,"expected_plugin_revision":revision}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "draft create failed: {draft}");
    let draft_id = draft["data"]["summary"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, preview) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/preview"),
            json!({"expected_revision":1,"config":{},"access":{"permissions":[],"credential_bindings":{}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "preview failed: {preview}");
    let descriptor = preview["data"]["descriptor"].clone();
    let (status, response) = harness
        .bridge(
            &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
            &descriptor,
            json!({"target":"kv","request":{"operation":"set","key":"scope","value":"preview"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_bridge_success(&response);
    let preview_revision = preview["data"]["draft_revision"].as_u64().unwrap();
    let (status, reloaded_preview) = harness
        .json(
            "POST",
            &format!("/api/plugin-drafts/{draft_id}/preview"),
            json!({"expected_revision":preview_revision,"config":{},"access":{"permissions":[],"credential_bindings":{}}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "Preview reload failed: {reloaded_preview}");
    let reloaded_descriptor = reloaded_preview["data"]["descriptor"].clone();
    assert_eq!(
        reloaded_descriptor["surface_session_id"],
        descriptor["surface_session_id"],
    );
    assert!(
        reloaded_descriptor["surface_generation"].as_u64().unwrap()
            > descriptor["surface_generation"].as_u64().unwrap()
    );
    assert_eq!(
        harness
            .bridge(
                &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
                &descriptor,
                json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
    );
    let (status, response) = harness
        .bridge(
            &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
            &reloaded_descriptor,
            json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assert_bridge_success(&response)["result"]["value"], "preview");
    let (status, configured) = harness
        .json(
            "PUT",
            &format!("/api/plugins/{plugin_id}/config"),
            json!({
                "expected_revision":revision,
                "config":{},
                "credential_bindings":{},
                "grants":{}
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "configure failed: {configured}");
    let configured_revision = configured["data"]["summary"]["revision"]
        .as_u64()
        .unwrap();
    assert_eq!(
        harness
            .bridge(
                &format!("/api/plugin-drafts/{draft_id}/surface/bridge"),
                &reloaded_descriptor,
                json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
        "installed Plugin lifecycle changes must revoke related Preview admission"
    );

    let reopened = harness.open_surface(&plugin_id, configured_revision).await;
    let (status, response) = harness
        .bridge(
            &format!("/api/plugins/{plugin_id}/surface/bridge"),
            &reopened,
            json!({"target":"kv","request":{"operation":"get","key":"scope"}}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        assert_bridge_success(&response)["result"]["value"],
        "production"
    );
    let preview_staging = harness
        .services
        .plugin_data_roots
        .root()
        .join(&draft_id)
        .join("staging");
    assert!(preview_staging.is_dir());
    assert_eq!(fs::read_dir(preview_staging).unwrap().count(), 0);
}

fn zip_entries(path: &Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect()
}

fn assert_zip_has_no_secret(path: &Path, plaintext: &str, credential_id: &str) {
    let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert!(
            !bytes.windows(plaintext.len()).any(|window| window == plaintext.as_bytes()),
            "{} leaked credential plaintext",
            entry.name()
        );
        assert!(
            !bytes
                .windows(credential_id.len())
                .any(|window| window == credential_id.as_bytes()),
            "{} leaked a Host Credential identity",
            entry.name()
        );
    }
}
