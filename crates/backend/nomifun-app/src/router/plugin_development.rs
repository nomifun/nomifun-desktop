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
    Router::new().route("/api/plugins/authoring/preflight",post(preflight))
        .route("/api/agent-sessions/{id}/plugin-continuation",post(continue_with_input))
        .with_state(state.clone()).merge(super::plugin_authoring_sessions::routes(state))
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
    if live.metadata.purpose != nomifun_agent_contracts::SessionPurpose::PluginAuthoring {
        return Err(AppError::Forbidden("Historical ordinary-session drafts are read-only in the plugin workbench".into()).into());
    }
    let binding:AgentBindingValueDto=serde_json::from_value(serde_json::to_value(&live.agent_binding)?)?;
    let (_,_,snapshot)=state.control_plane.saved_binding_artifacts(&user.id.to_string().into(),&binding).await?;
    super::plugin_authoring_sessions::validate_scope(&snapshot)?;
    let input=super::nomi_core_session::bounded_turn_input(json!({"content":body.input.content,"files":body.input.files}))?;
    store.append_paused_native_input(&principal,&session,&body.request,
        StrictJsonValue(super::nomi_core_session::canonical_turn_input(&input)),
        &["PLUGIN_VERIFICATION_REQUIRED","PLUGIN_DELIVERY_REQUIRED","PLUGIN_CURRENT_CONVERSATION_PENDING","EXECUTION_USER_REQUESTED"],
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
    let registered=catalog.modules.iter().any(|module|module.module.id==MODULE_ID);
    let capabilities=match &request.selection {
        AgentSelection::Preset{preset_id}=>{
            match state.control_plane.editor(&owner,preset_id,None).await {
                Ok(editor) if editor.revision.is_some()=>editor.revision.expect("checked").document.enabled_capabilities,
                Err(error) if error.status() != axum::http::StatusCode::NOT_FOUND => return Err(error.into()),
                _=>return Ok(Json(ApiResponse::ok(json!({"status":"configure_agent","reason":"AGENT_DEFAULT_UNAVAILABLE","selection":request.selection,"owner_user_id":owner.as_ref()})))),
            }
        },
        AgentSelection::Template{template_key}=>{
            let library=state.control_plane.library(&owner).await?;
            let key=serde_json::from_value::<OfficialPresetKeyDto>(json!(template_key)).ok();
            match library.official_templates.into_iter().find(|template|Some(template.template_key)==key) {
                Some(template)=>template.seed.enabled_capabilities,
                None=>return Ok(Json(ApiResponse::ok(json!({"status":"configure_agent","reason":"AGENT_DEFAULT_UNAVAILABLE","selection":request.selection,"owner_user_id":owner.as_ref()})))),
            }
        },
    };
    let selected=capabilities.iter().find(|selection|selection.capability.id==MODULE_ID);
    let missing=nomifun_plugin_development::CREATE_ACTIONS.iter().filter(|action|
        !selected.is_some_and(|selection|selection.action_allowlist.contains(**action)))
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

    /// An open identical to one that already created a draft for this accepted
    /// request is a replay once the request has every draft it needs: the
    /// admitted plugin_delivery.expected_count, or a larger planned output list.
    /// A different open form (template, plugin_id) is a distinct request and is
    /// never merged into an existing draft.
    async fn replayed_open(
        &self, owner: &str, context: &CapabilityInvocationContext,
        source: &plugin::PluginDraftSource, request: &CreatePluginDraftRequest,
    ) -> Result<Option<PluginDraftRecord>, String> {
        let digest = digest_payload(request).map_err(|error| error.to_string())?;
        let siblings: Vec<PluginDraftRecord> = self.state.repository.list_drafts(owner).await
            .map_err(|error| error.to_string())?.into_iter()
            .filter(|record| record.source_conversation_id.as_deref() == Some(source.conversation_id.as_str())
                && record.source_message_id.as_deref() == Some(source.message_id.as_str()))
            .collect();
        let Some(recent) = identical_open_sibling(&siblings, digest.as_ref(), &source.operation_key)
        else { return Ok(None); };
        let planned = siblings.iter().map(|record|
            record.verification["plan"]["outputs"].as_array().map_or(0, Vec::len))
            .max().unwrap_or(0);
        let required = self.admitted_output_count(context, source).await?.max(planned).max(1);
        Ok((siblings.len() >= required).then(|| recent.clone()))
    }

    /// The accepted message's plugin_delivery.expected_count (1 without one),
    /// read from the canonical source input of this running Turn.
    async fn admitted_output_count(
        &self, context: &CapabilityInvocationContext, source: &plugin::PluginDraftSource,
    ) -> Result<usize, String> {
        let store = nomifun_agent_session::AgentSessionStore::from_pool(self.state.repository.pool().clone())
            .await.map_err(|error| error.to_string())?;
        let facts = store.native_recovery_facts(&context.agent_session_id, &context.turn_id)
            .await.map_err(|error| error.to_string())?;
        let payload = facts.event_payloads.get(&source.message_id)
            .ok_or("PLUGIN_SOURCE_MISSING: the admitted request payload is unavailable")?;
        match payload.get("plugin_delivery").filter(|value| !value.is_null()) {
            Some(value) => Ok(usize::from(
                serde_json::from_value::<PluginDeliveryRequirement>(value.clone())
                    .map_err(|error| error.to_string())?.expected_count)),
            None => Ok(1),
        }
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
        plugin::require_authoring_revision(&draft,revision(&fields)?).map_err(core_error)?;
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
        draft.verification=json!({"edit_revision":draft.revision+1,"task_message_id":message,"plan":plan});
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
        plugin::require_authoring_revision(&draft, request.expected_revision).map_err(core_error)?;
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
        draft.verification = json!({"edit_revision":draft.revision+1,"task_message_id":message,"plan":draft.verification["plan"]});
        draft.updated_at_ms = nomifun_common::now_ms();
        plugin::revoke_draft_surfaces(&self.state, &draft.draft_id).await.map_err(core_error)?;
        replacement.publish().map_err(|error| error.to_string())?;
        let updated = match self.state.repository.update_draft(&draft, draft.revision).await {
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
        plugin::require_authoring_revision(&draft, revision).map_err(core_error)?;
        let revision=draft.revision;
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
            "diagnostics":[{"code":"PLUGIN_SHAPE_MISMATCH","message":format!(
                "The package must implement its planned UI/Service shape: the plan requires '{}' but the package is '{}'",
                kind.unwrap_or("unspecified"),actual)}]})); }
        let mut planned_ids:Vec<&str>=plan["cases"].as_object().into_iter().flatten()
            .filter(|(_,case)|case["kind"]=="action")
            .filter_map(|(_,case)|case["action"].as_str()).collect();
        planned_ids.extend(plan["current_conversation_case"]["action"].as_str());
        let mut missing:Vec<&str>=planned_ids.iter().copied()
            .filter(|id|!artifact.manifest.actions.contains_key(*id)).collect();
        missing.sort_unstable(); missing.dedup();
        if !missing.is_empty() {
            let declared:Vec<&str>=artifact.manifest.actions.keys().map(String::as_str).collect();
            return Ok(json!({"passed":false,"stage":"requirements","draft_id":id,"revision":revision,
                "diagnostics":[{"code":"PLUGIN_PLANNED_ACTION_MISSING","message":format!(
                    "The plan references action '{}' which the manifest does not declare; declared actions: {}",
                    missing.join("', '"),if declared.is_empty(){"none".into()}else{declared.join(", ")})}]}));
        }
        if let Some(case)=plan.get("current_conversation_case") {
            if !artifact.manifest.bindings.iter().any(|binding|
                binding.point==nomifun_agent_contracts::PluginBindingPoint::AgentTool && binding.action==case["action"]) {
                let bound:Vec<&str>=artifact.manifest.bindings.iter()
                    .filter(|binding|binding.point==nomifun_agent_contracts::PluginBindingPoint::AgentTool)
                    .map(|binding|binding.action.as_str()).collect();
                return Ok(json!({"passed":false,"stage":"requirements","draft_id":id,"revision":revision,
                    "diagnostics":[{"code":"PLUGIN_CURRENT_AGENT_TOOL_REQUIRED","message":format!(
                        "Current conversation use requires a declared agent.tool binding for the planned Action '{}'; agent.tool is currently bound to: {}",
                        case["action"].as_str().unwrap_or("unspecified"),
                        if bound.is_empty(){"none".into()}else{bound.join(", ")})}]}));
            }
        }
        draft.verification = json!({
            "edit_revision":draft.verification["edit_revision"].as_u64().unwrap_or(draft.revision),
            "artifact_digest":artifact.artifact_digest.as_ref(),"structure_passed":true,
            "has_ui":artifact.manifest.has_ui(),"has_service":artifact.manifest.has_service(),
            "cases":{},"runtime_ready":false,
            "source_message_id":draft.source_message_id,
            "task_message_id":draft.verification["task_message_id"],
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
        plugin::require_authoring_revision(&draft, revision).map_err(core_error)?;
        let revision=draft.revision;
        fields["expected_revision"]=json!(revision);
        let files = self.state.drafts.freeze(owner, &draft.draft_id).map_err(|error| error.to_string())?;
        let artifact = self.state.artifacts.inspect_files(&files, &NeverCancel).map_err(|error| error.to_string())?;
        let installed = match &draft.plugin_id {
            Some(id) => self.state.repository.inventory(owner, id).await.map_err(|error| error.to_string())?,
            None => None,
        };
        let config = fields.get("config").cloned()
            .or_else(|| installed.as_ref().map(|inventory| inventory.plugin.config.clone()))
            .unwrap_or_else(|| json!({}));
        let permissions = fields.get("permissions").cloned()
            .unwrap_or_else(|| json!(artifact.manifest.permissions));
        let credentials = fields.get("credential_bindings").cloned().unwrap_or_else(|| json!(
            installed.as_ref().map(|inventory| inventory.credential_bindings.iter()
                .filter(|(slot, _)| artifact.manifest.secrets.contains(slot))
                .map(|(slot, reference)| (slot.clone(), reference.clone())).collect::<BTreeMap<_, _>>())
                .unwrap_or_default()
        ));
        let execution = json!({"config":config,"permissions":permissions,"credential_bindings":credentials});
        let verification_context=super::plugin_authoring::verification_context(self.state.repository.pool(),&credentials).await.map_err(core_error)?;
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
        plugin::require_authoring_revision(&draft, expected_revision).map_err(core_error)?;
        let artifact = self.state.artifacts.inspect_files(&self.state.drafts.freeze(owner,&draft.draft_id).map_err(|error|error.to_string())?, &NeverCancel).map_err(|error|error.to_string())?;
        if draft.verification.get("runtime_ready") != Some(&json!(true))
            || draft.verification.get("artifact_digest").and_then(Value::as_str) != Some(artifact.artifact_digest.as_ref())
        { return Err("PLUGIN_PREVIEW_REQUIRED: run the exact package first".into()); }
        if super::plugin_authoring::installed_since_preview(&draft.verification) {
            return Err(super::plugin_authoring::INSTALLED_PREVIEW_CLOSED.into());
        }
        // Install (or any newer preview/apply) revokes the draft preview
        // Surface; only a still-registered preview session may dispatch. A
        // failed call after install would otherwise be recorded as a failed
        // case and permanently fail verification.
        let surface_id=draft.verification["surface"]["surface_session_id"].as_str().unwrap_or_default();
        let preview_live=self.state.surfaces.lock().await.get(surface_id).is_some_and(|session|
            session.is_preview && session.draft_id.as_ref()==Some(&draft.draft_id));
        if !preview_live {
            return Err("PLUGIN_PREVIEW_REQUIRED: the preview is not running (install or a newer edit closes it); run check and preview again before test_action. After install, do not use test_action; the installed tool is used through current_conversation_case.".into());
        }
        let action = string(&fields,"action")?.to_owned();
        let case = string(&fields,"case_name")?.to_owned();
        let restart=fields["restart"]==json!(true);
        let mut fields=fields;
        let planned_case=draft.verification["plan"]["cases"][case.as_str()].clone();
        let planned=!planned_case.is_null();
        decode_encoded_case_fields(&mut fields,&planned_case,
            artifact.manifest.actions.get(&action).map(|declared|(&declared.input.0,&declared.output.0)));
        let mut oracle = json!({"kind":"action","action":action,"input":fields["input"],"expected_output":fields["expected_output"]});
        if restart { oracle["restart"]=json!(true); }
        draft = super::plugin_authoring::begin_authoring_case(&self.state,&draft,&case,oracle)
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
            &plugin_action_id(&nomifun_agent_contracts::PluginId::from(id.clone()), &action),
            StrictJsonValue(fields["input"].clone()),
            PluginDispatchOptions { expected_artifact_digest:Some(artifact.artifact_digest.clone()), cancellation, call_chain:Vec::new() },
        ).await;
        let (output, error) = match outcome {
            Ok(output) => (output.0, None),
            Err(error) => (Value::Null, Some(error.to_string())),
        };
        let expected = &fields["expected_output"];
        let passed = error.is_none() && &output == expected;
        draft.verification["cases"][&case] = json!({"kind":"action","action":action,"input":fields["input"],"expected_output":expected,"actual_output":output,"passed":passed,"error":error,
            "persistence_checked":restart && passed,"state":if passed{"passed"}else{"failed"}});
        draft.updated_at_ms=nomifun_common::now_ms();
        let updated=self.state.repository.update_draft(&draft,execution_revision).await.map_err(|error|error.to_string())?;
        if let Some(guard)=&mut restart_guard { guard.retain(); }
        self.changed(conversation,&updated,None);
        Ok(json!({"draft_id":id,"case_name":case,"planned":planned,"revision":updated.revision,"passed":passed,"actual_output":output,"error":error,
            "verification_digest":digest_payload(&updated.verification).map_err(|error|error.to_string())?.as_ref()}))
    }

    async fn install(&self, owner:&str, conversation:&str, turn:&str, input:Value)->Result<Value,String> {
        let (id,mut request)=draft_request(input)?;
        let draft=self.draft(owner,conversation,&id).await?;
        plugin::require_authoring_revision(&draft,revision(&request)?).map_err(core_error)?;
        request["expected_revision"]=json!(draft.revision);
        let artifact=self.state.artifacts.inspect_files(&self.state.drafts.freeze(owner,&draft.draft_id).map_err(|error|error.to_string())?,&NeverCancel).map_err(|error|error.to_string())?;
        let verification_digest=string(&request,"verification_digest")?.to_owned();
        if digest_payload(&draft.verification).map_err(|error|error.to_string())?.as_ref()!=verification_digest
            || draft.verification["artifact_digest"].as_str()!=Some(artifact.artifact_digest.as_ref())
            || draft.verification["runtime_ready"]!=json!(true)
        { return Err("PLUGIN_VERIFICATION_REQUIRED: the exact package has not passed its required runtime/UI checks".into()); }
        if !nomifun_plugin_development::plan_evidence_complete(&draft.verification) {
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
        let PluginInstallOutcomeDto::Installed { plugin }=&outcome.result;
        {
            if !plugin.summary.enabled
                || plugin.summary.active.artifact_digest!=artifact.artifact_digest.as_ref()
                || plugin.summary.last_error.is_some()
            {return Err("PLUGIN_DELIVERY_FAILED: saved plugin is unavailable or differs from verified bytes".into());}
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
            for (case_name,case) in draft.verification["plan"]["cases"].as_object().into_iter().flatten() {
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
        if nomifun_plugin_development::CREATE_ACTIONS.contains(&context.action_id.as_ref()) {
            let store = nomifun_agent_session::AgentSessionStore::from_pool(self.state.repository.pool().clone())
                .await.map_err(|error| error.to_string())?;
            let session = store.get_live_session(&context.agent_session_id).await.map_err(|error| error.to_string())?;
            if session.metadata.purpose != nomifun_agent_contracts::SessionPurpose::PluginAuthoring {
                return Err("PLUGIN_AUTHORING_WORKSPACE_REQUIRED: create or edit plugins in the Plugins & Small Apps workbench; this ordinary conversation cannot start plugin authoring".into());
            }
        }
        let source=self.source(&context).await?;
        let conversation=source.conversation_id.clone();
        let owner=self.owner.as_ref();
        let action=context.action_id.as_ref().strip_prefix("plugin.development/").ok_or("Unknown module action")?;
        let result = match action {
            "list"=>{
                // Drafts bound to another conversation cannot be continued
                // here; unscoped drafts stay listed because open adopts them.
                let all=plugin::list_drafts_owned(&self.state,owner).await.map_err(core_error)?.drafts;
                let other=all.iter().filter(|draft|draft.source_conversation_id.as_deref()
                    .is_some_and(|source|source!=conversation.as_str())).count();
                let drafts=all.into_iter().filter(|draft|!draft.source_conversation_id.as_deref()
                    .is_some_and(|source|source!=conversation.as_str())).collect::<Vec<_>>();
                Ok(json!({"plugins":plugin::list_plugins_owned(&self.state,owner).await.map_err(core_error)?,
                    "drafts":drafts,"other_conversation_drafts":other,
                    "credential_references":plugin::list_credential_references_owned(&self.state).await.map_err(core_error)?,"guide":GUIDE}))
            },
            "open"=>{
                if let Some(id)=input.get("draft_id").and_then(Value::as_str){
                    let existing=plugin::draft_owned(&self.state,owner,id).await.map_err(|error|
                        if error.status==axum::http::StatusCode::NOT_FOUND {
                            format!("PLUGIN_DRAFT_NOT_FOUND: draft {id} does not exist. To start a new plugin call open with an empty arguments object {{}}; draft_id only reopens an existing draft returned by list.")
                        } else { core_error(error) })?;
                    if existing.source_conversation_id.as_deref().is_some_and(|prior|prior!=conversation){
                        return Err("PLUGIN_DRAFT_SCOPE_MISMATCH: continue this working copy in its original conversation".into());
                    }
                    if existing.source_message_id.is_none(){
                        let request_digest=digest_payload(&input).map_err(|error|error.to_string())?;
                        nomifun_db::sqlx::query(
                            "UPDATE plugin_drafts SET source_conversation_id=?,source_message_id=?,source_operation_key=?,source_request_digest=?,revision=revision+1 WHERE owner_user_id=? AND draft_id=? AND revision=? AND source_message_id IS NULL AND (source_conversation_id IS NULL OR source_conversation_id=?)"
                        ).bind(&conversation).bind(&source.message_id).bind(&source.operation_key)
                            .bind(request_digest.as_ref()).bind(owner).bind(id).bind(existing.revision as i64).bind(&conversation)
                            .execute(self.state.repository.pool()).await.map_err(|error|error.to_string())?;
                    }
                    let current=self.draft(owner,&conversation,id).await?;
                    self.changed(&conversation,&current,None);
                    return wire(plugin::draft_detail(&self.state,&current).map_err(core_error)?);
                }
                let plugin_id=input.get("plugin_id").and_then(Value::as_str).map(str::to_owned);
                let request:CreatePluginDraftRequest=parse(input)?;
                if let Some(recent)=self.replayed_open(owner,&context,&source,&request).await? {
                    self.changed(&conversation,&recent,None);
                    let mut detail=wire(plugin::draft_detail(&self.state,&recent).map_err(core_error)?)?;
                    detail["notice"]=json!(format!("This request already has draft {} from the same open request; it was reopened instead of creating another. Call open again only for an additional output declared in the plan.",recent.draft_id.as_ref()));
                    return Ok(detail);
                }
                let opened=plugin::create_draft_with_source(&self.state,owner,request,Some(source)).await.map_err(|error|
                    match (&plugin_id,error.status==axum::http::StatusCode::NOT_FOUND) {
                        (Some(id),true)=>format!("PLUGIN_NOT_FOUND: no installed plugin {id}. Omit plugin_id to create a new plugin; plugin_id only edits an installed plugin returned by list."),
                        _=>core_error(error),
                    })?;
                let draft=plugin::draft_owned(&self.state,owner,&opened.summary.draft_id).await.map_err(core_error)?;
                self.changed(&conversation,&draft,None); wire(opened)
            },
            "read"=>{
                let id=string(&input,"draft_id")?;
                let draft=self.draft(owner,&conversation,id).await?;
                let mut detail=wire(plugin::draft_detail(&self.state,&draft).map_err(core_error)?)?;
                let report=draft.verification.clone();
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
            "inspect"=>{
                let plugin_id=string(&input,"plugin_id")?;
                wire(plugin::get_plugin_owned(&self.state,owner,plugin_id).await.map_err(|error|
                    if error.status==axum::http::StatusCode::NOT_FOUND {
                        format!("PLUGIN_NOT_FOUND: no installed plugin {plugin_id}; installed plugin ids come from list or from a successful install result.")
                    } else { core_error(error) })?)
            },
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

/// The draft an `open` replays into, before the request's output count is
/// considered: the newest sibling created by an identical open request.
/// The same tool call replayed (equal source_operation_key) is answered by
/// create_draft_with_source's own idempotent replay, so it selects nothing.
fn identical_open_sibling<'a>(
    siblings: &'a [PluginDraftRecord], request_digest: &str, operation_key: &str,
) -> Option<&'a PluginDraftRecord> {
    if siblings.iter().any(|record| record.source_operation_key.as_deref() == Some(operation_key)) { return None; }
    siblings.iter()
        .filter(|record| record.source_request_digest.as_deref() == Some(request_digest))
        .max_by_key(|record| (record.updated_at_ms, record.created_at_ms))
}

/// A JSON value supplied as an encoded string is decoded only when that keeps
/// the oracle exact: a planned case must parse to the planned value; a
/// supplementary case (absent from the plan) uses the declared action schema
/// when it types the field as an object or array. Anything else stays verbatim.
fn decode_encoded_case_fields(fields:&mut Value, planned_case:&Value, declared:Option<(&Value,&Value)>) {
    if !planned_case.is_null() {
        for key in ["input","expected_output"] {
            if fields[key].is_string() && !planned_case[key].is_string()
                && let Ok(parsed)=serde_json::from_str::<Value>(fields[key].as_str().unwrap_or_default())
                && parsed==planned_case[key]
            {
                fields[key]=parsed;
            }
        }
        return;
    }
    let Some((input,output))=declared else { return; };
    for (key,schema) in [("input",input),("expected_output",output)] {
        if let Some(text)=fields[key].as_str()
            && let Some(kind)=schema["type"].as_str()
            && matches!(kind,"object"|"array")
            && let Ok(parsed)=serde_json::from_str::<Value>(text)
            && (kind=="object" && parsed.is_object() || kind=="array" && parsed.is_array())
        {
            fields[key]=parsed;
        }
    }
}

fn parse<T:for<'de>Deserialize<'de>>(value:Value)->Result<T,String>{serde_json::from_value(value).map_err(|error|format!("PLUGIN_INVALID_INPUT: {error}"))}
fn wire<T:Serialize>(value:T)->Result<Value,String>{serde_json::to_value(value).map_err(|error|error.to_string())}
fn string<'a>(value:&'a Value,key:&str)->Result<&'a str,String>{value.get(key).and_then(Value::as_str).filter(|text|!text.is_empty()).ok_or_else(||format!("PLUGIN_INVALID_INPUT: {key} is required"))}
fn revision(value:&Value)->Result<u64,String>{value.get("expected_revision").and_then(Value::as_u64).filter(|value|*value>0).ok_or("PLUGIN_INVALID_INPUT: expected_revision is required".into())}
fn scoped_request(mut input:Value,key:&str)->Result<(String,Value),String>{let id=string(&input,key)?.to_owned();input.as_object_mut().ok_or("object required")?.remove(key);Ok((id,input))}
fn draft_request(input:Value)->Result<(String,Value),String>{scoped_request(input,"draft_id")}
fn plugin_request(input:Value)->Result<(String,Value),String>{scoped_request(input,"plugin_id")}

const GUIDE:&str="Each deliverable is one standard nomifun.plugin/v1 package: nomifun.plugin.json; UI entrypoint must be ui/index.html; Service ESM entrypoint must be service/main.mjs; at least one is required. Use inline JSON Schema for actions and config. Manifest fields: schema,id,version,name,description,hostApi,entrypoints,actions,bindings,dataVersion,migrations,configSchema,secrets,permissions. Host creates a valid starting manifest; read it before editing. Call plan before apply: extract every requested output and required feature, link exact cases, and retain the accepted plan during repairs. Persistent data needs test_ui reopen assertions or test_action restart:true reads. All drafts for one request must declare the same output list with distinct output keys. Use only listed enabled credential_references; ask for missing information through the current conversation before planning. UI SDK: window.nomi.storage.kv.get/set/delete/compareAndSwap; storage.db.query/execute/batch; storage.files.read/write/list/delete; cache; actions.invoke; config.get. UI storage.kv.get returns the stored JSON value directly, or null when absent. storage.kv.set returns {revision}; compareAndSwap returns {applied,revision}. Service exports async activate(ctx) returning an object with async invoke(action,input) and optional deactivate. ctx exposes storage/cache/config/secrets/host/actions/signal. No fake APIs, external CDN scripts, runtime selectors or mobile layouts. Use test_ui to perform actual click/fill/text/count/reopen requirement cases on the real conversation preview; headless uses test_action on the real temporary Service. Structure/startup alone is not delivery. Install the verified revision, then inspect the installed plugin. Current desktop.files.open has no available Host owner; do not generate an implementation based on that name. UI sandbox rules: ui/index.html runs in a sandboxed iframe with scripts only. localStorage, sessionStorage, indexedDB and cookies throw, alert/confirm/prompt are blocked, and <form> submission is blocked (no submit event): bind click handlers to buttons and keydown handlers for Enter instead. Persist UI data only with await window.nomi.storage.kv.get(key) and await window.nomi.storage.kv.set(key, value); the SDK is injected automatically, so do not add a script tag for it. Give every element that a test case uses a stable id or data-testid. Business cases must operate the actual user control: for a checkbox, click the input itself, not its li/row/container as a proxy. Bind the checkbox change event to update application state and await storage; stopping click propagation does not save a checkbox change. Assert completion using count with selector #todos input[type=checkbox]:checked, then reopen and repeat that assertion to prove the actual checked state persisted. Example self-contained completion case: fill #new-todo with milk; click #add; click #todos input[type=checkbox]; count #todos input[type=checkbox]:checked equals 1; reopen; count #todos input[type=checkbox]:checked equals 1. Row text, styling, or a container click alone does not prove the user-facing checkbox works. Only plan.cases are required for delivery; extra test_ui/test_action cases are diagnostic probes and can fail or change without changing the plan. Async UI lifecycle: disable controls during storage initialization and while saving; enable them only after awaited storage and rendering finish. Treat a failed initial storage.kv.get as unknown existing state. Keep mutation controls disabled, show the real error and offer retry or reopen; allow writes only after a successful read. Handle real SDK failures without adding simulated-failure switches or test-only UI branches. Keep initialization and mutations serialized so a late load cannot overwrite newer edits. All add, toggle and delete handlers that read and rewrite the same state must share one serial queue or one consistent busy guard. Set busy before the first await and disable all related controls, not only the clicked button. Compute next state without mutating current state, await persistence, then commit and render it. If saving fails, show the error, retain the original state and restore controls to that state; do not catch the error and still render(next) as if it saved. Clear busy and re-enable controls in finally after the operation settles. All add, toggle, delete and row handlers must use the same mutation entry point. Correct busy pattern: if (busy) return; busy = true; disableAll(); try { await mutation(); } catch(e) { restoreOldUi(); showError(e); } finally { busy = false; enableAll(); }. mutation is your awaitable save-and-commit operation, and the other functions are your own UI helpers. With a Promise queue, assign each new task back to chain; merely awaiting an unchanged resolved Promise does not serialize mutations. A UI handler may absorb an error after fully restoring state and displaying it; it does not need to rethrow. Example initialization: const add=document.querySelector('#add'); add.disabled=true; const todos=(await window.nomi.storage.kv.get('todos'))??[]; render(todos); add.disabled=false; render is your own UI function. Show a visible error when initialization or saving fails. The test runner waits for standard disabled/aria-disabled/inert/hidden/readonly states before interactions. test_ui semantics: click calls element.click(); fill sets the value and dispatches input and change events; text compares the element's trimmed textContent exactly; count is the number of document.querySelectorAll(selector) matches; reopen reloads the page with the same plugin storage. Every test_ui case restarts the preview with fresh storage (for an update, a fresh copy of the installed data), and that restart also resets the Service storage used by test_action. Make each UI case self-contained by creating the data it asserts on. test_action cases share one preview storage in the order you run them, so for a plugin with both UI and Service run the test_action cases first, keep each write and its restart:true read consecutive, and run the test_ui cases afterwards. Manifest reference: actions is an object keyed by action id; each action requires exactly name (string), description (string), input (JSON Schema), output (JSON Schema) and effect (\"read\", \"write\" or \"external\"). bindings is an array of {\"point\":\"agent.tool\",\"action\":\"<action id>\"} entries; agent.tool exposes the action to Agents as a tool (other points: agent.context, agent.before_model, agent.before_tool, desktop.command, desktop.event, automation.action). entrypoints holds \"ui\": \"ui/index.html\" and/or \"service\": \"service/main.mjs\" with optional \"serviceMode\": \"onDemand\" or \"continuous\"; a headless plugin has only the service entrypoint. Service invoke(action, input) receives the action id and validated input and must return a value matching that action's output schema; test_action compares expected_output with the returned value exactly. Action ids are lowercase machine identifiers: they start with a-z and contain only a-z, 0-9, '_' or '-' (at most 96 characters), for example trim_and_uppercase. Plan cases, current_conversation_case and bindings must use exactly the manifest action ids. Add current_conversation_case only when the user explicitly asks to use the new tool in this same conversation; an agent.tool binding already makes an installed action available to Agents. With current_conversation_case, the task pauses once after install while the tool is attached; when it resumes, call that tool with exactly the planned input. install closes the preview and itself checks the installed plugin and its UI: do not call test_action or test_ui after install. When the user asks to use the new tool in this conversation, put current_conversation_case {action, input, expected_output} (exactly these three fields) in the plan instead of extra test cases.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoded JSON strings decode only where the oracle stays exact: a
    /// planned case must match the planned value, a supplementary case must
    /// type the field object or array in the declared action schema.
    #[test]
    fn encoded_case_fields_decode_only_when_the_oracle_stays_exact() {
        let object_schema=Some((&json!({"type":"object"}),&json!({"type":"object"})));
        // Planned case: an equal encoded string decodes; a different one
        // stays verbatim so the strict comparison rejects it below.
        let planned=json!({"kind":"action","input":{"text":"  Mixed Case  "},"expected_output":{"text":"MIXED CASE"}});
        let mut fields=json!({"input":"{\"text\":\"  Mixed Case  \"}","expected_output":"{\"text\":\"MIXED CASE\"}"});
        decode_encoded_case_fields(&mut fields,&planned,object_schema);
        assert_eq!(fields,json!({"input":{"text":"  Mixed Case  "},"expected_output":{"text":"MIXED CASE"}}));
        let mut fields=json!({"input":"{\"text\":\"other\"}","expected_output":{"text":"MIXED CASE"}});
        decode_encoded_case_fields(&mut fields,&planned,object_schema);
        assert_eq!(fields,json!({"input":"{\"text\":\"other\"}","expected_output":{"text":"MIXED CASE"}}));
        // A planned field that is itself a string is never decoded, even when the
        // declared schema types it as an object: the planned oracle is that string.
        let planned=json!({"kind":"action","input":"{\"text\":\"abc\"}","expected_output":"{}"});
        let mut fields=json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"});
        decode_encoded_case_fields(&mut fields,&planned,object_schema);
        assert_eq!(fields,json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"}));
        // Supplementary case: the declared object/array schema decides.
        let mut fields=json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"});
        decode_encoded_case_fields(&mut fields,&Value::Null,object_schema);
        assert_eq!(fields,json!({"input":{"text":"abc"},"expected_output":{}}));
        let string_schema=Some((&json!({"type":"string"}),&json!({"type":"string"})));
        let mut fields=json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"});
        decode_encoded_case_fields(&mut fields,&Value::Null,string_schema);
        assert_eq!(fields,json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"}));
        let mut fields=json!({"input":"[\"a\"]","expected_output":"{}"});
        decode_encoded_case_fields(&mut fields,&Value::Null,object_schema);
        assert_eq!(fields,json!({"input":"[\"a\"]","expected_output":{}}),"an encoded array is not the declared object");
        let mut fields=json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"});
        decode_encoded_case_fields(&mut fields,&Value::Null,None);
        assert_eq!(fields,json!({"input":"{\"text\":\"abc\"}","expected_output":"{}"}));
    }

    fn draft_record(digest:&str,operation_key:&str,updated_at_ms:i64)->PluginDraftRecord {
        PluginDraftRecord {
            owner_user_id:"owner".into(),draft_id:uuid::Uuid::now_v7().to_string().into(),
            revision:1,plugin_id:None,base_revision:None,name:"Working copy".into(),
            workspace_path:String::new(),
            source_conversation_id:Some("conversation".into()),source_message_id:Some("message".into()),
            source_operation_key:Some(operation_key.to_owned()),
            source_request_digest:Some(digest.to_owned()),
            verification:json!({}),
            status:nomifun_plugin_platform::PluginDraftStatus::Ready,last_error:None,
            created_at_ms:updated_at_ms,updated_at_ms,
        }
    }

    /// An identical open replays into the newest identical sibling; the same
    /// tool call replayed (equal operation key) defers to
    /// create_draft_with_source's own idempotent replay.
    #[test]
    fn identical_open_sibling_prefers_exact_replays_and_identical_requests() {
        let a=draft_record("d1","k1",1);
        let b=draft_record("d1","k2",2); let b_id=b.draft_id.clone();
        let t=draft_record("d2","k3",3); let t_id=t.draft_id.clone();
        let siblings=[a,b,t];
        assert_eq!(identical_open_sibling(&siblings,"d1","k9").map(|draft|&draft.draft_id),Some(&b_id));
        assert_eq!(identical_open_sibling(&siblings,"d2","k9").map(|draft|&draft.draft_id),Some(&t_id));
        assert!(identical_open_sibling(&siblings,"d3","k9").is_none());
        assert!(identical_open_sibling(&siblings,"d1","k1").is_none(),
            "the same tool call replayed is create_draft_with_source's idempotent replay");
    }

    /// Model-facing conflicts say to reload via read; terse HTTP conflicts
    /// keep the shared GUI message.
    #[test]
    fn authoring_revision_conflicts_tell_the_model_to_read_first() {
        let draft=draft_record("d1","k1",1);
        let error=plugin::require_authoring_revision(&draft,7).unwrap_err();
        assert!(error.message.contains("expected_revision 7"),"{error}");
        assert!(error.message.contains("Call read"),"{error}");
        assert!(!error.message.contains("revision is"),
            "the authoring message does not quote the actual revision: {error}");
        let error=plugin::require_draft_revision(&draft,7).unwrap_err();
        assert_eq!(error.message,"Draft revision changed");
    }
}
