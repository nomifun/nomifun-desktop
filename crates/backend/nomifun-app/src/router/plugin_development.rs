//! Native Agent consumer of the same owner commands used by Plugin Core UI.
//! No model, private chat history, worker or second installer lives here.
use std::{collections::BTreeMap, sync::Arc};
use async_trait::async_trait;
use nomifun_agent_contracts::{CanonicalSchemaRef, ResolvedCapability, StrictJsonValue, digest_payload, plugin_action_id};
use nomifun_agent_kernel::CapabilityInvocationContext;
use nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver;
use nomifun_api_types::*;
use nomifun_plugin_development::{MODULE_ID, PluginDevelopmentHost};
use nomifun_plugin_platform::{NeverCancel, PluginDraftRecord, PluginRepository, PluginDispatchOptions, PluginCancellation};
use nomifun_realtime::UserEventSink;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use axum::{Router, Json, extract::{State, Extension}, routing::post};
use nomifun_auth::CurrentUser;

use super::plugin::{self, PluginRouterState};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag="kind", rename_all="snake_case", deny_unknown_fields)]
pub(super) enum AgentSelection {
    Preset { #[serde(rename="presetId")] preset_id:String },
    Template { #[serde(rename="templateKey")] template_key:String },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreflightRequest { selection:AgentSelection }

pub(super) fn preflight_routes(state:super::nomi_core_session::NomiCoreAgentApiState)->Router {
    Router::new().route("/api/conversations/plugin-preflight",post(preflight))
        .route("/api/agent-sessions/{id}/plugin-continuation",post(continue_with_input)).with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationInput { content:String, #[serde(default)] files:Vec<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContinuationRequest { request:nomifun_agent_session::NativeResumeRequest, input:ContinuationInput }

async fn continue_with_input(
    State(state):State<super::nomi_core_session::NomiCoreAgentApiState>,
    Extension(user):Extension<CurrentUser>,axum::extract::Path(id):axum::extract::Path<String>,
    Json(body):Json<ContinuationRequest>,
)->Result<Json<ApiResponse<nomifun_agent_session::NativeResumeReceipt>>,super::nomi_core_session::NomiCoreApiError>{
    use nomifun_common::AppError;
    nomifun_common::validate_uuidv7(&id).map_err(|error|AppError::BadRequest(error.to_string()))?;
    if body.request.budget != Default::default() || body.request.cleanup_attestation.is_some() {
        return Err(AppError::BadRequest("Plugin reply does not grant budget or attest cleanup".into()).into());
    }
    let principal=nomifun_agent_contracts::PrincipalRef{principal_kind:"user".into(),principal_id:user.id.to_string()};
    let session=nomifun_agent_contracts::AgentSessionId::from(id.clone());
    let store=state.session_owner.canonical().store();
    let live=store.get_live_session(&session).await.map_err(|error|AppError::Conflict(error.to_string()))?;
    if live.owner_ref != principal { return Err(AppError::Forbidden("Plugin task belongs to another owner".into()).into()); }
    let binding:AgentBindingValueDto=serde_json::from_value(serde_json::to_value(&live.agent_binding)?)?;
    let (_,_,snapshot)=state.control_plane.saved_binding_artifacts(&user.id.to_string().into(),&binding).await?;
    let module=snapshot.content.enabled_capabilities.iter().find(|capability|
        capability.consumption.is_contribution() && capability.capability.id.as_ref()==MODULE_ID);
    if module.is_none() || nomifun_plugin_development::CREATE_ACTIONS.iter().any(|action|
        !module.expect("checked module").action_allowlist.iter().any(|allowed|allowed.as_ref()==*action)) {
        return Err(AppError::UnprocessableEntity("Plugin development is not enabled for this conversation".into()).into());
    }
    let input=super::nomi_core_session::bounded_turn_input(json!({"content":body.input.content,"files":body.input.files}))?;
    store.append_paused_native_input(&principal,&session,&body.request,
        StrictJsonValue(super::nomi_core_session::canonical_turn_input(&input)),
        &["PLUGIN_AUTHORIZATION_REQUIRED","PLUGIN_VERIFICATION_REQUIRED","PLUGIN_DELIVERY_REQUIRED","PLUGIN_CURRENT_CONVERSATION_PENDING"],
    ).await.map_err(|error|AppError::Conflict(error.to_string()))?;
    super::nomi_core_session::native_execution_control::resume(State(state),
        Extension(nomifun_agent_control_plane::AuthenticatedOwner(user.id.to_string().into())),
        axum::extract::Path(id),Json(body.request)).await
}

async fn preflight(
    State(state):State<super::nomi_core_session::NomiCoreAgentApiState>,
    Extension(user):Extension<CurrentUser>,Json(request):Json<PreflightRequest>,
)->Result<Json<ApiResponse<Value>>,super::nomi_core_session::NomiCoreApiError>{
    let owner=nomifun_agent_contracts::UserId::from(user.id.to_string());
    let catalog=state.control_plane.catalog()?;
    let catalog_value=serde_json::to_value(&catalog).expect("catalog");
    let registered=catalog_value["modules"].as_array().is_some_and(|modules|
        modules.iter().any(|module|module["module"]["id"].as_str()==Some(MODULE_ID)));
    let document=match &request.selection {
        AgentSelection::Preset{preset_id}=>{
            match state.control_plane.editor(&owner,preset_id,None).await {
                Ok(editor) if editor.revision.is_some()=>serde_json::to_value(editor.revision.expect("checked").document).expect("document"),
                Err(error) if error.status() != axum::http::StatusCode::NOT_FOUND => return Err(error.into()),
                _=>return Ok(Json(ApiResponse::ok(json!({"status":"configure_agent","reason":"AGENT_DEFAULT_UNAVAILABLE","selection":request.selection,"owner_user_id":owner.as_ref()})))),
            }
        },
        AgentSelection::Template{template_key}=>{
            let library=state.control_plane.library(&owner).await?;
            let value=serde_json::to_value(library).expect("library");
            match value["official_templates"].as_array().and_then(|templates|
                templates.iter().find(|template|template["template_key"].as_str()==Some(template_key.as_str()))) {
                Some(template)=>template["seed"].clone(),
                None=>return Ok(Json(ApiResponse::ok(json!({"status":"configure_agent","reason":"AGENT_DEFAULT_UNAVAILABLE","selection":request.selection,"owner_user_id":owner.as_ref()})))),
            }
        },
    };
    let selected=document["enabled_capabilities"].as_array().and_then(|capabilities|
        capabilities.iter().find(|selection|selection["capability"]["id"].as_str()==Some(MODULE_ID)));
    let granted=selected.and_then(|selection|selection["action_allowlist"].as_array());
    let missing=nomifun_plugin_development::CREATE_ACTIONS.iter().filter(|action|
        !granted.is_some_and(|actions|actions.iter().any(|granted|granted.as_str()==Some(**action))))
        .copied().collect::<Vec<_>>();
    Ok(Json(ApiResponse::ok(json!({
        "status":if !registered{"unavailable"}else if !missing.is_empty(){"configure_agent"}else{"ready"},
        "reason":if !registered{"PLUGIN_MODULE_UNAVAILABLE"}else if selected.is_none(){"PLUGIN_MODULE_DISABLED"}else if !missing.is_empty(){"PLUGIN_ACTIONS_REQUIRED"}else{""},
        "selection":request.selection,"owner_user_id":owner.as_ref(),"missing_actions":missing,
        "required_actions":nomifun_plugin_development::CREATE_ACTIONS,
    }))))
}

pub(super) struct Host {
    state: PluginRouterState,
    owner: Arc<str>,
    events: Arc<dyn UserEventSink>,
}

impl Host {
    pub(super) fn new(state: PluginRouterState, services: &crate::services::AppServices) -> Self {
        Self { state, owner: services.authoritative_user_id.clone(), events: services.event_bus.clone() }
    }

    async fn source(&self, context: &CapabilityInvocationContext) -> Result<plugin::PluginDraftSource, String> {
        if context.principal.principal_kind != "user" || context.principal.principal_id != self.owner.as_ref() {
            return Err("PLUGIN_PERMISSION_DENIED: development requires the installation owner".into());
        }
        let store = nomifun_agent_session::AgentSessionStore::from_pool(self.state.repository.pool().clone())
            .await.map_err(|error| error.to_string())?;
        let session = store.get_live_session(&context.agent_session_id).await.map_err(|error| error.to_string())?;
        if session.owner_ref != context.principal {
            return Err("PLUGIN_PERMISSION_DENIED: conversation belongs to another owner".into());
        }
        let receipt = store.read_turn_receipt(&context.agent_session_id, &context.turn_id).await.map_err(|error| error.to_string())?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Err("PLUGIN_TURN_ENDED: this creation turn is no longer running".into());
        }
        let message_id = receipt.started_event.and_then(|event| match event.payload {
            nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(value) => value.0.get("source_message_id").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        }).ok_or("PLUGIN_SOURCE_MISSING: admitted user message is missing")?;
        Ok(plugin::PluginDraftSource {
            conversation_id: context.agent_session_id.as_ref().to_owned(), message_id,
            operation_key: format!("{}:{}", context.action_id.as_ref(), context.idempotency_key.as_ref()),
        })
    }

    async fn draft(&self, owner: &str, conversation: &str, id: &str) -> Result<PluginDraftRecord, String> {
        let draft = plugin::draft_owned(&self.state, owner, id).await.map_err(core_error)?;
        if draft.source_conversation_id.as_deref() != Some(conversation) {
            return Err("PLUGIN_DRAFT_SCOPE_MISMATCH: open a working copy in this conversation first".into());
        }
        Ok(draft)
    }

    fn changed(&self, conversation: &str, draft: &PluginDraftRecord, descriptor: Option<PluginSurfaceDescriptorDto>) {
        self.events.send_to_user(&self.owner, WebSocketMessage::new(
            "plugin.authoring.changed", json!({
                "conversation_id":conversation,"draft_id":draft.draft_id.as_ref(),
                "revision":draft.revision,"descriptor":descriptor,
            }),
        ));
        self.events.send_to_user(&self.owner, WebSocketMessage::new("plugins.changed", json!({})));
    }

    async fn plan(&self, owner: &str, conversation: &str, message: &str, input: Value) -> Result<Value,String> {
        let (id,fields)=draft_request(input)?;
        let mut draft=self.draft(owner,conversation,&id).await?;
        plugin::require_draft_revision(&draft,revision(&fields)?).map_err(core_error)?;
        let mut plan=fields["plan"].clone();
        nomifun_plugin_development::validate_plan(&plan)?;
        for case in plan["cases"].as_object_mut().expect("validated cases").values_mut() {
            if case["kind"]=="action" && case["restart"]==json!(false) { case.as_object_mut().expect("case").remove("restart"); }
        }
        if draft.verification["task_message_id"].as_str()==Some(message) && !draft.verification["plan"].is_null() {
            if draft.verification["plan"]!=plan { return Err("PLUGIN_PLAN_CHANGED: retain the accepted requirements while repairing; start a new request for changed requirements".into()); }
            return wire(plugin::draft_detail(&self.state,&draft).map_err(core_error)?);
        }
        let peers:Vec<String>=nomifun_db::sqlx::query_scalar(
            "SELECT verification_json FROM plugin_drafts WHERE owner_user_id=? AND source_conversation_id=? AND json_extract(verification_json,'$.task_message_id')=? AND draft_id<>?"
        ).bind(owner).bind(conversation).bind(message).bind(&id).fetch_all(self.state.repository.pool()).await.map_err(|error|error.to_string())?;
        for peer in peers {
            let peer:Value=serde_json::from_str(&peer).map_err(|error|error.to_string())?;
            if !peer["plan"].is_null() && (peer["plan"]["outputs"]!=plan["outputs"] || peer["plan"]["output_key"]==plan["output_key"]) {
                return Err("PLUGIN_OUTPUT_PLAN_CONFLICT: all drafts must retain the same requested outputs and use distinct output keys".into());
            }
        }
        plugin::revoke_draft_surfaces(&self.state,&draft.draft_id).await.map_err(core_error)?;
        draft.verification=json!({"task_message_id":message,"plan":plan,"acceptance":plan["cases"]});
        draft.updated_at_ms=nomifun_common::now_ms();
        let next=self.state.repository.update_draft(&draft,draft.revision).await.map_err(|error|error.to_string())?;
        self.changed(conversation,&next,None);
        wire(plugin::draft_detail(&self.state,&next).map_err(core_error)?)
    }

    async fn apply(&self, owner: &str, conversation: &str, message: &str, input: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Apply { draft_id: String, expected_revision: u64, files: BTreeMap<String,String>, #[serde(default)] delete: Vec<String> }
        let request: Apply = parse(input)?;
        let mut draft = self.draft(owner, conversation, &request.draft_id).await?;
        plugin::require_draft_revision(&draft, request.expected_revision).map_err(core_error)?;
        if draft.verification["task_message_id"].as_str()!=Some(message) || draft.verification["plan"].is_null() {
            return Err("PLUGIN_PLAN_REQUIRED: record this request's required outputs and acceptance cases before editing".into());
        }
        let mut files = self.state.drafts.freeze(owner, &draft.draft_id).map_err(|error| error.to_string())?;
        for path in request.delete {
            if path == "nomifun.plugin.json" { return Err("PLUGIN_INVALID_INPUT: the manifest cannot be deleted".into()); }
            if files.remove(&path).is_none() { return Err(format!("PLUGIN_INVALID_INPUT: file {path} does not exist")); }
        }
        files.extend(request.files.into_iter().map(|(path,text)| (path,text.into_bytes())));
        let mut replacement = self.state.drafts.stage_exact_replacement(owner, &draft.draft_id, &files, &NeverCancel).map_err(|error| error.to_string())?;
        let same_task = draft.verification["task_message_id"].as_str() == Some(message)
            || (draft.verification["task_message_id"].is_null() && draft.source_message_id.as_deref() == Some(message));
        let acceptance = if same_task {
            draft.verification["acceptance"].as_object().cloned().unwrap_or_default()
        } else { serde_json::Map::new() };
        draft.verification = json!({"acceptance":acceptance,"task_message_id":message,"plan":draft.verification["plan"]});
        draft.updated_at_ms = nomifun_common::now_ms();
        plugin::revoke_draft_surfaces(&self.state, &draft.draft_id).await.map_err(core_error)?;
        replacement.publish().map_err(|error| error.to_string())?;
        let updated = match self.state.repository.update_draft(&draft, request.expected_revision).await {
            Ok(value) => { replacement.commit().map_err(|error| error.to_string())?; value },
            Err(error) => { replacement.rollback().map_err(|rollback| format!("{error}; rollback: {rollback}"))?; return Err(error.to_string()); }
        };
        self.changed(conversation, &updated, None);
        wire(plugin::draft_detail(&self.state, &updated).map_err(core_error)?)
    }

    async fn check(&self, owner: &str, conversation: &str, input: Value) -> Result<Value,String> {
        let (id, request) = draft_request(input)?;
        let revision = revision(&request)?;
        let mut draft = self.draft(owner, conversation, &id).await?;
        plugin::require_draft_revision(&draft, revision).map_err(core_error)?;
        let files = self.state.drafts.freeze(owner, &draft.draft_id).map_err(|error| error.to_string())?;
        let artifact = match self.state.artifacts.inspect_files(&files, &NeverCancel) {
            Ok(value) => value,
            Err(error) => return Ok(json!({"passed":false,"stage":"structure","draft_id":id,"revision":revision,"diagnostics":[{"code":"PLUGIN_PACKAGE_INVALID","message":error.to_string()}]})),
        };
        let plan=&draft.verification["plan"];
        let kind=plan["outputs"].as_array().and_then(|outputs|outputs.iter().find(|output|output["key"]==plan["output_key"]))
            .and_then(|output|output["kind"].as_str());
        let actual=match (artifact.manifest.has_ui(),artifact.manifest.has_service()) {
            (true,true)=>"mixed",(true,false)=>"ui",(false,true)=>"headless",_=>"invalid",
        };
        if kind!=Some(actual) { return Ok(json!({"passed":false,"stage":"requirements","draft_id":id,"revision":revision,
            "diagnostics":[{"code":"PLUGIN_SHAPE_MISMATCH","message":"The package must implement its planned UI/Service shape"}]})); }
        if let Some(case)=plan.get("current_conversation_case") {
            if !artifact.manifest.bindings.iter().any(|binding|
                binding.point==nomifun_agent_contracts::PluginBindingPoint::AgentTool && binding.action==case["action"]) {
                return Ok(json!({"passed":false,"stage":"requirements","draft_id":id,"revision":revision,
                    "diagnostics":[{"code":"PLUGIN_CURRENT_AGENT_TOOL_REQUIRED","message":"Current conversation use requires a declared agent.tool binding for the planned Action"}]}));
            }
        }
        draft.verification = json!({
            "artifact_digest":artifact.artifact_digest.as_ref(),"structure_passed":true,
            "has_ui":artifact.manifest.has_ui(),"has_service":artifact.manifest.has_service(),
            "cases":{},"runtime_ready":false,"ui_ready":false,
            "acceptance":draft.verification["acceptance"].as_object().cloned().unwrap_or_default(),
            "source_message_id":draft.source_message_id,
            "task_message_id":draft.verification["task_message_id"],
            "approval":draft.verification["approval"],
            "plan":draft.verification["plan"],
        });
        draft.updated_at_ms = nomifun_common::now_ms();
        let updated = self.state.repository.update_draft(&draft, revision).await.map_err(|error| error.to_string())?;
        self.changed(conversation, &updated, None);
        Ok(json!({
            "passed":true,"stage":"structure","draft_id":id,"revision":updated.revision,
            "artifact_digest":artifact.artifact_digest.as_ref(),"diagnostics":[],
            "runtime_checks_pending":true,
            "next":"preview the exact package, then execute required business cases; structure is not usable delivery",
        }))
    }

    async fn preview(&self, owner: &str, conversation: &str, input: Value) -> Result<Value,String> {
        let (id, mut fields) = draft_request(input)?;
        let revision = revision(&fields)?;
        let mut draft = self.draft(owner, conversation, &id).await?;
        plugin::require_draft_revision(&draft, revision).map_err(core_error)?;
        let files = self.state.drafts.freeze(owner, &draft.draft_id).map_err(|error| error.to_string())?;
        let artifact = self.state.artifacts.inspect_files(&files, &NeverCancel).map_err(|error| error.to_string())?;
        let config = fields.get("config").cloned().unwrap_or_else(|| json!({}));
        let permissions = fields.get("permissions").cloned().unwrap_or_else(|| json!([]));
        let credentials = fields.get("credential_bindings").cloned().unwrap_or_else(|| json!({}));
        let execution = json!({"config":config,"permissions":permissions,"credential_bindings":credentials});
        let verification_context=super::plugin_authoring::verification_context(self.state.repository.pool(),&credentials).await.map_err(core_error)?;
        if (artifact.manifest.has_service() || !artifact.manifest.permissions.is_empty() || !artifact.manifest.secrets.is_empty()) && !plugin::has_authoring_approval(&draft, &artifact.artifact_digest, &execution) {
            return plugin::request_authoring_approval(&self.state, owner, &draft, &artifact, &execution).await.map_err(core_error);
        }
        if draft.verification.get("artifact_digest").and_then(Value::as_str) != Some(artifact.artifact_digest.as_ref()) {
            return Err("PLUGIN_CHECK_REQUIRED: check this exact revision before preview".into());
        }
        fields["config"] = config;
        fields["access"] = json!({"permissions":permissions,"credential_bindings":credentials});
        fields.as_object_mut().expect("request").remove("permissions");
        fields.as_object_mut().expect("request").remove("credential_bindings");
        let preview = plugin::preview_draft_owned(&self.state, owner, &id, parse(fields)?).await.map_err(core_error)?;
        let mut surface_guard = super::plugin_authoring::AuthoringSurfaceGuard::new(&self.state, owner, &preview.descriptor);
        draft.verification["execution"] = execution;
        draft.verification["context"] = verification_context;
        draft.verification["runtime_ready"] = json!(true);
        draft.verification["surface"] = wire(preview.descriptor.clone())?;
        draft.verification["cases"] = json!({});
        draft.verification["ui_ready"] = json!(false);
        draft.verification.as_object_mut().expect("verification").remove("delivery");
        draft.verification.as_object_mut().expect("verification").remove("installed_observation");
        draft.updated_at_ms = nomifun_common::now_ms();
        let updated = self.state.repository.update_draft(&draft, revision).await.map_err(|error| error.to_string())?;
        self.changed(conversation, &updated, Some(preview.descriptor.clone()));
        surface_guard.retain();
        Ok(json!({"draft_id":id,"revision":updated.revision,"descriptor":preview.descriptor,
            "has_ui":artifact.manifest.has_ui(),"has_service":artifact.manifest.has_service(),
            "verification_digest":digest_payload(&updated.verification).map_err(|error| error.to_string())?.as_ref(),
            "business_checks_pending":true}))
    }

    async fn test_action(&self, owner: &str, conversation: &str, input: Value) -> Result<Value,String> {
        let (id, fields) = draft_request(input)?;
        let mut draft = self.draft(owner, conversation, &id).await?;
        let expected_revision = revision(&fields)?;
        plugin::require_draft_revision(&draft, expected_revision).map_err(core_error)?;
        let artifact = self.state.artifacts.inspect_files(&self.state.drafts.freeze(owner,&draft.draft_id).map_err(|error|error.to_string())?, &NeverCancel).map_err(|error|error.to_string())?;
        if draft.verification.get("runtime_ready") != Some(&json!(true))
            || draft.verification.get("artifact_digest").and_then(Value::as_str) != Some(artifact.artifact_digest.as_ref())
        { return Err("PLUGIN_PREVIEW_REQUIRED: run the exact package first".into()); }
        let action = string(&fields,"action")?;
        let case = string(&fields,"case_name")?;
        let restart=fields["restart"]==json!(true);
        let mut oracle = json!({"kind":"action","action":action,"input":fields["input"],"expected_output":fields["expected_output"]});
        if restart { oracle["restart"]=json!(true); }
        draft = super::plugin_authoring::begin_authoring_case(&self.state,&draft,case,oracle)
            .await.map_err(core_error)?;
        let mut restart_guard=None;
        if restart {
            let old_generation=draft.verification["surface"]["surface_generation"].as_u64().ok_or("PLUGIN_PREVIEW_REQUIRED")?;
            let execution=&draft.verification["execution"];
            let preview=plugin::preview_draft_owned(&self.state,owner,&id,parse(json!({
                "expected_revision":draft.revision,"config":execution["config"],
                "access":{"permissions":execution["permissions"],"credential_bindings":execution["credential_bindings"]}
            }))?).await.map_err(core_error)?;
            if preview.descriptor.surface_generation<=old_generation { return Err("PLUGIN_RESTART_NOT_OBSERVED".into()); }
            restart_guard=Some(super::plugin_authoring::AuthoringSurfaceGuard::new(&self.state,owner,&preview.descriptor));
            draft.verification["surface"]=wire(preview.descriptor)?;
            draft=self.state.repository.update_draft(&draft,draft.revision).await.map_err(|error|error.to_string())?;
        }
        let execution_revision = draft.revision;
        self.changed(conversation,&draft,None);
        let cancellation = PluginCancellation::new();
        let _cancel_on_drop = CancelOnDrop(cancellation.clone());
        let outcome = self.state.registry.dispatch_action(
            &plugin_action_id(&nomifun_agent_contracts::PluginId::from(id.clone()), action),
            StrictJsonValue(fields["input"].clone()),
            PluginDispatchOptions { expected_artifact_digest:Some(artifact.artifact_digest.clone()), cancellation, call_chain:Vec::new() },
        ).await;
        let (output, error) = match outcome {
            Ok(output) => (output.0, None),
            Err(error) => (Value::Null, Some(error.to_string())),
        };
        let expected = &fields["expected_output"];
        let passed = error.is_none() && &output == expected;
        draft.verification["cases"][case] = json!({"kind":"action","action":action,"input":fields["input"],"expected_output":expected,"actual_output":output,"passed":passed,"error":error,
            "persistence_checked":restart && passed,"state":if passed{"passed"}else{"failed"}});
        draft.updated_at_ms=nomifun_common::now_ms();
        let updated=self.state.repository.update_draft(&draft,execution_revision).await.map_err(|error|error.to_string())?;
        if let Some(guard)=&mut restart_guard { guard.retain(); }
        self.changed(conversation,&updated,None);
        Ok(json!({"draft_id":id,"revision":updated.revision,"passed":passed,"actual_output":output,"error":error,
            "verification_digest":digest_payload(&updated.verification).map_err(|error|error.to_string())?.as_ref()}))
    }

    async fn install(&self, owner:&str, conversation:&str, turn:&str, input:Value)->Result<Value,String> {
        let (id,mut request)=draft_request(input)?;
        let draft=self.draft(owner,conversation,&id).await?;
        plugin::require_draft_revision(&draft,revision(&request)?).map_err(core_error)?;
        let artifact=self.state.artifacts.inspect_files(&self.state.drafts.freeze(owner,&draft.draft_id).map_err(|error|error.to_string())?,&NeverCancel).map_err(|error|error.to_string())?;
        let verification_digest=string(&request,"verification_digest")?.to_owned();
        if digest_payload(&draft.verification).map_err(|error|error.to_string())?.as_ref()!=verification_digest
            || draft.verification["artifact_digest"].as_str()!=Some(artifact.artifact_digest.as_ref())
            || draft.verification["runtime_ready"]!=json!(true)
            || (artifact.manifest.has_ui() && draft.verification["ui_ready"]!=json!(true))
        { return Err("PLUGIN_VERIFICATION_REQUIRED: the exact package has not passed its required runtime/UI checks".into()); }
        let cases=draft.verification["cases"].as_object().ok_or("PLUGIN_BUSINESS_CHECK_REQUIRED")?;
        if cases.is_empty() || cases.values().any(|case|case["passed"]!=json!(true))
            || !nomifun_plugin_development::plan_evidence_complete(&draft.verification)
            || draft.verification["acceptance"].as_object().is_some_and(|accepted|accepted.keys().any(|key|cases.get(key).is_none_or(|case|case["passed"]!=json!(true)))) {
            return Err("PLUGIN_BUSINESS_CHECK_REQUIRED: execute the requirement cases and repair failures".into());
        }
        request.as_object_mut().expect("object").remove("verification_digest");
        let execution=draft.verification["execution"].clone();
        let config=request.get("config").cloned().unwrap_or_else(||execution["config"].clone());
        let credentials=request.get("credential_bindings").cloned().unwrap_or_else(||execution["credential_bindings"].clone());
        if config!=execution["config"] || credentials!=execution["credential_bindings"] {
            return Err("PLUGIN_CONFIGURATION_CHANGED: preview and verify the new configuration".into());
        }
        if draft.verification["context"]!=super::plugin_authoring::verification_context(self.state.repository.pool(),&credentials).await.map_err(core_error)? {
            return Err("PLUGIN_VERIFICATION_CONTEXT_CHANGED: runtime or Credential changed; preview and verify again".into());
        }
        request["config"]=config; request["credential_bindings"]=credentials.clone();
        let outcome=plugin::save_authoring_draft(&self.state,owner,&id,parse(request)?,&draft).await.map_err(core_error)?;
        let mut current=plugin::draft_owned(&self.state,owner,&id).await.map_err(core_error)?;
        if let PluginInstallOutcomeDto::ConfirmationRequired { .. }=&outcome.result {
            self.changed(conversation,&current,None);
            return Ok(json!({"waiting_for_user":true,"draft_id":id,"revision":current.revision,
                "message":"Review additional privileges in the conversation card, then continue."}));
        }
        if let PluginInstallOutcomeDto::Installed { plugin }=&outcome.result {
            if !plugin.summary.enabled
                || plugin.summary.active.artifact_digest!=artifact.artifact_digest.as_ref()
                || plugin.summary.last_error.is_some()
            {return Err("PLUGIN_DELIVERY_FAILED: installed version is unavailable or differs from verified bytes".into());}
            if super::plugin_authoring::verification_context(self.state.repository.pool(),&credentials).await.ok()
                .as_ref()!=Some(&current.verification["context"]) {
                current.verification.as_object_mut().expect("verification").remove("delivery");
                current=self.state.repository.update_draft(&current,current.revision).await.map_err(|error|error.to_string())?;
                self.changed(conversation,&current,None);
                return Ok(json!({"installed":true,"delivery_pending":true,"stage":"verification_context",
                    "draft_id":id,"revision":current.revision,"plugin_id":plugin.summary.plugin_id,
                    "diagnostic":"Credential or verifier changed during installation; preview and verify again."}));
            }
            let context_digest=digest_payload(&current.verification["context"]).map_err(|error|error.to_string())?;
            let plugin_id=nomifun_agent_contracts::PluginId::from(plugin.summary.plugin_id.clone());
            let mut published=Vec::new();
            for binding in &artifact.manifest.bindings {
                let actions=self.state.registry.list_binding(binding.point.clone()).map_err(|error|error.to_string())?;
                if !actions.iter().any(|action| action.publication.plugin_id==plugin_id
                    && action.publication.action_id==binding.action
                    && action.availability==nomifun_plugin_platform::PluginActionAvailability::Available)
                {return Err("PLUGIN_DELIVERY_FAILED: the declared production Binding is unavailable".into());}
                published.push(json!({"point":binding.point,"action":binding.action}));
            }
            let mut probes=Vec::new();
            for (case_name,case) in draft.verification["acceptance"].as_object().into_iter().flatten() {
                let Some(action)=case["action"].as_str() else {continue;};
                if case["kind"]!=json!("action")
                    || artifact.manifest.actions.get(action).is_none_or(|action|
                        action.effect!=nomifun_agent_contracts::PluginActionEffect::Read)
                    || !artifact.manifest.bindings.iter().any(|binding|binding.action==action)
                {continue;}
                let cancellation=PluginCancellation::new();
                let _cancel_on_drop=CancelOnDrop(cancellation.clone());
                // Actual installed dispatch validates both schemas. Production
                // storage is independent of Preview, so do not compare its data
                // with temporary test data or replay write/external cases.
                let output=self.state.registry.dispatch_action(&plugin_action_id(&plugin_id,action),
                    StrictJsonValue(case["input"].clone()), PluginDispatchOptions {
                        expected_artifact_digest:Some(artifact.artifact_digest.clone()), cancellation, call_chain:Vec::new(),
                    }).await.map_err(|error|format!("PLUGIN_DELIVERY_FAILED: installed Action {action}: {error}"))?;
                probes.push(json!({"case_name":case_name,"action":action,"schema_passed":true,"actual_output":output.0}));
            }
            current.verification["installed_observation"]=json!({
                "plugin_id":plugin_id.as_ref(),"artifact_digest":artifact.artifact_digest.as_ref(),
                "plugin_revision":plugin.summary.revision,"bindings":published,"read_probes":probes,
                "context_digest":context_digest.as_ref(),
                "observed_at_ms":nomifun_common::now_ms(),
            });
            if artifact.manifest.has_ui() {
                current=super::plugin_authoring::run_installed_ui_check(&self.state,owner,conversation,
                    &current,&plugin.summary.plugin_id,plugin.summary.revision,turn).await.map_err(core_error)?;
                if current.verification["installed_observation"]["ui_ready"]!=json!(true) {
                    self.changed(conversation,&current,None);
                    return Ok(json!({"installed":true,"delivery_pending":true,"stage":"installed_ui",
                        "draft_id":id,"revision":current.revision,"plugin_id":plugin.summary.plugin_id,
                        "diagnostic":current.verification["installed_observation"]["ui_error"],
                        "verification_digest":digest_payload(&current.verification).map_err(|error|error.to_string())?.as_ref()}));
                }
            }
            if super::plugin_authoring::verification_context(self.state.repository.pool(),&credentials).await.ok()
                .as_ref()!=Some(&current.verification["context"]) {
                self.changed(conversation,&current,None);
                return Ok(json!({"installed":true,"delivery_pending":true,"stage":"verification_context",
                    "draft_id":id,"revision":current.revision,"plugin_id":plugin.summary.plugin_id,
                    "diagnostic":"Credential or verifier changed during formal verification; preview and verify again."}));
            }
            current.verification["delivery"]=json!({
                "plugin_id":plugin.summary.plugin_id,"artifact_digest":plugin.summary.active.artifact_digest,
                "plugin_revision":plugin.summary.revision,"has_ui":plugin.summary.has_ui,
                "has_service":plugin.summary.has_service,"installed_at_ms":nomifun_common::now_ms(),
                "context_digest":context_digest.as_ref(),
                "verified_source_message_id":current.source_message_id,
            });
            current.updated_at_ms=nomifun_common::now_ms();
            current=self.state.repository.update_draft(&current,current.revision).await.map_err(|error|error.to_string())?;
        }
        self.changed(conversation,&current,None);
        let mut result=wire(outcome)?;
        result["draft"]=wire(plugin::draft_summary(&self.state,&current).map_err(core_error)?)?;
        result["delivery"]=current.verification["delivery"].clone();
        Ok(result)
    }
}

#[async_trait]
impl PluginDevelopmentHost for Host {
    async fn invoke(&self,context:CapabilityInvocationContext,input:Value)->Result<Value,String> {
        let source=self.source(&context).await?;
        let conversation=source.conversation_id.clone();
        let owner=self.owner.as_ref();
        let action=context.action_id.as_ref().strip_prefix("plugin.development/").ok_or("Unknown module action")?;
        let pause_key = format!("plugin-await:{}", nomifun_agent_contracts::digest_bytes(source.operation_key.as_bytes()).as_ref());
        let result = match action {
            "list"=>Ok(json!({"plugins":plugin::list_plugins_owned(&self.state,owner).await.map_err(core_error)?,
                "drafts":plugin::list_drafts_owned(&self.state,owner).await.map_err(core_error)?,
                "credential_references":plugin::list_credential_references_owned(&self.state).await.map_err(core_error)?,"guide":GUIDE})),
            "open"=>{
                if let Some(id)=input.get("draft_id").and_then(Value::as_str){
                    let existing=plugin::draft_owned(&self.state,owner,id).await.map_err(core_error)?;
                    if existing.source_conversation_id.as_deref().is_some_and(|prior|prior!=conversation){
                        return Err("PLUGIN_DRAFT_SCOPE_MISMATCH: continue this working copy in its original conversation".into());
                    }
                    if existing.source_conversation_id.is_none(){
                        let request_digest=digest_payload(&input).map_err(|error|error.to_string())?;
                        nomifun_db::sqlx::query(
                            "UPDATE plugin_drafts SET source_conversation_id=?,source_message_id=?,source_operation_key=?,source_request_digest=?,revision=revision+1 WHERE owner_user_id=? AND draft_id=? AND revision=? AND source_conversation_id IS NULL"
                        ).bind(&conversation).bind(&source.message_id).bind(&source.operation_key)
                            .bind(request_digest.as_ref()).bind(owner).bind(id).bind(existing.revision as i64)
                            .execute(self.state.repository.pool()).await.map_err(|error|error.to_string())?;
                    }
                    let current=self.draft(owner,&conversation,id).await?;
                    self.changed(&conversation,&current,None);
                    return wire(plugin::draft_detail(&self.state,&current).map_err(core_error)?);
                }
                let opened=plugin::create_draft_with_source(&self.state,owner,parse(input)?,Some(source)).await.map_err(core_error)?;
                let draft=plugin::draft_owned(&self.state,owner,&opened.summary.draft_id).await.map_err(core_error)?;
                self.changed(&conversation,&draft,None); wire(opened)
            },
            "read"=>{
                let id=string(&input,"draft_id")?;
                let draft=self.draft(owner,&conversation,id).await?;
                let mut detail=wire(plugin::draft_detail(&self.state,&draft).map_err(core_error)?)?;
                let mut report=draft.verification.clone();
                report.as_object_mut().expect("verification object").remove("approval");
                detail["verification"]=report;
                detail["verification_digest"]=json!(digest_payload(&draft.verification).map_err(|error|error.to_string())?.as_ref());
                Ok(detail)
            },
            "apply"=>self.apply(owner,&conversation,&source.message_id,input).await,
            "plan"=>self.plan(owner,&conversation,&source.message_id,input).await,
            "check"=>self.check(owner,&conversation,input).await,
            "preview"=>self.preview(owner,&conversation,input).await,
            "test_action"=>self.test_action(owner,&conversation,input).await,
            "test_ui"=>plugin::run_authoring_ui_test(&self.state,owner,&conversation,context.turn_id.as_ref(),input).await.map_err(core_error),
            "install"=>self.install(owner,&conversation,context.turn_id.as_ref(),input).await,
            "inspect"=>wire(plugin::get_plugin_owned(&self.state,owner,string(&input,"plugin_id")?).await.map_err(core_error)?),
            "configure"=>{let(id,request)=plugin_request(input)?;wire(plugin::configure_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "enable"=>{let(id,request)=plugin_request(input)?;wire(plugin::set_enabled_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "export"=>{let(id,request)=plugin_request(input)?;wire(plugin::export_package_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "trash"=>{let(id,request)=plugin_request(input)?;wire(plugin::trash_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "restore"=>{let(id,request)=plugin_request(input)?;wire(plugin::restore_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "delete"=>{let(id,request)=plugin_request(input)?;wire(plugin::permanent_delete_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?)},
            "discard"=>{let(id,request)=draft_request(input)?;self.draft(owner,&conversation,&id).await?;Ok(json!({"discarded":plugin::delete_draft_owned(&self.state,owner,&id,parse(request)?).await.map_err(core_error)?}))},
            "close_preview"=>{let(id,request)=draft_request(input)?;self.draft(owner,&conversation,&id).await?;Ok(json!({"closed":plugin::close_surface_owned(&self.state,owner,parse(request)?).await.map_err(core_error)?}))},
            _=>Err("Unknown Plugin development action".into()),
        }?;
        if result["waiting_for_user"] == json!(true) {
            // Suspend at the next ordinary Runtime boundary. A model does not
            // have to close its task ledger to ask for a human code decision.
            let store = nomifun_agent_session::AgentSessionStore::from_pool(self.state.repository.pool().clone())
                .await.map_err(|error| error.to_string())?;
            store.request_native_pause(&context.principal, &context.agent_session_id, &context.turn_id,
                &pause_key, "Plugin execution requires the user's code or permission approval")
                .await.map_err(|error| error.to_string())?;
        }
        Ok(result)
    }
}

pub(super) struct SchemaResolver;
#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for SchemaResolver {
    async fn resolve(&self,capability:&ResolvedCapability,reference:&CanonicalSchemaRef)->Result<StrictJsonValue,String> {
        if capability.capability.id.as_ref()==MODULE_ID {
            if let Some(schema)=nomifun_plugin_development::resolve_schema(reference){return Ok(schema);}
        }
        Err("Unknown Plugin development schema".into())
    }
}

struct CancelOnDrop(PluginCancellation);
impl Drop for CancelOnDrop { fn drop(&mut self){self.0.cancel();} }
fn core_error(error:plugin::PluginHttpError)->String { format!("{}: {}",error.code,error.message) }
fn parse<T:for<'de>Deserialize<'de>>(value:Value)->Result<T,String>{serde_json::from_value(value).map_err(|error|format!("PLUGIN_INVALID_INPUT: {error}"))}
fn wire<T:Serialize>(value:T)->Result<Value,String>{serde_json::to_value(value).map_err(|error|error.to_string())}
fn string<'a>(value:&'a Value,key:&str)->Result<&'a str,String>{value.get(key).and_then(Value::as_str).filter(|text|!text.is_empty()).ok_or_else(||format!("PLUGIN_INVALID_INPUT: {key} is required"))}
fn revision(value:&Value)->Result<u64,String>{value.get("expected_revision").and_then(Value::as_u64).filter(|value|*value>0).ok_or("PLUGIN_INVALID_INPUT: expected_revision is required".into())}
fn scoped_request(mut input:Value,key:&str)->Result<(String,Value),String>{let id=string(&input,key)?.to_owned();input.as_object_mut().ok_or("object required")?.remove(key);Ok((id,input))}
fn draft_request(input:Value)->Result<(String,Value),String>{scoped_request(input,"draft_id")}
fn plugin_request(input:Value)->Result<(String,Value),String>{scoped_request(input,"plugin_id")}

const GUIDE:&str="Each deliverable is one standard nomifun.plugin/v1 package: nomifun.plugin.json; UI entrypoint must be ui/index.html; Service ESM entrypoint must be service/main.mjs; at least one is required. Use inline JSON Schema for actions and config. Manifest fields: schema,id,version,name,description,hostApi,entrypoints,actions,bindings,dataVersion,migrations,configSchema,secrets,permissions. Host creates a valid starting manifest; read it before editing. Call plan before apply: extract every requested output and required feature, link exact cases, and retain the accepted plan during repairs. Persistent data needs test_ui reopen assertions or test_action restart:true reads. All drafts for one request must declare the same output list with distinct output keys. Use only listed enabled credential_references; ask for missing information through the current conversation before planning. UI SDK: window.nomi.storage.kv.get/set/delete/compareAndSwap; storage.db.query/execute/batch; storage.files.read/write/list/delete; cache; actions.invoke; config.get. UI storage.kv.get returns the stored JSON value directly, or null when absent. storage.kv.set returns {revision}; compareAndSwap returns {applied,revision}. Service exports async activate(ctx) returning an object with async invoke(action,input) and optional deactivate. ctx exposes storage/cache/config/secrets/host/actions/signal. No fake APIs, external CDN scripts, runtime selectors or mobile layouts. Use test_ui to perform actual click/fill/text/count/reopen requirement cases on the real conversation preview; headless uses test_action on the real temporary Service. Structure/startup alone is not delivery. Install the verified revision, then inspect the installed plugin. Current desktop.files.open has no available Host owner; do not generate an implementation based on that name.";
