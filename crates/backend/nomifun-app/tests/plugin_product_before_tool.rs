//! Real Product publication and Node checks around a real Nomi Product tool.
//! Only the model transport is simulated; invocation owners and effects are real.
use super::*;
use nomifun_ai_agent::tool_middleware as middleware;
use nomifun_app::compatibility::AppServices;
use std::{path::Path, time::Duration};

const TARGET_ACTION: &str = "fixture.before-tool.target";
const TARGET_SOURCE: &str = r#"
import {appendFile} from 'node:fs/promises';
export async function start() {
  return {async invoke({method,payload}) {
    if (method !== 'fixture.before-tool.target') throw new Error('wrong target');
    if (payload.value !== 7 || payload.password !== 'secret-fixture-value') throw new Error('gate mutated target arguments');
    await appendFile(TARGET_LOG, payload.mode + '\n');
    return {executed: payload.mode};
  }, async dispose() {}};
}
"#;
const HOOK_SOURCE: &str = r#"
import {appendFile} from 'node:fs/promises';
export async function start() {
  return {async invoke({method,payload,signal}) {
    if (method !== 'agent.before_tool' || payload.phase !== 'before_tool') throw new Error('wrong phase');
    if (Object.keys(payload).sort().join(',') !== 'arguments,invocation_id,phase,redacted,tool_call_id,tool_name') throw new Error('unexpected authority');
    if (!payload.invocation_id || !payload.tool_call_id || !payload.tool_name) throw new Error('missing identity');
    if (!payload.redacted || payload.arguments.password !== '[REDACTED]') throw new Error('secret not masked');
    const mode = payload.arguments.mode;
    await appendFile(HOOK_LOG, JSON.stringify({tag:HOOK_TAG,mode,invocation_id:payload.invocation_id,tool_call_id:payload.tool_call_id}) + '\n');
    if (mode === 'invalid') return {decision:'allow',patch:{value:999}};
    if (mode === 'wait' || mode === 'cancel') return new Promise((resolve,reject) => {
      signal.addEventListener('abort', async () => {
        await appendFile(ABORT_LOG, mode + '\n');
        reject(new Error('cancelled'));
      }, {once:true});
    });
    if (mode === 'deny' || mode.startsWith('order')) return {decision:'deny',reason:'DENIED_BY_' + HOOK_TAG};
    return {decision:'allow'};
  }, async dispose() {}};
}
"#;

fn schema_ref(name: &str, schema: &Value) -> CanonicalSchemaRef {
    format!(
        "schema://fixture.before-tool/{name}@1#{}",
        digest_payload(schema).unwrap().as_ref()
    )
    .into()
}
fn target_schemas() -> BTreeMap<CanonicalSchemaRef, StrictJsonValue> {
    let input = json!({"type":"object","additionalProperties":false,"required":["mode","value","password"],
        "properties":{"mode":{"type":"string"},"value":{"type":"integer"},"password":{"type":"string"}}});
    let output = json!({"type":"object","additionalProperties":false,"required":["executed"],"properties":{"executed":{"type":"string"}}});
    [
        (schema_ref("input", &input), StrictJsonValue(input)),
        (schema_ref("output", &output), StrictJsonValue(output)),
    ]
    .into()
}
fn target_capability(id: &str) -> CapabilityManifest {
    let mut manifest = discovery_capability(id);
    manifest.id = format!("plugin.{id}.before-tool-target").into();
    manifest.contribution_id = format!("capability:{}", manifest.id.as_ref()).into();
    manifest.display.description = "BEFORE_TOOL_TARGET_FIXTURE".into();
    let schemas = target_schemas();
    manifest.contributions.actions = vec![CapabilityActionDescriptor {
        action_id: TARGET_ACTION.into(),
        input_schema: schemas
            .keys()
            .find(|key| key.as_ref().contains("/input@"))
            .unwrap()
            .clone(),
        output_schema: schemas
            .keys()
            .find(|key| key.as_ref().contains("/output@"))
            .unwrap()
            .clone(),
        effect_class: EffectClass::WriteReversible,
        presentation: ToolPresentationKind::FunctionTool,
    }];
    manifest
}
fn hook_capability(id: &str) -> CapabilityManifest {
    let mut manifest = discovery_capability(id);
    manifest.id = format!("plugin.{id}.before-tool").into();
    manifest.contribution_id = format!("capability:{}", manifest.id.as_ref()).into();
    manifest.kind = CapabilityKind::TurnMiddleware;
    manifest.display.name = "Before tool check fixture".into();
    manifest.contributions.actions = vec![middleware::before_action()];
    manifest
}
fn lines(path: &Path) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("cannot read fixture effect log: {error}"),
    }
}
fn hook_log(path: &Path) -> Vec<Value> {
    lines(path)
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn ordinary_before_tool_template_publishes_through_the_user_save_action() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let draft = data(
        &router,
        "POST",
        "/api/plugins/drafts/from-template/before-tool",
        Value::Null,
    )
    .await;
    let id = draft["id"].as_str().unwrap();
    let path = format!("/api/plugins/drafts/{id}");
    let (status, pending) = request(
        &router,
        "POST",
        &format!("{path}/save"),
        json!({"expected_revision":draft["revision"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{pending}");
    assert_eq!(pending["code"], "PLUGIN_SERVICE_TEST_INPUT_REQUIRED");
    let details = &pending["details"];
    assert_eq!(details["draft_id"], id);
    let preserved = data(&router, "GET", &path, Value::Null).await;
    assert_eq!(preserved["status"], "ready");
    assert!(
        preserved["error"].is_null(),
        "pending confirmation is not save_failed"
    );
    assert_eq!(preserved["revision"], details["expected_revision"]);
    let product_path = format!(
        "/api/plugins/runtimes/{}",
        preserved["plugin_id"].as_str().unwrap()
    );
    let workshop = data(
        &router,
        "GET",
        &format!("{product_path}/workshop"),
        Value::Null,
    )
    .await;
    assert!(workshop["plugin"]["releases"]["active"].is_null());
    assert_ne!(workshop["plugin"]["lifecycle"], "enabled");
    assert_eq!(workshop["ready"]["test"]["status"], "needs_test_input");
    assert_eq!(
        workshop["ready"]["test"]["receipt_id"],
        details["receipt_id"]
    );
    assert_eq!(
        workshop["ready"]["release"]["release_digest"],
        details["release_digest"]
    );
    let acknowledgement =
        json!({"release_digest":details["release_digest"],"receipt_id":details["receipt_id"]});
    for field in ["release_digest", "receipt_id"] {
        let mut forged = acknowledgement.clone();
        forged[field] = json!("forged");
        let (status, error) = request(
            &router,
            "POST",
            &format!("{path}/save"),
            json!({
                "expected_revision":details["expected_revision"],"acknowledge_service_test":forged
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        let unchanged = data(
            &router,
            "GET",
            &format!("{product_path}/workshop"),
            Value::Null,
        )
        .await;
        assert!(unchanged["plugin"]["releases"]["active"].is_null());
        assert_eq!(
            unchanged["ready"]["test"]["receipt_id"], details["receipt_id"],
            "forged acknowledgement must not rerun the check"
        );
    }
    // Fetching a newer draft revision cannot legitimize an earlier acknowledgement.
    let changed = data(
        &router,
        "POST",
        &format!("{path}/cancel"),
        json!({"expected_revision":details["expected_revision"]}),
    )
    .await;
    let (status, error) = request(
        &router,
        "POST",
        &format!("{path}/save"),
        json!({
            "expected_revision":changed["revision"],"acknowledge_service_test":acknowledgement
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    // A new ordinary save yields a new reviewable receipt; it never auto-accepts it.
    let (status, renewed) = request(
        &router,
        "POST",
        &format!("{path}/save"),
        json!({"expected_revision":changed["revision"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{renewed}");
    assert_eq!(renewed["code"], "PLUGIN_SERVICE_TEST_INPUT_REQUIRED");
    let fresh = &renewed["details"];
    assert_ne!(fresh["receipt_id"], details["receipt_id"]);
    assert_eq!(fresh["release_digest"], details["release_digest"],
        "rechecking an unchanged draft must reuse its exact Ready Release");
    let saved = data(&router, "POST", &format!("{path}/save"), json!({
        "expected_revision":fresh["expected_revision"],
        "acknowledge_service_test":{"release_digest":fresh["release_digest"],"receipt_id":fresh["receipt_id"]}
    })).await;
    assert_eq!(saved["plugin"]["lifecycle"], "enabled");
    assert_eq!(
        saved["plugin"]["releases"]["active"]["release_digest"],
        fresh["release_digest"]
    );
    let final_draft = data(&router, "GET", &path, Value::Null).await;
    assert_eq!(final_draft["status"], "saved");
    assert!(final_draft.get("service_test_confirmation").is_none());
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

struct Fixture {
    router: axum::Router,
    services: AppServices,
    _root: tempfile::TempDir,
    target_log: std::path::PathBuf,
    hook_log: std::path::PathBuf,
    abort_log: std::path::PathBuf,
    upstream: wiremock::MockServer,
    preset: String,
    hooks: Vec<(String, String, String)>, // capability id, Product endpoint, tag
}
impl Fixture {
    async fn new(tags: &[&str]) -> Self {
        let (router, services) = common::build_local_trust_app(TRUST).await;
        let root = tempfile::Builder::new()
            .prefix("before-tool Unicode 空格 ")
            .tempdir()
            .unwrap();
        let target_log = root.path().join("target.log");
        let hook_log = root.path().join("hook.log");
        let abort_log = root.path().join("abort.log");
        let source =
            TARGET_SOURCE.replace("TARGET_LOG", &serde_json::to_string(&target_log).unwrap());
        let (target_id, _) =
            publish_source(&router, &source, target_capability, target_schemas()).await;
        let mut hooks = Vec::new();
        for tag in tags {
            let source = HOOK_SOURCE
                .replace("HOOK_LOG", &serde_json::to_string(&hook_log).unwrap())
                .replace("HOOK_TAG", &serde_json::to_string(tag).unwrap())
                .replace("ABORT_LOG", &serde_json::to_string(&abort_log).unwrap());
            let (id, base) =
                publish_source(&router, &source, hook_capability, middleware::schemas()).await;
            hooks.push((
                hook_capability(&id).id.as_ref().to_owned(),
                base,
                (*tag).to_owned(),
            ));
        }
        let upstream = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(|request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            let messages = body["messages"].as_array().unwrap();
            let known_modes = ["allow", "deny", "baseline", "bad-schema", "order-original",
                "order-new", "order-frozen", "invalid", "wait", "cancel", "disabled"];
            let (user_index, mode) = messages.iter().enumerate().rev().find_map(|(index, message)| {
                let content = message["content"].as_str()?;
                known_modes.contains(&content).then_some((index, content))
            }).expect("fixture input must remain in model context");
            let tool_results = messages[user_index + 1..].iter()
                .filter(|message| message["role"] == "tool").collect::<Vec<_>>();
            let requirement_id = format!("fixture-{mode}");
            let requirements = json!([{
                "id":requirement_id,
                "description":format!("Process the {mode} fixture request through the selected target"),
                "source":{"input":0,"quote":mode}
            }]);
            let result_content = |prefix: &str| tool_results.iter().find(|message|
                message["tool_call_id"].as_str().is_some_and(|id| id.starts_with(prefix)))
                .and_then(|message| message["content"].as_str()).map(str::to_owned);
            let control_call = |id: String, name: &str, arguments: Value| json!({"choices":[{"index":0,
                "delta":{"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function","function":{
                    "name":name,"arguments":arguments.to_string()
                }}]},"finish_reason":"tool_calls"}]});
            let frame = if tool_results.is_empty() {
                let tools = body["tools"].as_array().expect("selected target must reach real model request");
                assert!(tools.iter().all(|t| !t.to_string().contains("agent.before_tool")), "hidden check became a model tool");
                let target = tools.iter().find(|t| t["function"]["description"].as_str().is_some_and(|s| s.contains("BEFORE_TOOL_TARGET_FIXTURE"))).expect("Product target tool absent");
                let value = if mode == "bad-schema" { json!("invalid") } else { json!(7) };
                json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":format!("target-{mode}"),"type":"function","function":{
                    "name":target["function"]["name"],"arguments":json!({"mode":mode,"value":value,"password":"secret-fixture-value"}).to_string()
                }}]},"finish_reason":"tool_calls"}]})
            } else if result_content(&format!("report-{mode}")).is_some() {
                let subject = result_content(&format!("target-{mode}-retry"))
                    .or_else(|| result_content(&format!("target-{mode}"))).unwrap_or_default();
                json!({"choices":[{"index":0,"delta":{"content":format!("BEFORE_TOOL_DONE_{mode}:{subject}")},"finish_reason":"stop"}]})
            } else if result_content(&format!("plan-finish-{mode}")).is_some() {
                let succeeded = result_content(&format!("target-{mode}-retry"))
                    .is_some_and(|content| content.contains("executed"));
                control_call(format!("report-{mode}"), "report_completion", json!({
                    "summary":"Before-tool fixture reached a terminal outcome",
                    "criteria":[{"step":"Process the selected fixture target",
                        "disposition":if succeeded { "supported" } else { "unverified" },
                        "evidence_call_ids":if succeeded { vec![format!("target-{mode}-retry")] } else { Vec::<String>::new() },
                        "rationale":if succeeded { "The selected target returned successfully" } else { "The target was rejected before execution" },
                        "requirement_ids":[requirement_id]
                    }]
                }))
            } else if result_content(&format!("target-{mode}-retry")).is_some() {
                control_call(format!("plan-finish-{mode}"), "update_plan", json!({
                    "explanation":"Record the fixture target outcome",
                    "plan":[{"step":"Process the selected fixture target","status":"completed"}],
                    "requirements":requirements
                }))
            } else if result_content(&format!("plan-start-{mode}")).is_some() {
                let tools = body["tools"].as_array().expect("planned target must remain exposed");
                let target = tools.iter().find(|t| t["function"]["description"].as_str().is_some_and(|s| s.contains("BEFORE_TOOL_TARGET_FIXTURE"))).expect("planned Product target absent");
                let value = if mode == "bad-schema" { json!("invalid") } else { json!(7) };
                json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":format!("target-{mode}-retry"),"type":"function","function":{
                    "name":target["function"]["name"],"arguments":json!({"mode":mode,"value":value,"password":"secret-fixture-value"}).to_string()
                }}]},"finish_reason":"tool_calls"}]})
            } else {
                let needs_effect_plan = tool_results.last().unwrap()["content"].as_str()
                    .is_some_and(|content| content.contains("Call update_plan"));
                control_call(
                    if needs_effect_plan { format!("plan-start-{mode}") } else { format!("plan-finish-{mode}") },
                    "update_plan",
                    json!({
                        "explanation":if needs_effect_plan { "Admit the selected fixture effect" } else { "Record the rejected fixture input" },
                        "plan":[{"step":"Process the selected fixture target","status":if needs_effect_plan { "in_progress" } else { "completed" }}],
                        "requirements":requirements
                    }),
                )
            };
            wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                .set_body_string(format!("data: {frame}\n\ndata: [DONE]\n\n"))
        }).mount(&upstream).await;
        let provider = data(&router,"POST","/api/providers",json!({
            "platform":"stepfun-plan","name":"Before tool model fixture","base_url":format!("{}/step_plan/v1",upstream.uri()),
            "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
            "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{
                "task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}
            }]}
        })).await;
        let preset = data(
            &router,
            "POST",
            "/api/agent-presets/from-template/chat.minimal",
            json!({
                "display_name":"Before tool fixture","reuse_existing":false,
                "model":{"provider_id":provider["provider_id"],"model":"step-3.7-flash"}
            }),
        )
        .await["preset"]["preset_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let editor = data(
            &router,
            "GET",
            &format!("/api/agent-presets/{preset}/editor"),
            Value::Null,
        )
        .await;
        let mut draft = editor["draft"].clone();
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().push(json!({
            "capability":{"id":target_capability(&target_id).id,"version":"1.0.0"},
            "action_allowlist":[TARGET_ACTION]
        }));
        data(
            &router,
            "POST",
            &format!("/api/agent-presets/{preset}/revisions"),
            json!({"expected_current_revision":draft["current_revision"],"draft":draft}),
        )
        .await;
        Self {
            router,
            services,
            _root: root,
            target_log,
            hook_log,
            abort_log,
            upstream,
            preset,
            hooks,
        }
    }
    async fn select_hooks(&self, order: &[String]) {
        let path = format!("/api/agent-presets/{}", self.preset);
        let editor = data(&self.router, "GET", &format!("{path}/editor"), Value::Null).await;
        let mut draft = editor["draft"].clone();
        let enabled = draft["document"]["enabled_capabilities"]
            .as_array_mut()
            .unwrap();
        for (id, _, _) in &self.hooks {
            if !enabled.iter().any(|item| item["capability"]["id"] == *id) {
                enabled
                    .push(json!({
                        "capability":{"id":id,"version":"1.0.0"},
                        "action_allowlist":[middleware::BEFORE_ACTION_ID]
                    }));
            }
        }
        draft["document"]["middleware_order"] = json!(order);
        data(
            &self.router,
            "POST",
            &format!("{path}/revisions"),
            json!({"expected_current_revision":draft["current_revision"],"draft":draft}),
        )
        .await;
    }
    async fn session(&self, title: &str) -> String {
        data(
            &self.router,
            "POST",
            "/api/agent-sessions",
            json!({"preset_id":self.preset,"title":title}),
        )
        .await["agent_session_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    async fn start(&self, session: &str, mode: &str) -> (StatusCode, Value) {
        request(
            &self.router,
            "POST",
            &format!("/api/agent-sessions/{session}/turns"),
            json!({
                "input":{"content":mode},"idempotency_key":format!("before-tool-{mode}")
            }),
        )
        .await
    }
    async fn settle(&self, session: &str, mode: &str, success: bool) -> Value {
        let mut latest = Value::Null;
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                latest = data(
                    &self.router,
                    "GET",
                    &format!("/api/agent-sessions/{session}"),
                    Value::Null,
                )
                .await;
                let active = matches!(
                    latest["head"]["status"].as_str(),
                    Some("running" | "starting")
                );
                let complete = if success {
                    latest
                        .to_string()
                        .contains(&format!("BEFORE_TOOL_DONE_{mode}"))
                } else {
                    true
                };
                if !active && complete {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{mode} did not settle: {latest}"));
        latest
    }
    async fn turn(&self, session: &str, mode: &str, success: bool) -> Value {
        let (status, response) = self.start(session, mode).await;
        assert!(status.is_success(), "{mode}: {status} {response}");
        self.settle(session, mode, success).await
    }
    async fn shutdown(&self) {
        self.services
            .plugin_runtime
            .shutdown_service_runtime(self.services.authoritative_user_id.as_ref())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn product_before_tool_publish_select_allow_deny_and_unselected_baseline() {
    let fixture = Fixture::new(&["A"]).await;
    let catalog = data(&fixture.router, "GET", "/api/agent-catalog", Value::Null).await;
    let published = catalog["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["capability"]["id"] == fixture.hooks[0].0)
        .unwrap();
    assert_eq!(published["middleware_phase"], "before_tool");
    let no_hook = fixture.session("No hook frozen baseline").await;
    fixture.select_hooks(&[]).await;
    let selected = fixture.session("Selected hook").await;
    fixture.turn(&selected, "allow", true).await;
    let denied = fixture.turn(&selected, "deny", true).await;
    assert!(denied.to_string().contains("DENIED_BY_A"), "{denied}");
    fixture.turn(&no_hook, "baseline", true).await;
    assert_eq!(lines(&fixture.target_log), ["allow", "baseline"]);
    let hooks = hook_log(&fixture.hook_log);
    assert_eq!(
        hooks
            .iter()
            .map(|v| v["mode"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["allow", "deny"]
    );
    assert_ne!(hooks[0]["invocation_id"], hooks[1]["invocation_id"]);
    assert_eq!(hooks[0]["tool_call_id"], "target-allow-retry");
    let observations: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT bounded_observation_json FROM agent_effects WHERE session_id = ? AND action_id = ? AND state = 'returned'"
    ).bind(&selected).bind(middleware::BEFORE_ACTION_ID).fetch_all(fixture.services.database.pool()).await.unwrap();
    assert_eq!(observations.len(), 2);
    assert!(
        observations
            .iter()
            .all(|v| !v.contains("secret-fixture-value"))
    );

    let invalid_session = fixture
        .session("Owner rejects invalid target arguments")
        .await;
    let failed = fixture.turn(&invalid_session, "bad-schema", true).await;
    assert!(
        failed.to_string().contains("JSON Schema validation failed"),
        "{failed}"
    );
    assert!(
        failed["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["kind"] == "tool/result-recorded"
                && event["payload"]["value"]["error"]
                    .as_str()
                    .is_some_and(|text| text.contains("tool was not executed")))
    );
    assert_eq!(
        hook_log(&fixture.hook_log).len(),
        2,
        "invalid target input reached hook"
    );
    assert_eq!(lines(&fixture.target_log), ["allow", "baseline"]);

    fixture.shutdown().await;
}

#[tokio::test]
async fn product_before_tool_order_is_frozen_and_first_deny_stops_later_hooks() {
    let fixture = Fixture::new(&["A", "B"]).await;
    fixture.select_hooks(&[]).await;
    let old = fixture.session("Frozen order").await;
    let mut ordered = fixture.hooks.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    let first = ordered[0];
    let second = ordered[1];
    // Neither Service is prestarted: both checks must allow this first real
    // target under the unchanged shared checking deadline.
    fixture.turn(&old, "allow", true).await;
    assert_eq!(lines(&fixture.target_log), ["allow"]);
    fixture.turn(&old, "order-original", true).await;
    fixture.select_hooks(&[second.0.clone()]).await;
    let new = fixture.session("Changed order").await;
    fixture.turn(&new, "order-new", true).await;
    fixture.turn(&old, "order-frozen", true).await;
    let hooks = hook_log(&fixture.hook_log);
    assert_eq!(hooks.len(), 5, "two cold allows then one check per denied turn");
    assert_eq!(
        hooks
            .iter()
            .map(|entry| entry["tag"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [first.2.as_str(), second.2.as_str(), first.2.as_str(), second.2.as_str(), first.2.as_str()]
    );
    assert_eq!(lines(&fixture.target_log), ["allow"], "denied turns must not dispatch the target");
    fixture.shutdown().await;
}

#[tokio::test]
async fn product_before_tool_invalid_timeout_cancel_and_disable_never_dispatch_target() {
    let fixture = Fixture::new(&["A"]).await;
    fixture.select_hooks(&[]).await;
    let disabled = fixture.session("Frozen before disable").await;
    for mode in ["invalid", "wait"] {
        let session = fixture.session(mode).await;
        let page = fixture.turn(&session, mode, false).await;
        assert!(
            page.to_string().contains("before_tool")
                || page.to_string().contains("Hosted tool effects"),
            "{mode}: {page}"
        );
        assert!(
            !page
                .to_string()
                .contains(&format!("BEFORE_TOOL_DONE_{mode}")),
            "technical failure continued model reasoning"
        );
        assert!(lines(&fixture.target_log).is_empty());
    }
    let cancel = fixture.session("Cancel running hook").await;
    let (status, response) = fixture.start(&cancel, "cancel").await;
    assert!(status.is_success(), "{status} {response}");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !hook_log(&fixture.hook_log)
            .iter()
            .any(|entry| entry["mode"] == "cancel")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancel must happen after the actual Node hook entered");
    let (cancel_status, cancel_response) = request(
        &fixture.router,
        "POST",
        &format!("/api/agent-sessions/{cancel}/turns/cancel"),
        json!({"idempotency_key":"cancel-running-before-tool"}),
    )
    .await;
    assert!(
        cancel_status.is_success()
            || (cancel_status == StatusCode::BAD_GATEWAY
                && cancel_response["code"] == "TIMEOUT"
                && cancel_response
                    .to_string()
                    .contains("stop is still in progress")),
        "{cancel_status} {cancel_response}"
    );
    // Even if durable stop finalization is pending, the same caller signal must
    // promptly reach the actual JS AbortSignal. A 30s watchdog is not this proof.
    tokio::time::timeout(Duration::from_secs(3), async {
        while !lines(&fixture.abort_log)
            .iter()
            .any(|mode| mode == "cancel")
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("caller cancellation did not reach the actual Node hook");
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let session = data(
                &fixture.router,
                "GET",
                &format!("/api/agent-sessions/{cancel}"),
                Value::Null,
            )
            .await;
            if !matches!(
                session["head"]["status"].as_str(),
                Some("running" | "starting")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("cancelled tool gate must settle");
    assert!(lines(&fixture.target_log).is_empty());
    let base = &fixture.hooks[0].1;
    let workshop = data(
        &fixture.router,
        "GET",
        &format!("{base}/workshop"),
        Value::Null,
    )
    .await;
    let p = &workshop["plugin"];
    data(&fixture.router,"POST",&format!("{base}/enabled"),json!({
        "plugin_id":p["plugin_id"],"expected_product_revision":p["product_revision"],
        "expected_pointer_revision":p["releases"]["pointer_revision"],
        "expected_active_release_digest":p["releases"]["active"]["release_digest"],"enabled":false
    })).await;
    let before_disable = hook_log(&fixture.hook_log).len();
    let (status, response) = fixture.start(&disabled, "disabled").await;
    if status.is_success() {
        let page = fixture.settle(&disabled, "disabled", false).await;
        assert!(
            page.to_string().contains("before_tool") || page.to_string().contains("Plugin"),
            "{page}"
        );
    } else {
        assert!(
            response.to_string().contains("capability")
                || response.to_string().contains("Plugin")
                || response.to_string().contains("before_tool"),
            "{status} {response}"
        );
    }
    assert_eq!(hook_log(&fixture.hook_log).len(), before_disable);
    assert!(lines(&fixture.target_log).is_empty());
    // The mandatory effect plan may require proposal -> plan -> retry. A
    // technical middleware failure must still end at that retry: it can never
    // be converted into a tool result supplied to another model request.
    for mode in ["invalid", "wait", "cancel", "disabled"] {
        let calls = fixture
            .upstream
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| {
                let body = request.body_json::<Value>().unwrap();
                body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|m| m["role"] == "user")
                    .unwrap()["content"]
                    == mode
            })
            .collect::<Vec<_>>();
        assert!(calls.len() <= 3, "{mode} continued model reasoning");
        assert!(calls.iter().all(|request| {
            let body = request.body_json::<Value>().unwrap();
            !body["messages"].as_array().unwrap().iter().any(|message| {
                message["role"] == "tool"
                    && message["tool_call_id"] == format!("target-{mode}-retry")
            })
        }), "{mode} technical failure was supplied back to the model");
    }
    fixture.shutdown().await;
}
