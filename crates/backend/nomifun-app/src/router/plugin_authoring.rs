//! Host-controlled authoring evidence and user decisions for an exact Artifact.
//! UI commands are temporary handles; accepted cases and delivery facts live in
//! the existing draft/conversation, not in a second job or publication platform.
use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};
use axum::{Json, extract::{State, Extension, Path}};
use nomifun_agent_contracts::{DigestHex, PluginArtifact, digest_payload};
use nomifun_api_types::*;
use nomifun_auth::CurrentUser;
use nomifun_plugin_platform::{PluginDraftRecord, PluginRepository};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use uuid::Uuid;
use super::plugin::{self, PluginHttpError, PluginRouterState};

fn hash(value:&Value)->String { digest_payload(value).expect("JSON value").as_ref().to_owned() }
fn error(message:&str)->PluginHttpError { PluginHttpError::bad_request(message) }

/// Proof comes from the actual Agent dispatch and its settled result, after
/// this exact installation. Global publications and creator preview calls do
/// not establish consumption by the current conversation.
pub(super) async fn current_conversation_consumed(
    pool:&nomifun_db::SqlitePool, conversation:&str, operation:&str, report:&Value,
) -> Result<bool,nomifun_common::AppError> {
    let case=&report["plan"]["current_conversation_case"];
    if case.is_null() { return Ok(true); }
    let rows:Vec<String>=nomifun_db::sqlx::query_scalar(
        "SELECT inline_json FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/progress-recorded' AND inline_json IS NOT NULL ORDER BY seq"
    ).bind(conversation).bind(operation).fetch_all(pool).await.map_err(|error|nomifun_common::AppError::Internal(error.to_string()))?;
    let events=rows.iter().map(|row|serde_json::from_str::<Value>(row))
        .collect::<Result<Vec<_>,_>>().map_err(|error|nomifun_common::AppError::Internal(error.to_string()))?;
    Ok(consumption_evidence(&events,report))
}

fn consumption_evidence(events:&[Value], report:&Value) -> bool {
    let case=&report["plan"]["current_conversation_case"];
    let plugin=report["delivery"]["plugin_id"].as_str().unwrap_or_default();
    let stable=format!("plugin:{plugin}/{}",case["action"].as_str().unwrap_or_default());
    let mut proposals=HashMap::new();
    let mut dispatches=HashMap::new();
    let mut installed=false;
    for payload in events {
        let event=&payload["event"];
        match event["event"].as_str() {
            Some("tool_call_completed")=>{
                if let Some(id)=event["call"]["call_id"].as_str() { proposals.insert(id.to_owned(),event["call"]["arguments"].clone()); }
            }
            Some("host_tool_dispatch")=>{
                if let Some(id)=event["dispatch"]["operation_id"].as_str() { dispatches.insert(id.to_owned(),event["dispatch"].clone()); }
            }
            Some("host_tool_settled")=>{
                let Some(dispatch)=event["operation_id"].as_str().and_then(|id|dispatches.get(id)) else { continue; };
                let Ok(result)=serde_json::from_value::<nomifun_engine_core::EngineToolResult>(event["result"].clone()) else { continue; };
                if result.is_error || !event["error"].is_null() { continue; }
                let Ok(output)=serde_json::from_str::<Value>(&result.output_text()) else { continue; };
                if dispatch["capability_id"]=="plugin.development" && dispatch["action_id"]=="plugin.development/install"
                    && output["delivery"]==report["delivery"] { installed=true; }
                else if installed && dispatch["capability_id"]==stable && dispatch["action_id"]==case["action"]
                    && dispatch["call_id"].as_str().and_then(|id|proposals.get(id))==Some(&case["input"])
                    && output==case["expected_output"] { return true; }
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
    fn current_consumption_requires_exact_dispatch_input_and_settlement_after_install() {
        let delivery=json!({"plugin_id":"plugin-id","artifact_digest":"digest","context_digest":"context"});
        let report=json!({"delivery":delivery,"plan":{"current_conversation_case":{
            "action":"normalize","input":{"text":"abc"},"expected_output":{"text":"ABC"}
        }}});
        let result=|call:&str,value:Value| serde_json::to_value(nomifun_engine_core::EngineToolResult::text(
            call.to_owned().into(),value.to_string(),false)).unwrap();
        let event=|event:Value|json!({"event":event});
        let mut events=vec![
            event(json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"install","call_id":"install","capability_id":"plugin.development","action_id":"plugin.development/install"}})),
            event(json!({"event":"host_tool_settled","operation_id":"install","result":result("install",json!({"delivery":delivery}))})),
            event(json!({"event":"tool_call_completed","call":{"call_id":"use","arguments":{"text":"abc"}}})),
            event(json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"use","call_id":"use","capability_id":"plugin:plugin-id/normalize","action_id":"normalize"}})),
            event(json!({"event":"host_tool_settled","operation_id":"use","result":result("use",json!({"text":"ABC"}))})),
        ];
        assert!(consumption_evidence(&events,&report));
        events[3]["event"]["dispatch"]["capability_id"]=json!("plugin.development");
        assert!(!consumption_evidence(&events,&report),"preview/creator calls cannot prove conversation consumption");
        events[3]["event"]["dispatch"]["capability_id"]=json!("plugin:plugin-id/normalize");
        events[2]["event"]["call"]["arguments"]=json!({"text":"different"});
        assert!(!consumption_evidence(&events,&report));
        assert!(!consumption_evidence(&events[2..],&report),"an installation boundary is required");
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

pub(super) fn has_authoring_approval(draft:&PluginDraftRecord,digest:&DigestHex,execution:&Value)->bool {
    let approval=&draft.verification["approval"];
    approval["approved"]==json!(true)
        && approval["artifact_digest"].as_str()==Some(digest.as_ref())
        && approval["execution_digest"].as_str()==Some(hash(execution).as_str())
        && approval["expires_at_ms"].as_i64().is_some_and(|until|until>nomifun_common::now_ms())
}

pub(super) async fn request_authoring_approval(
    state:&PluginRouterState,owner:&str,draft:&PluginDraftRecord,artifact:&PluginArtifact,execution:&Value,
)->Result<Value,PluginHttpError>{
    let existing=if let Some(id)=&draft.plugin_id {
        state.repository.get_plugin(owner,id).await?
    }else{None};
    let decision=plugin::consume_confirmation(state,owner,artifact,None,existing.as_ref()).await?;
    let mut next=draft.clone();
    let approved=decision.required.is_none();
    next.verification["approval"]=json!({
        "approved":approved,"artifact_digest":artifact.artifact_digest.as_ref(),
        "execution_digest":hash(execution),"expires_at_ms":nomifun_common::now_ms()+5*60*1000,
        "confirmation":decision.required,"execution":execution,
    });
    next.updated_at_ms=nomifun_common::now_ms();
    let next=state.repository.update_draft(&next,draft.revision).await?;
    state.events.send_to_user(owner,WebSocketMessage::new("plugin.authoring.changed",json!({
        "conversation_id":draft.source_conversation_id,"draft_id":draft.draft_id.as_ref(),"revision":next.revision,
    })));
    // Confirmation tokens are available only through the authenticated UI read.
    // Never return them to a model as an argument it can echo to approve itself.
    Ok(json!({"waiting_for_user":!approved,"approved_from_existing_grants":approved,
        "draft_id":draft.draft_id.as_ref(),"revision":next.revision,
        "message":if approved{"Existing instance grants cover this artifact; retry preview at this revision."}
            else{"Approve the actual code/permissions in the conversation card, then continue this draft."}}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApproveRequest { expected_revision:u64, confirmation_id:String, approved:bool }

pub(super) async fn approve(
    State(state):State<PluginRouterState>,Extension(user):Extension<CurrentUser>,
    Path(id):Path<String>,Json(request):Json<ApproveRequest>,
)->Result<Json<ApiResponse<Value>>,PluginHttpError>{
    let owner=user.id.to_string();
    let mut draft=plugin::draft_owned(&state,&owner,&id).await?;
    plugin::require_draft_revision(&draft,request.expected_revision)?;
    let pending=&draft.verification["approval"];
    if pending["confirmation"]["confirmation_id"].as_str()!=Some(request.confirmation_id.as_str())
        || pending["expires_at_ms"].as_i64().is_none_or(|until|until<=nomifun_common::now_ms())
    { return Err(PluginHttpError::conflict("Authoring confirmation expired or changed")); }
    let record=state.confirmations.lock().await.get(&request.confirmation_id).cloned()
        .ok_or_else(||PluginHttpError::conflict("Authoring confirmation is no longer active"))?;
    if record.owner_user_id!=owner
        || pending["artifact_digest"].as_str()!=Some(record.artifact_digest.as_ref())
        || record.expires_at_ms<nomifun_common::now_ms()
    { return Err(PluginHttpError::forbidden("Confirmation owner or Artifact changed")); }
    let files=state.drafts.freeze(&owner,&draft.draft_id)?;
    let artifact=state.artifacts.inspect_files(&files,&nomifun_plugin_platform::NeverCancel)?;
    if artifact.artifact_digest!=record.artifact_digest {return Err(PluginHttpError::conflict("Code changed before confirmation"));}
    draft.verification["approval"]["approved"]=json!(request.approved);
    draft.updated_at_ms=nomifun_common::now_ms();
    let updated=state.repository.update_draft(&draft,request.expected_revision).await?;
    state.events.send_to_user(&owner,WebSocketMessage::new("plugin.authoring.changed",json!({
        "conversation_id":updated.source_conversation_id,"draft_id":id,"revision":updated.revision,
    })));
    Ok(Json(ApiResponse::ok(json!({"approved":request.approved,"revision":updated.revision}))))
}

pub(super) async fn save_authoring_draft(
    state:&PluginRouterState,owner:&str,id:&str,mut request:SavePluginDraftRequest,draft:&PluginDraftRecord,
)->Result<SavePluginDraftResponseDto,PluginHttpError>{
    if request.expected_plugin_revision.is_none(){request.expected_plugin_revision=draft.base_revision;}
    if let Some(confirmation)=draft.verification["approval"]["confirmation"]["confirmation_id"].as_str(){
        if draft.verification["approval"]["approved"]!=json!(true){
            return Err(PluginHttpError::forbidden("The user has not approved this Artifact"));
        }
        request.permission_confirmation_id=Some(confirmation.to_owned());
    }
    let outcome=plugin::save_draft_owned(state,owner,id,request).await?;
    if let PluginInstallOutcomeDto::ConfirmationRequired{confirmation}=&outcome.result {
        let mut current=plugin::draft_owned(state,owner,id).await?;
        current.verification["approval"]=json!({"approved":false,
            "artifact_digest":current.verification["artifact_digest"],"confirmation":confirmation,
            "execution_digest":hash(&current.verification["execution"]),
            "execution":current.verification["execution"],"expires_at_ms":nomifun_common::now_ms()+5*60*1000,
        });
        current.updated_at_ms=nomifun_common::now_ms();
        state.repository.update_draft(&current,current.revision).await?;
    }
    Ok(outcome)
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
    let mut public=draft.verification.clone();
    public.as_object_mut().expect("verification object").remove("approval");
    let confirmation=draft.verification["approval"]["confirmation"].clone();
    Ok(Json(ApiResponse::ok(json!({
        "draft":plugin::draft_detail(&state,&draft)?,"verification":public,"commands":commands,
        "confirmation":if draft.verification["approval"]["approved"]==json!(true){Value::Null}else{confirmation},
    }))))
}

/// Persist the immutable requirement before effects or an ephemeral UI wait.
/// Dropping the caller leaves a failed/pending case rather than erasing it.
pub(super) async fn begin_authoring_case(
    state: &PluginRouterState,
    draft: &PluginDraftRecord,
    case_name: &str,
    oracle: Value,
) -> Result<PluginDraftRecord, PluginHttpError> {
    let prior = &draft.verification["acceptance"][case_name];
    if !prior.is_null() && *prior != oracle {
        return Err(PluginHttpError::conflict(
            "PLUGIN_ORACLE_CHANGED: keep the accepted case unchanged while repairing",
        ));
    }
    let mut next = draft.clone();
    next.verification["acceptance"][case_name] = oracle.clone();
    next.verification["cases"][case_name] = json!({
        "kind": oracle["kind"], "passed": false, "state": "running",
    });
    next.verification.as_object_mut().expect("verification object").remove("delivery");
    next.updated_at_ms = nomifun_common::now_ms();
    state.repository.update_draft(&next, draft.revision).await.map_err(Into::into)
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

pub(super) async fn run_authoring_ui_test(
    state:&PluginRouterState,owner:&str,conversation:&str,turn:&str,input:Value,
)->Result<Value,PluginHttpError>{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {draft_id:String,expected_revision:u64,case_name:String,steps:Vec<Value>}
    let request:Request=serde_json::from_value(input).map_err(|error|PluginHttpError::bad_request(&error.to_string()))?;
    let mut draft=plugin::draft_owned(state,owner,&request.draft_id).await?;
    plugin::require_draft_revision(&draft,request.expected_revision)?;
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
    let oracle=json!({"kind":"ui","steps":request.steps});
    let descriptor:PluginSurfaceDescriptorDto=serde_json::from_value(draft.verification["surface"].clone())
        .map_err(|_|error("UI preview descriptor is missing"))?;
    let mut surface_guard = AuthoringSurfaceGuard::new(state, owner, &descriptor);
    draft=begin_authoring_case(state,&draft,&request.case_name,oracle).await?;
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
    draft.verification["ui_ready"]=json!(passed);
    draft.updated_at_ms=nomifun_common::now_ms();
    let updated=state.repository.update_draft(&draft,execution_revision).await?;
    surface_guard.retain();
    state.events.send_to_user(owner,WebSocketMessage::new("plugin.authoring.changed",json!({
        "conversation_id":conversation,"draft_id":request.draft_id,"revision":updated.revision,
    })));
    Ok(json!({"draft_id":request.draft_id,"revision":updated.revision,"passed":passed,
        "persistence_checked":persistence,"verification_digest":hash(&updated.verification)}))
}
