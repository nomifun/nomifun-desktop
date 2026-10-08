//! Host-controlled authoring evidence and user decisions for an exact Artifact.
//! UI commands are temporary handles; accepted cases and delivery facts live in
//! the existing draft/conversation, not in a second job or publication platform.
use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};
use axum::{Json, extract::{State, Extension, Path}};
use nomifun_agent_contracts::digest_payload;
use nomifun_api_types::*;
use nomifun_auth::CurrentUser;
use nomifun_plugin_platform::{PluginDraftRecord, PluginRepository};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use uuid::Uuid;
use super::plugin::{self, PluginHttpError, PluginRouterState};
use super::engine_tool_host::EngineToolDispatchRecord;
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_engine_core::EngineToolResult;

fn hash(value:&Value)->String { digest_payload(value).expect("JSON value").as_ref().to_owned() }
fn error(message:&str)->PluginHttpError { PluginHttpError::bad_request(message) }

/// Install revokes the draft preview and itself checks the installed plugin
/// and UI. A Preview case after it would strip that delivery evidence.
pub(super) const INSTALLED_PREVIEW_CLOSED: &str = "PLUGIN_PREVIEW_REQUIRED: install already ran for this verified revision and closed its preview; install itself checks the installed plugin and its UI. After install, do not call test_action or test_ui; when the plan has a current_conversation_case, call the installed tool instead. To retry an incomplete installed check, call install again; to change the plugin, call apply, then check, preview and rerun the planned cases before installing again.";

/// Install records `installed_observation` (and `delivery` on success); check,
/// preview and apply all rebuild verification without them, so their presence
/// means install has run since the last preview.
pub(super) fn installed_since_preview(verification: &Value) -> bool {
    !verification["installed_observation"].is_null() || !verification["delivery"].is_null()
}

/// Proof comes from the actual Agent dispatch and its settled result, after
/// this exact installation. Global publications and creator preview calls do
/// not establish consumption by the current conversation.
pub(super) async fn current_conversation_consumed(
    pool:&nomifun_db::SqlitePool, conversation:&str, operation:&str, report:&Value,
) -> Result<bool,nomifun_common::AppError> {
    let case=&report["plan"]["current_conversation_case"];
    if case.is_null() { return Ok(true); }
    let failure=|error: String|nomifun_common::AppError::Conflict(format!("Plugin consumption evidence: {error}"));
    let store=nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await.map_err(|error|failure(error.to_string()))?;
    let facts=store.turn_output_facts(&conversation.into(),&operation.into()).await.map_err(|error|failure(error.to_string()))?;
    let events=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        && event.correlation_id.as_ref()==operation).map(|event| {
        if event.kind_version!=1 || event.producer_id.as_ref()!="runtime_supervisor" {
            return Err(failure("unsupported Runtime progress source".into()));
        }
        let payload=facts.event_payloads.get(event.event_id.as_ref())
            .ok_or_else(||failure("Runtime progress has no resolved canonical payload".into()))?;
        serde_json::from_value::<ConsumptionProgress>(payload.clone()).map(|progress|progress.event)
            .map_err(|error|failure(error.to_string()))
    }).collect::<Result<Vec<_>,_>>()?;
    Ok(consumption_evidence(&events,report))
}

#[derive(Deserialize)]
struct ConsumptionProgress { event: ConsumptionEvent }

#[derive(Deserialize)]
#[serde(untagged)]
enum ConsumptionEvent {
    Runtime(AgentEngineEvent),
    Host(ConsumptionHostEvent),
}

#[derive(Deserialize)]
#[serde(tag="event",rename_all="snake_case")]
enum ConsumptionHostEvent {
    HostToolDispatch { dispatch: EngineToolDispatchRecord },
    HostToolSettled { operation_id: String, call_id: String, result: Option<EngineToolResult>, error: Option<String> },
    #[serde(rename="host_resource_dispatch",alias="host_resource_settled",alias="host_process_dispatch",
        alias="host_process_quiescent",alias="host_cleanup_proven")]
    Other,
}

fn consumption_evidence(events:&[ConsumptionEvent], report:&Value) -> bool {
    let case=&report["plan"]["current_conversation_case"];
    let (Some(plugin),Some(action),Some(input),Some(expected))=(report["delivery"]["plugin_id"].as_str(),
        case["action"].as_str(),case.get("input"),case.get("expected_output")) else{return false;};
    let stable=nomifun_agent_contracts::plugin::plugin_action_id(&plugin.into(),action);
    let mut proposals=HashMap::new();
    let mut dispatches=HashMap::new();
    let mut installed=false;
    for event in events {
        match event {
            ConsumptionEvent::Runtime(AgentEngineEvent::ToolCallCompleted {call,..})=>{
                proposals.insert(call.call_id.as_ref(),&call.arguments.0);
            }
            ConsumptionEvent::Host(ConsumptionHostEvent::HostToolDispatch {dispatch})=>{
                dispatches.insert(dispatch.operation_id.as_str(),dispatch);
            }
            ConsumptionEvent::Host(ConsumptionHostEvent::HostToolSettled {operation_id,call_id,result:Some(result),error:None})=>{
                let Some(dispatch)=dispatches.get(operation_id.as_str()) else { continue; };
                if result.is_error || result.call_id.as_ref()!=call_id || *call_id!=dispatch.call_id { continue; }
                let Ok(output)=serde_json::from_str::<Value>(&result.output_text()) else { continue; };
                if dispatch.capability_id=="plugin.development" && dispatch.action_id=="plugin.development/install"
                    && output["delivery"]==report["delivery"] { installed=true; }
                else if installed && dispatch.capability_id==stable && dispatch.action_id==action
                    && proposals.get(dispatch.call_id.as_str())==Some(&input)
                    && output==*expected { return true; }
            }
            _=>{},
        }
    }
    false
}

/// Versions derive from encrypted owner records. Neither ciphertext nor
/// decrypted credentials enter the verification report or model response.
pub(super) async fn verification_context(
    pool: &nomifun_db::SqlitePool, bindings: &Value,
) -> Result<Value, PluginHttpError> {
    Ok(json!({"verifier":"nomifun.plugin-development.verification/v1",
        "host_version":env!("CARGO_PKG_VERSION"),
        "sdk_digest":nomifun_agent_contracts::digest_bytes(plugin::PLUGIN_SDK.as_bytes()),
        "credential_versions":credential_versions(pool,bindings).await?,
    }))
}

#[cfg(test)]
mod credential_context_tests {
    use super::*;
    #[test]
    fn oracle_conflict_reports_the_planned_case_and_actionable_guidance() {
        let acceptance = json!({"required-uppercase":{"kind":"action","action":"normalize",
            "input":{"text":"  Mixed Case  "},"expected_output":{"text":"MIXED CASE"}},
            "read-after-restart":{"kind":"action","action":"persist","input":{},
                "expected_output":{"value":"saved"},"restart":true}});
        let oracle = json!({"kind":"action","action":"normalize","input":{"text":"  Mixed Case  "},
            "expected_output":{"text":"mixed case"}});
        let conflict = oracle_conflict(&acceptance, "required-uppercase", &oracle).unwrap();
        assert!(conflict.contains("case 'required-uppercase' must run exactly as planned"), "{conflict}");
        assert!(conflict.contains("\"expected_output\":{\"text\":\"MIXED CASE\"}"), "{conflict}");
        assert!(conflict.contains("Pass every value as JSON"), "{conflict}");
        assert!(!conflict.contains("mixed case\""), "the rejected value must not be shown: {conflict}");
        assert!(oracle_conflict(&acceptance, "required-uppercase", &acceptance["required-uppercase"]).is_none());
    }

    #[test]
    fn oracle_conflict_allows_a_supplementary_case_absent_from_the_plan() {
        let acceptance = json!({"required-uppercase":{"kind":"action"},"read":{"kind":"ui"}});
        assert!(oracle_conflict(&acceptance, "extra-check", &json!({"kind":"action"})).is_none());
        assert!(oracle_conflict(&Value::Null, "extra-check", &json!({"kind":"action"})).is_none());
    }

    #[test]
    fn oracle_conflict_truncates_the_planned_case_on_a_char_boundary() {
        let wide = json!({"kind":"action","action":"a","input":{"pad":"界".repeat(1500)},"expected_output":{}});
        let acceptance = json!({"x": wide});
        let conflict = oracle_conflict(&acceptance, "x", &json!({"kind":"action","action":"b"})).unwrap();
        let start = conflict.find("{").unwrap();
        let end = conflict.find(". Pass every value as JSON").unwrap();
        assert!(end - start <= 2048 && end - start > 2000, "{conflict}");
    }

    #[test]
    fn ui_failure_diagnostics_include_assertion_and_interrupted_step() {
        let steps=vec![
            json!({"operation":"click","selector":"#add"}),
            json!({"operation":"text","selector":"#list","value":"milk"}),
            json!({"operation":"count","selector":"#missing","value":1}),
        ];
        let diagnostics=ui_case_diagnostics(&steps,&[Value::Null,json!("empty")],Some("UI element not found: #missing"));
        assert_eq!(diagnostics[0],json!({"step_index":1,"step_number":2,"operation":"text",
            "selector":"#list","expected":"milk","actual":"empty","reason":"assertion_mismatch"}));
        assert_eq!(diagnostics[1]["step_index"],2);
        assert_eq!(diagnostics[1]["reason"],"step_execution_failed");
        assert_eq!(diagnostics[1]["error"],"UI element not found: #missing");
        assert!(ui_case_diagnostics(&steps,&[Value::Null,json!("milk"),json!(1)],None).is_empty());
    }

    #[test]
    fn ui_failure_diagnostics_bound_plugin_text_and_result_count() {
        let steps=vec![json!({"operation":"text","selector":"界".repeat(500),"value":"界".repeat(2000)});8];
        let observations=vec![json!({"untrusted":"界".repeat(2000)});8];
        let diagnostics=ui_case_diagnostics(&steps,&observations,None);
        assert_eq!(diagnostics.len(),4);
        assert_eq!(diagnostics[0]["selector"].as_str().unwrap().chars().count(),257);
        assert_eq!(diagnostics[0]["expected"].as_str().unwrap().chars().count(),1025);
        assert_eq!(diagnostics[0]["actual"]["truncated"],true);
        let interrupted=ui_case_diagnostics(&steps,&[],Some(&"界".repeat(2000)));
        assert_eq!(interrupted[0]["error"].as_str().unwrap().chars().count(),1025);
        assert_eq!(interrupted[0]["step_index"],0);
    }

    fn consumption_fixture(input:Value) -> (Value,Vec<Value>) {
        let delivery=json!({"plugin_id":"plugin-id","artifact_digest":"digest","context_digest":"context"});
        let report=json!({"delivery":delivery,"plan":{"current_conversation_case":{
            "action":"normalize","input":input,"expected_output":{"text":"ABC"}
        }}});
        let result=|call:&str,value:Value| serde_json::to_value(nomifun_engine_core::EngineToolResult::text(
            call.to_owned().into(),value.to_string(),false)).unwrap();
        let event=|event:Value|json!({"event":event});
        let events=vec![
            event(json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"install","call_id":"install","capability_id":"plugin.development","action_id":"plugin.development/install"}})),
            event(json!({"event":"host_tool_settled","operation_id":"install","call_id":"install","result":result("install",json!({"delivery":delivery}))})),
            event(json!({"event":"tool_call_completed","step":1,"call":{"call_id":"use","name":"normalize","arguments":input}})),
            event(json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"use","call_id":"use","capability_id":"plugin:plugin-id/normalize","action_id":"normalize"}})),
            event(json!({"event":"host_tool_settled","operation_id":"use","call_id":"use","result":result("use",json!({"text":"ABC"}))})),
        ];
        (report,events)
    }

    fn evidence(events:&[Value],report:&Value) -> bool {
        let events=events.iter().map(|event|serde_json::from_value::<ConsumptionProgress>(event.clone()).unwrap().event).collect::<Vec<_>>();
        consumption_evidence(&events,report)
    }

    #[test]
    fn current_consumption_requires_exact_dispatch_input_and_settlement_after_install() {
        let (report,mut events)=consumption_fixture(json!({"text":"abc"}));
        assert!(evidence(&events,&report));
        events[3]["event"]["dispatch"]["capability_id"]=json!("plugin.development");
        assert!(!evidence(&events,&report),"preview/creator calls cannot prove conversation consumption");
        events[3]["event"]["dispatch"]["capability_id"]=json!("plugin:plugin-id/normalize");
        events[2]["event"]["call"]["arguments"]=json!({"text":"different"});
        assert!(!evidence(&events,&report));
        assert!(!evidence(&events[2..],&report),"an installation boundary is required");
        events[2]["event"]["call"]["arguments"]=json!({"text":"abc"});
        events[4]["event"]["call_id"]=json!("other-call");
        assert!(!evidence(&events,&report),"the settlement must identify the dispatched call");
        events[4]["event"]["call_id"]=json!("use");
        events[4]["event"]["result"]["call_id"]=json!("other-result");
        assert!(!evidence(&events,&report),"the result must belong to the dispatched call");
    }

    #[tokio::test]
    async fn current_consumption_resolves_stored_canonical_payloads_and_rejects_wrong_input() {
        let (mut report,events)=consumption_fixture(json!({"text":"x".repeat(70*1024)}));
        let (journal,pool)=super::super::engine_journal::test_fixture().await;
        for (index,payload) in events.iter().enumerate() {
            let write=if matches!(index,1|4) {super::super::engine_journal::EngineJournalWrite::Settlement}
                else {super::super::engine_journal::EngineJournalWrite::Progress};
            journal.append(payload["event"].to_string(),None,write).await.unwrap();
        }
        let stored:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind='runtime/progress-recorded' AND payload_id IS NOT NULL")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(stored,1,"the oversized model proposal must use a stored Payload");
        let session="0190f5fe-7c00-7a00-8000-000000000002";
        assert!(current_conversation_consumed(&pool,session,"turn",&report).await.unwrap());
        report["plan"]["current_conversation_case"]["input"]=json!({"text":"wrong"});
        assert!(!current_conversation_consumed(&pool,session,"turn",&report).await.unwrap());
        assert!(current_conversation_consumed(&pool,session,"other-turn",&report).await.is_err());
    }

    #[test]
    fn installed_since_preview_detects_the_install_boundary() {
        assert!(installed_since_preview(&json!({"installed_observation":{"ui_ready":false}})));
        assert!(installed_since_preview(&json!({"delivery":{"plugin_id":"p"}})));
        assert!(!installed_since_preview(&json!({"runtime_ready":true,"surface":{"is_preview":true}})));
        assert!(!installed_since_preview(&json!({"installed_observation":null})));
    }
    #[tokio::test]
    async fn encrypted_credential_rotation_invalidates_context_without_exposing_values() {
        let pool=nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        nomifun_db::sqlx::query("CREATE TABLE providers(provider_id TEXT PRIMARY KEY, enabled INTEGER, credentials_encrypted TEXT)")
            .execute(&pool).await.unwrap();
        let id=Uuid::now_v7().to_string();
        let first="ciphertext-context-sentinel-one";
        nomifun_db::sqlx::query("INSERT INTO providers VALUES(?,1,?)").bind(&id).bind(first).execute(&pool).await.unwrap();
        let bindings=json!({"api_key":format!("provider:{id}")});
        let original=verification_context(&pool,&bindings).await.unwrap();
        assert!(!original.to_string().contains(first));
        assert_eq!(original["credential_versions"]["api_key"]["reference"],format!("provider:{id}"));
        nomifun_db::sqlx::query("UPDATE providers SET credentials_encrypted=? WHERE provider_id=?")
            .bind("ciphertext-context-sentinel-two").bind(&id).execute(&pool).await.unwrap();
        assert_ne!(original,verification_context(&pool,&bindings).await.unwrap());
        nomifun_db::sqlx::query("UPDATE providers SET enabled=0 WHERE provider_id=?").bind(&id).execute(&pool).await.unwrap();
        assert!(verification_context(&pool,&bindings).await.is_err());
        assert!(verification_context(&pool,&json!({"api_key":"plaintext-key"})).await.is_err());
        assert_eq!(verification_context(&pool,&json!({})).await.unwrap()["credential_versions"],json!({}));
    }
}

async fn credential_versions(pool: &nomifun_db::SqlitePool, bindings: &Value) -> Result<Value, PluginHttpError> {
    let bindings=bindings.as_object().ok_or_else(||error("Credential bindings must be an object"))?;
    let mut versions=serde_json::Map::new();
    for (slot,reference) in bindings {
        let reference=reference.as_str().ok_or_else(||error("Use a Host Credential reference"))?;
        let (kind,id)=reference.split_once(':').ok_or_else(||error("Use a listed Host Credential reference"))?;
        nomifun_common::validate_uuidv7(id).map_err(|_|error("Invalid Host Credential reference"))?;
        let query=match kind {
            "provider"=>"SELECT credentials_encrypted FROM providers WHERE provider_id=? AND enabled=1",
            "connection"=>"SELECT c.credentials_encrypted FROM provider_connections c JOIN providers p ON p.provider_id=c.provider_id WHERE c.connection_id=? AND p.enabled=1",
            _=>return Err(error("Unknown Host Credential reference type")),
        };
        let encrypted:Option<String>=nomifun_db::sqlx::query_scalar(query).bind(id).fetch_optional(pool).await
            .map_err(|error|PluginHttpError::internal(error.to_string()))?;
        let encrypted=encrypted.filter(|value|!value.trim().is_empty())
            .ok_or_else(||error("PLUGIN_CREDENTIAL_UNAVAILABLE: configure an enabled Host Credential reference"))?;
        versions.insert(slot.clone(),json!({"reference":reference,
            "version":hash(&json!({"domain":"plugin-authoring-credential/v1","reference":reference,"encrypted":encrypted}))}));
    }
    Ok(Value::Object(versions))
}

pub(super) async fn save_authoring_draft(
    state:&PluginRouterState,owner:&str,id:&str,mut request:SavePluginDraftRequest,draft:&PluginDraftRecord,
)->Result<SavePluginDraftResponseDto,PluginHttpError>{
    if request.expected_plugin_revision.is_none(){request.expected_plugin_revision=draft.base_revision;}
    plugin::save_draft_owned(state,owner,id,request).await
}

#[derive(Clone, Serialize)]
pub(super) struct UiCommand {
    test_token:String, draft_id:String, descriptor:PluginSurfaceDescriptorDto,
    steps:Vec<Value>, case_name:String,
}
struct PendingUi { owner:String, command:UiCommand, sender:oneshot::Sender<UiResponse> }
#[derive(Default)]
pub(super) struct UiTests { pending:Mutex<HashMap<String,PendingUi>> }
struct UiGuard { tests:Arc<UiTests>, token:String }
impl Drop for UiGuard {fn drop(&mut self){self.tests.pending.lock().expect("UI queue").remove(&self.token);}}

/// Close only the exact authoring-owned Surface if verification is interrupted.
pub(super) struct AuthoringSurfaceGuard {
    state: PluginRouterState,
    owner: String,
    descriptor: Option<PluginSurfaceDescriptorDto>,
}
impl AuthoringSurfaceGuard {
    pub(super) fn new(state: &PluginRouterState, owner: &str, descriptor: &PluginSurfaceDescriptorDto) -> Self {
        Self { state: state.clone(), owner: owner.into(), descriptor: Some(descriptor.clone()) }
    }
    pub(super) fn retain(&mut self) { self.descriptor = None; }
    async fn close(&mut self) -> Result<(), PluginHttpError> {
        if let Some(descriptor) = &self.descriptor {
            plugin::close_surface_owned(&self.state, &self.owner, ClosePluginSurfaceRequest {
                surface_session_id: descriptor.surface_session_id.clone(),
                surface_generation: descriptor.surface_generation,
            }).await?;
        }
        self.retain();
        Ok(())
    }
}
impl Drop for AuthoringSurfaceGuard {
    fn drop(&mut self) {
        if let Some(descriptor) = self.descriptor.take() {
            let state = self.state.clone();
            let owner = self.owner.clone();
            tokio::spawn(async move {
                let _ = plugin::close_surface_owned(&state, &owner, ClosePluginSurfaceRequest {
                    surface_session_id: descriptor.surface_session_id, surface_generation: descriptor.surface_generation,
                }).await;
            });
        }
    }
}

/// Participates in the ordinary Runtime's owned cleanup witness, after tools join.
/// Only this turn's draft previews are revoked; installed app Surfaces are independent.
pub(super) async fn cleanup_authoring_turn(
    state: &PluginRouterState, owner: &str, conversation: &str, root: &str,
) -> Result<(), PluginHttpError> {
    let ids: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT draft_id FROM plugin_drafts WHERE owner_user_id=? AND source_conversation_id=? AND (source_message_id=? OR json_extract(verification_json, '$.task_message_id')=?)"
    ).bind(owner).bind(conversation).bind(root).bind(root).fetch_all(state.repository.pool()).await
        .map_err(|error| PluginHttpError::internal(error.to_string()))?;
    for id in ids {
        let mut draft = plugin::draft_owned(state, owner, &id).await?;
        plugin::revoke_draft_surfaces(state, &draft.draft_id).await?;
        if let Ok(surface) = serde_json::from_value::<PluginSurfaceDescriptorDto>(draft.verification["surface"].clone()) {
            if !surface.is_preview {
                plugin::close_surface_owned(state, owner, ClosePluginSurfaceRequest {
                    surface_session_id: surface.surface_session_id, surface_generation: surface.surface_generation,
                }).await?;
            }
            draft.verification.as_object_mut().expect("verification").remove("surface");
            let updated = state.repository.update_draft(&draft, draft.revision).await?;
            state.events.send_to_user(owner, WebSocketMessage::new("plugin.authoring.changed", json!({
                "conversation_id":conversation,"draft_id":id,"revision":updated.revision,
            })));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UiResponse {
    test_token:String, descriptor:PluginSurfaceDescriptorDto,
    observations:Vec<Value>, #[serde(default)] error:Option<String>,
}

pub(super) async fn ui_results(
    State(state):State<PluginRouterState>,Extension(user):Extension<CurrentUser>,
    Path(id):Path<String>,Json(response):Json<UiResponse>,
)->Result<Json<ApiResponse<bool>>,PluginHttpError>{
    let owner=user.id.to_string();
    let mut queue=state.ui_tests.pending.lock().expect("UI queue");
    let pending=queue.get(&response.test_token).ok_or_else(PluginHttpError::not_found)?;
    if pending.owner!=owner || pending.command.draft_id!=id
        || pending.command.descriptor!=response.descriptor
        || response.observations.len()>pending.command.steps.len()
    {return Err(PluginHttpError::not_found());}
    let pending=queue.remove(&response.test_token).expect("checked command");
    let _=pending.sender.send(response);
    Ok(Json(ApiResponse::ok(true)))
}

pub(super) async fn details(
    State(state):State<PluginRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,
)->Result<Json<ApiResponse<Value>>,PluginHttpError>{
    let owner=user.id.to_string();
    let draft=plugin::draft_owned(&state,&owner,&id).await?;
    let commands=state.ui_tests.pending.lock().expect("UI queue").values()
        .filter(|entry|entry.owner==owner && entry.command.draft_id==id)
        .map(|entry|entry.command.clone()).collect::<Vec<_>>();
    let public=draft.verification.clone();
    Ok(Json(ApiResponse::ok(json!({
        "draft":plugin::draft_detail(&state,&draft)?,"verification":public,"commands":commands,
    }))))
}

/// Persist the case attempt before effects or an ephemeral UI wait. Only
/// plan.cases defines immutable requirements; extra cases remain diagnostic.
/// Dropping the caller leaves a failed/pending result rather than erasing it.
pub(super) async fn begin_authoring_case(
    state: &PluginRouterState,
    draft: &PluginDraftRecord,
    case_name: &str,
    oracle: Value,
) -> Result<PluginDraftRecord, PluginHttpError> {
    if let Some(conflict) = oracle_conflict(&draft.verification["plan"]["cases"], case_name, &oracle) {
        return Err(PluginHttpError::conflict(&conflict));
    }
    let mut next = draft.clone();
    next.verification["cases"][case_name] = json!({
        "kind": oracle["kind"], "passed": false, "state": "running",
    });
    next.verification.as_object_mut().expect("verification object").remove("delivery");
    next.updated_at_ms = nomifun_common::now_ms();
    state.repository.update_draft(&next, draft.revision).await.map_err(Into::into)
}

/// The accepted oracle is immutable: a rerun must reproduce it exactly, so a
/// conflict reports the planned case (compact, bounded) rather than the guess.
/// A name absent from the plan is a supplementary case and can change while
/// debugging; it never becomes a requirement by running it.
fn oracle_conflict(planned_cases: &Value, case_name: &str, oracle: &Value) -> Option<String> {
    let planned_case = &planned_cases[case_name];
    if planned_case.is_null() || *planned_case == *oracle {
        return None;
    }
    let mut planned = serde_json::to_string(planned_case).unwrap_or_else(|_| planned_case.to_string());
    let mut end = planned.len().min(2048);
    while !planned.is_char_boundary(end) {
        end -= 1;
    }
    planned.truncate(end);
    Some(format!(
        "PLUGIN_ORACLE_CHANGED: case '{case_name}' must run exactly as planned: {planned}. Pass every value as JSON (objects, arrays, numbers), not as strings containing JSON."
    ))
}

/// Check the actual installed Surface and Bridge without replaying mutations
/// against production storage. This observation is not a Preview business case.
async fn wait_for_ui_response(
    state: &PluginRouterState, conversation: &str, turn: &str, mut receiver: oneshot::Receiver<UiResponse>,
) -> Result<UiResponse, String> {
    let store = nomifun_agent_session::AgentSessionStore::from_pool(state.repository.pool().clone())
        .await.map_err(|error| error.to_string())?;
    let session = nomifun_agent_contracts::AgentSessionId::from(conversation.to_owned());
    let operation = nomifun_agent_contracts::OperationId::from(turn.to_owned());
    let mut poll = tokio::time::interval(Duration::from_millis(200));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::sleep(Duration::from_secs(25));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            response = &mut receiver => return response.map_err(|_| "UI verification was cancelled".into()),
            _ = &mut deadline => return Err("Actual Surface did not respond; open the conversation and retry".into()),
            _ = poll.tick() => {
                let receipt = store.read_turn_receipt(&session, &operation).await.map_err(|error| error.to_string())?;
                if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
                    return Err("Plugin UI verification stopped with its owning conversation turn".into());
                }
            }
        }
    }
}

pub(super) async fn run_installed_ui_check(
    state: &PluginRouterState, owner: &str, conversation: &str, draft: &PluginDraftRecord,
    plugin_id: &str, plugin_revision: u64, turn: &str,
) -> Result<PluginDraftRecord, PluginHttpError> {
    if let Ok(previous) = serde_json::from_value::<PluginSurfaceDescriptorDto>(draft.verification["surface"].clone()) {
        if !previous.is_preview {
            let _ = plugin::close_surface_owned(state, owner, ClosePluginSurfaceRequest {
                surface_session_id: previous.surface_session_id, surface_generation: previous.surface_generation,
            }).await;
        }
    }
    let descriptor = plugin::open_surface_owned(state, owner, plugin_id, OpenPluginSurfaceRequest {
        expected_revision: plugin_revision,
    }).await?;
    let mut surface_guard = AuthoringSurfaceGuard::new(state, owner, &descriptor);
    let mut current = draft.clone();
    current.verification["surface"] = serde_json::to_value(&descriptor).expect("descriptor");
    current.verification["installed_observation"]["ui_ready"] = json!(false);
    current.verification.as_object_mut().expect("verification").remove("delivery");
    current.updated_at_ms = nomifun_common::now_ms();
    current = state.repository.update_draft(&current, current.revision).await?;
    let revision = current.revision;
    let token = Uuid::now_v7().to_string();
    let (sender, receiver) = oneshot::channel();
    let command = UiCommand {
        test_token: token.clone(), draft_id: current.draft_id.as_ref().to_owned(), descriptor: descriptor.clone(),
        steps: vec![json!({"operation":"ready"})], case_name: "installed_surface_bridge".into(),
    };
    {
        let mut pending = state.ui_tests.pending.lock().expect("UI queue");
        if pending.len() >= 64 { return Err(PluginHttpError::unavailable("UI verification queue is full")); }
        pending.insert(token.clone(), PendingUi { owner: owner.into(), command, sender });
    }
    let _guard = UiGuard { tests: state.ui_tests.clone(), token };
    state.events.send_to_user(owner, WebSocketMessage::new("plugin.authoring.changed", json!({
        "conversation_id":conversation,"draft_id":current.draft_id.as_ref(),"revision":revision,
    })));
    let (passed, diagnostic) = match wait_for_ui_response(state, conversation, turn, receiver).await {
        Ok(response) => (response.error.is_none() && response.observations == vec![json!(true)], response.error),
        Err(error) => (false, Some(error)),
    };
    current = plugin::draft_owned(state, owner, current.draft_id.as_ref()).await?;
    plugin::require_draft_revision(&current, revision)?;
    if current.verification["surface"] != serde_json::to_value(&descriptor).expect("descriptor") {
        return Err(PluginHttpError::conflict("Installed Surface changed during verification"));
    }
    current.verification["installed_observation"]["ui_ready"] = json!(passed);
    current.verification["installed_observation"]["ui_error"] = json!(diagnostic);
    current.verification["installed_observation"]["surface"] = serde_json::to_value(descriptor).expect("descriptor");
    current.verification.as_object_mut().expect("verification").remove("surface");
    current.updated_at_ms = nomifun_common::now_ms();
    let updated = state.repository.update_draft(&current, revision).await?;
    surface_guard.close().await?;
    Ok(updated)
}

fn bounded_ui_text(value: &str, max_chars: usize) -> String {
    let mut preview: String=value.chars().take(max_chars).collect();
    if value.chars().nth(max_chars).is_some() { preview.push('…'); }
    preview
}

fn bounded_ui_value(value: &Value) -> Value {
    match value {
        Value::String(text)=>json!(bounded_ui_text(text,1024)),
        Value::Array(_) | Value::Object(_)=>{
            let text=value.to_string();
            if text.chars().count()<=1024 { value.clone() }
            else { json!({"preview":bounded_ui_text(&text,1024),"truncated":true}) }
        },
        _=>value.clone(),
    }
}

/// These observations are plugin-authored tool data, never host instructions.
/// Keep enough context for a repair without echoing an unbounded DOM result.
fn ui_case_diagnostics(steps: &[Value], observations: &[Value], error: Option<&str>) -> Vec<Value> {
    let mut diagnostics=Vec::new();
    for (index,(step,actual)) in steps.iter().zip(observations).enumerate() {
        if matches!(step["operation"].as_str(),Some("text"|"count")) && step["value"]!=*actual {
            diagnostics.push(json!({"step_index":index,"step_number":index+1,
                "operation":step["operation"],"selector":bounded_ui_text(step["selector"].as_str().unwrap_or(""),256),
                "expected":bounded_ui_value(&step["value"]),"actual":bounded_ui_value(actual),"reason":"assertion_mismatch"}));
            if diagnostics.len()==4 { break; }
        }
    }
    if diagnostics.len()<4 {
        if let Some(step)=steps.get(observations.len()) {
            diagnostics.push(json!({"step_index":observations.len(),"step_number":observations.len()+1,
                "operation":step["operation"],"selector":bounded_ui_text(step["selector"].as_str().unwrap_or(""),256),
                "expected":bounded_ui_value(&step["value"]),"actual":null,
                "reason":if error.is_some(){"step_execution_failed"}else{"missing_observation"},
                "error":bounded_ui_text(error.unwrap_or("The preview did not return this step's observation"),1024)}));
        } else if let Some(error)=error {
            diagnostics.push(json!({"reason":"ui_execution_failed","error":bounded_ui_text(error,1024)}));
        } else if observations.len()>steps.len() {
            diagnostics.push(json!({"reason":"unexpected_observation_count","expected":steps.len(),"actual":observations.len()}));
        }
    }
    if diagnostics.is_empty() && !steps.iter().any(|step|matches!(step["operation"].as_str(),Some("text"|"count"))) {
        diagnostics.push(json!({"reason":"no_assertions","error":"A UI case requires a text or count assertion"}));
    }
    diagnostics
}

pub(super) async fn run_authoring_ui_test(
    state:&PluginRouterState,owner:&str,conversation:&str,turn:&str,input:Value,
)->Result<Value,PluginHttpError>{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {draft_id:String,expected_revision:u64,case_name:String,steps:Vec<Value>}
    let request:Request=serde_json::from_value(input).map_err(|error|PluginHttpError::bad_request(&error.to_string()))?;
    let mut draft=plugin::draft_owned(state,owner,&request.draft_id).await?;
    plugin::require_authoring_revision(&draft,request.expected_revision)?;
    if installed_since_preview(&draft.verification) { return Err(error(INSTALLED_PREVIEW_CLOSED)); }
    if draft.source_conversation_id.as_deref()!=Some(conversation)
        || draft.verification["runtime_ready"]!=json!(true)
        || draft.verification["has_ui"]!=json!(true)
    {return Err(error("Open this conversation's actual UI preview first"));}
    if request.steps.is_empty() || request.steps.len()>32 {return Err(error("UI cases require 1 to 32 steps"));}
    for step in &request.steps{
        let op=step["operation"].as_str().ok_or_else(||error("UI operation is required"))?;
        if !["click","fill","text","count","reopen"].contains(&op){return Err(error("Unknown UI operation"));}
        if op!="reopen" && step["selector"].as_str().is_none_or(|value|value.is_empty()){
            return Err(error("A concrete selector is required"));
        }
        if ["fill","text","count"].contains(&op) && step.get("value").is_none(){return Err(error("An input or expected value is required"));}
    }
    let planned=!draft.verification["plan"]["cases"][&request.case_name].is_null();
    let oracle=json!({"kind":"ui","steps":request.steps});
    draft=begin_authoring_case(state,&draft,&request.case_name,oracle).await?;
    // Every UI case starts from fresh preview storage: revoking releases the
    // previous session, so this preview gets its own data root and cases
    // cannot read state earlier cases or previews left behind.
    plugin::revoke_draft_surfaces(state,&draft.draft_id).await?;
    let execution=&draft.verification["execution"];
    let preview=plugin::preview_draft_owned(state,owner,&request.draft_id,
        serde_json::from_value::<PreviewPluginDraftRequest>(json!({
            "expected_revision":draft.revision,"config":execution["config"],
            "access":{"permissions":execution["permissions"],"credential_bindings":execution["credential_bindings"]}
        })).map_err(|serde_error|error(&serde_error.to_string()))?).await?;
    let descriptor=preview.descriptor;
    let mut surface_guard = AuthoringSurfaceGuard::new(state, owner, &descriptor);
    draft.verification["surface"]=serde_json::to_value(&descriptor).expect("descriptor");
    draft.updated_at_ms=nomifun_common::now_ms();
    draft=state.repository.update_draft(&draft,draft.revision).await?;
    let execution_revision=draft.revision;
    let token=Uuid::now_v7().to_string();
    let command=UiCommand{test_token:token.clone(),draft_id:request.draft_id.clone(),descriptor:descriptor.clone(),
        steps:request.steps.clone(),case_name:request.case_name.clone()};
    let (sender,receiver)=oneshot::channel();
    {
        let mut pending=state.ui_tests.pending.lock().expect("UI queue");
        if pending.len()>=64{return Err(PluginHttpError::unavailable("UI verification queue is full"));}
        pending.insert(token.clone(),PendingUi{owner:owner.into(),command,sender});
    }
    let _guard=UiGuard{tests:state.ui_tests.clone(),token};
    state.events.send_to_user(owner,WebSocketMessage::new("plugin.authoring.changed",json!({
        "conversation_id":conversation,"draft_id":request.draft_id,"revision":draft.revision,
    })));
    let response=match wait_for_ui_response(state,conversation,turn,receiver).await {
        Ok(response)=>response,
        Err(diagnostic)=>UiResponse {
            test_token:String::new(), descriptor:descriptor.clone(), observations:Vec::new(),
            error:Some(diagnostic),
        },
    };
    let mut passed=response.error.is_none() && response.observations.len()==request.steps.len();
    let mut assertions=0;
    let mut reopened=false;
    let mut persistence=false;
    for (step,observation) in request.steps.iter().zip(&response.observations) {
        match step["operation"].as_str(){
            Some("reopen")=>reopened=true,
            Some("text") | Some("count")=>{
                assertions+=1;
                let matched=step["value"]==*observation;
                passed &= matched;
                if reopened && matched {persistence=true;}
            },
            _=>{},
        }
    }
    if assertions==0 {passed=false;}
    let diagnostics=ui_case_diagnostics(&request.steps,&response.observations,response.error.as_deref());
    let diagnostic_error=response.error.as_deref().map(|error|bounded_ui_text(error,1024));
    // Refresh before committing; file edits/revocation invalidate a pending UI result.
    draft=plugin::draft_owned(state,owner,&request.draft_id).await?;
    plugin::require_draft_revision(&draft,execution_revision)?;
    if draft.verification["surface"]!=serde_json::to_value(&descriptor).expect("descriptor"){
        return Err(PluginHttpError::conflict("Preview changed during UI verification"));
    }
    draft.verification["cases"][&request.case_name]=json!({
        "kind":"ui","passed":passed,"state":if passed{"passed"}else{"failed"},"steps":request.steps,"observations":response.observations,
        "error":response.error,"persistence_checked":persistence,
    });
    draft.updated_at_ms=nomifun_common::now_ms();
    let updated=state.repository.update_draft(&draft,execution_revision).await?;
    surface_guard.retain();
    state.events.send_to_user(owner,WebSocketMessage::new("plugin.authoring.changed",json!({
        "conversation_id":conversation,"draft_id":request.draft_id,"revision":updated.revision,
    })));
    Ok(json!({"draft_id":request.draft_id,"case_name":request.case_name,"planned":planned,
        "revision":updated.revision,"passed":passed,"diagnostics":diagnostics,"error":diagnostic_error,
        "persistence_checked":persistence,"verification_digest":hash(&updated.verification)}))
}
