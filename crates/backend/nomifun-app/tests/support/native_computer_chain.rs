//! Opt-in ordinary native acceptance; only the model HTTP provider is scripted.
//! The fixture executable, worker and fresh artifact directory are explicit.
//!
//! Build the owned fixture from the repository root with the existing .NET
//! Framework compiler (no installation); choose an output in your test Temp root:
//! ```powershell
//! & "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /target:winexe /out:PATH_TO_TEST_TEMP/NomiFunLifetimeFixture.exe /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll crates/backend/nomifun-app/tests/fixtures/owned-computer-window.cs
//! ```
//! Set NOMIFUN_COMPUTER_FIXTURE_EXE to that executable, NOMIFUN_COMPUTER_WORKER
//! to the test worker, and NOMIFUN_COMPUTER_ARTIFACT_DIR to a fresh directory.
//! Fixture source SHA-256: fa1087933afe899e07f02f612b732d2afb236286c7d4f19334f4fde5bb7b2144.
use super::*;
use futures_util::FutureExt;
use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

const MARKER: &str = "OWNED_COMPUTER_CHAIN_VERIFIED";
struct CloseOwnedWindow(PathBuf);
impl Drop for CloseOwnedWindow {
    fn drop(&mut self) {
        let _ = std::fs::write(self.0.join("close.request"), b"close only this fixture");
    }
}
// Model text is never evidence. Extract only explicit JSON tool payloads from
// the real OpenAI wire text parts, including the structured-content mirror.
fn json_payload(value: &Value, field: &str) -> Option<Value> {
    if value.get(field).is_some() {
        return Some(value.clone());
    }
    if let Some(value) = value
        .get("structuredContent")
        .filter(|value| value.get(field).is_some())
    {
        return Some(value.clone());
    }
    match value {
        Value::String(text) => text
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|value| json_payload(&value, field)),
        Value::Array(parts) => parts
            .iter()
            .rev()
            .find_map(|part| json_payload(part, field)),
        Value::Object(part) => part.get("text").and_then(|text| json_payload(text, field)),
        _ => None,
    }
}
fn observed(body: &Value, id: &str, field: &str) -> Value {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .filter(|message| message["role"] == "tool" && message["tool_call_id"] == id)
        .find_map(|message| json_payload(&message["content"], field))
        .expect("real structured tool observation")
}
fn emit(id: &str, action: &str, args: Value) -> wiremock::ResponseTemplate {
    let name = &nomifun_computer::surface::tool(action)
        .expect("frozen action")
        .model_tool;
    stream(Some((id, name, args)), "")
}
fn write(root: &Path, name: &str, value: &Value) {
    std::fs::write(root.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn digest(path: &Path) -> String {
    Sha256::digest(std::fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "opens only an explicitly supplied task-owned WinForms fixture; requires worker/fixture/artifact environment paths"]
async fn dedicated_window_token_action_crosses_http_runtime_kernel_and_canonical_history() {
    let fixture = PathBuf::from(
        std::env::var_os("NOMIFUN_COMPUTER_FIXTURE_EXE")
            .expect("explicit dedicated fixture executable"),
    );
    let worker = PathBuf::from(
        std::env::var_os("NOMIFUN_COMPUTER_WORKER").expect("explicit test worker executable"),
    );
    let root = PathBuf::from(
        std::env::var_os("NOMIFUN_COMPUTER_ARTIFACT_DIR").expect("new owned artifact directory"),
    );
    assert!(
        fixture.is_absolute()
            && fixture.is_file()
            && worker.is_absolute()
            && worker.is_file()
            && root.is_absolute()
    );
    std::fs::create_dir(&root).expect("artifact directory must be fresh");
    let close = CloseOwnedWindow(root.clone());
    let upstream = wiremock::MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let native_steps = Arc::new(AtomicUsize::new(0));
    let auxiliary_requests = Arc::new(AtomicUsize::new(0));
    let searches = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let request_trace = Arc::new(Mutex::new(Vec::new()));
    let target = Arc::new(Mutex::new(None::<(u64, u64)>));
    let script_root = root.clone();
    let script_fixture = fixture.clone();
    let seen = requests.clone();
    let stage = native_steps.clone();
    let auxiliary = auxiliary_requests.clone();
    let discovered = searches.clone();
    let trace = request_trace.clone();
    let active = target.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request:&wiremock::Request|{
            let body:Value=serde_json::from_slice(&request.body).unwrap();
            let request_number=seen.fetch_add(1,Ordering::SeqCst)+1;
            let Some(tools)=body["tools"].as_array().filter(|tools|!tools.is_empty()) else {
                // The selected ToolSearch policy ranks in the pure Kernel.
                // Never guess a response to a different auxiliary protocol.
                auxiliary.fetch_add(1,Ordering::SeqCst);
                trace.lock().unwrap().push(json!({"request":request_number,"kind":"unsupported_auxiliary","native_stage":stage.load(Ordering::SeqCst)}));
                panic!("unexpected no-tools auxiliary model request; no native stage advanced");
            };
            let step=stage.load(Ordering::SeqCst);
            let action=match step {0=>Some("computer/launch_app"),1|3=>Some("computer/get_window_state"),2=>Some("computer/click"),4=>None,_=>panic!("unexpected additional native stage")};
            if let Some(action)=action {
                let name=&nomifun_computer::surface::tool(action).unwrap().model_tool;
                let visible=tools.iter().any(|tool|tool["function"]["name"]==*name);
                if !visible {
                    assert!(tools.iter().any(|tool|tool["function"]["name"]=="ToolSearch"),"an omitted schema must have the actual discovery entry point");
                    assert!(body.to_string().contains(action),"search must use an actually advertised frozen action ID");
                    assert!(discovered.lock().unwrap().insert(step),"the exact ToolSearch result failed to reveal its schema on the next model step");
                    trace.lock().unwrap().push(json!({"request":request_number,"kind":"tool_search","native_stage":step,"action":action}));
                    return stream(Some((&format!("owned-discover-{step}"),"ToolSearch",json!({"query":action}))),"");
                }
                assert_eq!(body["parallel_tool_calls"],false,"Computer calls must be serial at the actual visible provider route");
            }
            trace.lock().unwrap().push(json!({"request":request_number,"kind":if action.is_some(){"native"}else{"final"},"native_stage":step,"action":action}));
            stage.fetch_add(1,Ordering::SeqCst);
            match step{
                0=>{
                    emit("owned-launch","computer/launch_app",json!({"launch_path":script_fixture,"additional_arguments":["functional",script_root],"start_minimized":false}))
                },
                1=>{
                    let launch=observed(&body,"owned-launch","pid");
                    let pid=launch["pid"].as_u64().unwrap();let window=launch["windows"][0]["window_id"].as_u64().unwrap();
                    *active.lock().unwrap()=Some((pid,window));
                    emit("owned-read","computer/get_window_state",json!({"pid":pid,"window_id":window,"include_screenshot":false,"tree_format":"elements","max_elements":100}))
                },
                2=>{
                    let read=observed(&body,"owned-read","elements");
                    let control=read["elements"].as_array().unwrap().iter().find(|item|item["label"]=="Owned two").unwrap();
                    emit("owned-click","computer/click",json!({"element_token":control["element_token"],"delivery_mode":"background"}))
                },
                3=>{
                    let (pid,window)=active.lock().unwrap().unwrap();
                    emit("owned-verify","computer/get_window_state",json!({"pid":pid,"window_id":window,"include_screenshot":false,"tree_format":"elements","max_elements":100}))
                },
                4=>{
                    let read=observed(&body,"owned-verify","elements");
                    assert!(read["elements"].as_array().unwrap().iter().any(|item|item["label"]=="Owned calculation"&&item["value"]=="2"));
                    stream(None,MARKER)
                },
                _=>panic!("unexpected retry or additional model call in the ordinary Computer chain"),
            }
        }).mount(&upstream).await;
    let config = AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(TRUST.into()),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path())
        .await
        .unwrap();
    let mut app = AppServices::from_config(database, &config).await.unwrap();
    app.browser_resources = Some(Arc::new(
        nomifun_browser_platform::workspace::BrowserResourceService::new(Arc::new(
            BindingOnlyBrowser(Arc::new(AtomicUsize::new(0))),
        )),
    ));
    let router = create_router(&app).await;
    let outcome=std::panic::AssertUnwindSafe(async{
        let provider=call(&router,"POST","/api/providers",json!({"platform":"custom","name":"owned Computer scripted test","base_url":format!("{}/v1",upstream.uri()),"auth_scheme":"bearer","credentials":{"api_keys":["isolated-test-only"]},"enabled":true,
            "initial_model":{"model":"owned-computer-script","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","context_limit":128000,"output_limit":4096}]}})).await;
        let model=json!({"provider_id":provider["provider_id"],"model":"owned-computer-script"});
        let preset=call(&router,"POST","/api/agent-presets/from-template/assistant.general",json!({"reuse_existing":false,"display_name":"Owned Computer chain","model":model})).await;
        let session=call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,"resource_selections":[
            {"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"},
            {"resource_kind":"browser","resource_id":"managed-browser"},{"resource_kind":"computer","resource_id":"local-desktop"},{"resource_kind":"scheduler","resource_id":"installation-scheduler"}]})).await;
        let id=session["agent_session_id"].as_str().unwrap();
        let accepted=call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"owned-computer-chain","input":{"content":"In the dedicated test window, click Owned two once and verify its calculator displays 2. Do not interact with any other application."}})).await;
        let store=AgentSessionStore::from_pool(app.database.pool().clone()).await.unwrap();
        let sid=nomifun_agent_contracts::AgentSessionId::from(id.to_owned());
        let operation=nomifun_agent_contracts::OperationId::from(accepted["operation_id"].as_str().unwrap().to_owned());
        let receipt=tokio::time::timeout(Duration::from_secs(60),async{loop{let receipt=store.read_turn_receipt(&sid,&operation).await.unwrap();if matches!(receipt.status,TurnReceiptStatus::Completed|TurnReceiptStatus::Failed|TurnReceiptStatus::Cancelled){break receipt;}tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
        assert_eq!(receipt.status,TurnReceiptStatus::Completed);
        let effects=store.list_effects(&sid).await.unwrap().into_iter().filter(|effect|effect.turn_id==operation&&effect.capability_module.as_ref()=="computer").collect::<Vec<_>>();
        assert_eq!(effects.len(),2,"one launch and one token input effect");
        assert!(effects.iter().all(|effect|effect.state==AgentEffectState::Returned));
        assert!(effects.iter().any(|effect|effect.action_id.as_ref()=="computer/launch_app")&&effects.iter().any(|effect|effect.action_id.as_ref()=="computer/click"));
        let history=call(&router,"GET",&format!("/api/agent-sessions/{id}/messages?after_seq=0&limit=100"),Value::Null).await;
        assert!(history.to_string().contains(MARKER)&&history.to_string().contains("computer_click"));
        let (pid,window)=target.lock().unwrap().unwrap();
        let report=json!({"ok":true,"evidence_kind":"real_product_native_scripted_provider","session_id":id,"app_pid":pid,"window_id":window,"model_requests":requests.load(Ordering::SeqCst),"tool_search_calls":searches.lock().unwrap().len(),"auxiliary_model_requests":auxiliary_requests.load(Ordering::SeqCst),"native_steps_and_final":native_steps.load(Ordering::SeqCst),"effects":effects,"screenshots":0,"clipboard_calls":0,"pixel_calls":0,"model_was_scripted":true,"worker_sha256":digest(&worker),"fixture_sha256":digest(&fixture),"source_sha256":digest(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/cua-driver/SOURCE.json"))});
        write(&root,"canonical-effects.json",&serde_json::to_value(&effects).unwrap());write(&root,"history.json",&history);
        assert_eq!(native_steps.load(Ordering::SeqCst),5);
        assert_eq!(requests.load(Ordering::SeqCst),5+searches.lock().unwrap().len()+auxiliary_requests.load(Ordering::SeqCst));
        report
    }).catch_unwind().await;
    write(
        &root,
        "model-request-trace.json",
        &json!(request_trace.lock().unwrap().clone()),
    );
    // Request the fixture's own normal Close even if any assertion panics.
    drop(close);
    let shutdown = app.shutdown_nomi_core_host().await;
    let closed = if root.join("ready.json").exists() {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !root.join("exit.json").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok()
    } else {
        true
    };
    // This cached, read-only status endpoint needs neither live storage nor
    // native startup. Its stopped state comes from the actual Computer owner.
    let status = call(&router, "GET", "/api/computer/status", Value::Null).await;
    let exit = std::fs::read(root.join("exit.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let expected_pid = target.lock().unwrap().map(|(pid, _)| pid);
    let graceful = closed
        && expected_pid.is_none_or(|pid| {
            exit.as_ref().is_some_and(|exit| {
                exit["pid"] == pid && exit["graceful"] == true && exit["control_file"] == true
            })
        });
    let cleanup = json!({"normal_shutdown_succeeded":shutdown.is_ok(),"worker_running":status["running"],"fixture_form_closed_gracefully":graceful});
    write(&root, "cleanup-evidence.json", &cleanup);
    match outcome {
        Err(panic) => std::panic::resume_unwind(panic),
        Ok(mut report) => {
            shutdown.expect("normal App owners must retire before success");
            assert_eq!(
                status["running"], false,
                "Computer must actually be stopped"
            );
            assert!(
                graceful,
                "owned fixture must report its exact PID and graceful control-file closure"
            );
            report["cleanup"] = cleanup;
            write(&root, "product-chain-report.json", &report);
        }
    }
}
