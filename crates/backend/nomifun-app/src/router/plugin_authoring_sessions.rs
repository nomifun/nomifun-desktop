//! Plugin-workbench presentation of canonical Agent Sessions.
//! No private transcript, execution owner, or product binding ledger.
use axum::{extract::{Extension, Path, State}, routing::{get, post}, Json, Router};
use nomifun_agent_contracts::{AgentSessionId, PrincipalRef, SessionPurpose, UserId};
use nomifun_api_types::{AgentChatModelSelectionDto, ApiResponse, SessionReasoningEffortDto};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

use super::{nomi_core_session::{NomiCoreAgentApiState, NomiCoreApiError}, plugin_development::AgentSelection};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    selection: AgentSelection,
    #[serde(default)]
    model: Option<AgentChatModelSelectionDto>,
    idempotency_key: String,
    #[serde(default)]
    reasoning_effort: Option<SessionReasoningEffortDto>,
    #[serde(default)]
    plugin_id: Option<String>,
    #[serde(default)]
    expected_plugin_revision: Option<u64>,
    #[serde(default)]
    draft_id: Option<String>,
    #[serde(default)]
    template: Option<String>,
}

#[derive(Serialize)]
struct SessionReference {
    agent_session_id: String,
}

pub(super) fn routes(state: NomiCoreAgentApiState) -> Router {
    Router::new()
        .route("/api/plugins/authoring/sessions", post(create).get(list))
        .route("/api/plugins/authoring/sessions/{id}", get(open))
        .with_state(state)
}

fn principal(user: &CurrentUser) -> PrincipalRef {
    PrincipalRef { principal_kind: "user".into(), principal_id: user.id.to_string().into() }
}

/// Purpose selects the product surface. The selected Agent retains its existing
/// capabilities and resources; plugin creation requires its own module actions.
pub(super) fn validate_scope(
    snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
) -> Result<(), AppError> {
    if !snapshot.content.contributions().any(|module|
        module.capability.id.as_ref() == nomifun_plugin_development::MODULE_ID
            && nomifun_plugin_development::CREATE_ACTIONS.iter().all(|id|
                module.action_allowlist.iter().any(|granted| granted.as_ref() == *id)))
    {
        return Err(AppError::Conflict("Enable plugin creation on the selected Agent before starting the workbench".into()));
    }
    Ok(())
}

async fn reference(state: &NomiCoreAgentApiState, user: &CurrentUser, id: &str)
    -> Result<SessionReference, NomiCoreApiError>
{
    nomifun_common::validate_uuidv7(id).map_err(|e| AppError::BadRequest(e.to_string()))?;
    let observation = state.session_owner.canonical().get(&principal(user), &AgentSessionId::from(id)).await?;
    if observation.session.metadata.purpose == SessionPurpose::PluginAuthoring {
        return Ok(SessionReference { agent_session_id: id.into() });
    }
    Err(AppError::NotFound("Plugin authoring Session does not exist".into()).into())
}

async fn open(State(state): State<NomiCoreAgentApiState>, Extension(user): Extension<CurrentUser>, Path(id): Path<String>)
    -> Result<Json<ApiResponse<SessionReference>>, NomiCoreApiError>
{
    Ok(Json(ApiResponse::ok(reference(&state, &user, &id).await?)))
}

async fn list(State(state): State<NomiCoreAgentApiState>, Extension(user): Extension<CurrentUser>)
    -> Result<Json<ApiResponse<serde_json::Value>>, NomiCoreApiError>
{
    let mut cursor = None;
    let mut sessions = Vec::new();
    loop {
        let page = state.session_owner.canonical().store().list_live_sessions_for_purpose(&principal(&user), cursor.as_deref(), 100,
            SessionPurpose::PluginAuthoring).await
            .map_err(|e| AppError::Conflict(e.to_string()))?;
        for item in page.items {
            if item.session.metadata.purpose == SessionPurpose::PluginAuthoring {
                if let Some(projection) = state.session_owner.canonical_conversation_projection(&user.id.to_string(), &item.session.agent_session_id).await? {
                    sessions.push(projection);
                }
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() { break; }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({"sessions": sessions}))))
}

async fn create(State(state): State<NomiCoreAgentApiState>, Extension(user): Extension<CurrentUser>, Json(body): Json<CreateRequest>)
    -> Result<Json<ApiResponse<SessionReference>>, NomiCoreApiError>
{
    nomifun_common::validate_uuidv7(&body.idempotency_key).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if body.draft_id.is_some() && (body.plugin_id.is_some() || body.template.is_some())
        || body.template.as_deref().is_some_and(|value| value != "agent.before_tool")
        || body.plugin_id.is_some() && body.template.is_some()
        || body.plugin_id.is_none() && body.expected_plugin_revision.is_some()
    { return Err(AppError::BadRequest("Choose one plugin authoring source".into()).into()); }
    let pool = state.session_owner.domain_pool();
    let mut detach_source = None;
    if let Some(id) = &body.draft_id {
        nomifun_common::validate_uuidv7(id).map_err(|e| AppError::BadRequest(e.to_string()))?;
        let source: Option<(Option<String>, i64)> = nomifun_db::sqlx::query_as(
            "SELECT source_conversation_id,revision FROM plugin_drafts WHERE owner_user_id=? AND draft_id=?")
            .bind(user.id.to_string()).bind(id).fetch_optional(pool).await.map_err(|e| AppError::Internal(e.to_string()))?;
        let (source, revision) = source.ok_or_else(|| AppError::NotFound("Plugin draft does not exist".into()))?;
        if let Some(source) = source {
            // Verify the current-generation Session through its canonical owner.
            // Only files are carried forward; a Conversation's Agent history is
            // never attached to a newly created PluginAuthoring Session.
            let observation = state.session_owner.canonical().get(&principal(&user), &AgentSessionId::from(source.clone())).await?;
            if observation.session.metadata.purpose == SessionPurpose::PluginAuthoring {
                return Ok(Json(ApiResponse::ok(reference(&state, &user, &source).await?)));
            }
            detach_source = Some((id.clone(), Some(source), revision));
        } else {
            detach_source = Some((id.clone(), None, revision));
        }
    }
    if let Some(id) = &body.plugin_id {
        nomifun_common::validate_uuidv7(id).map_err(|e| AppError::BadRequest(e.to_string()))?;
        let revision: Option<i64> = nomifun_db::sqlx::query_scalar(
            "SELECT revision FROM plugins WHERE owner_user_id=? AND plugin_id=? AND trashed_at_ms IS NULL")
            .bind(user.id.to_string()).bind(id).fetch_optional(pool).await.map_err(|e| AppError::Internal(e.to_string()))?;
        let revision = revision.ok_or_else(|| AppError::NotFound("Installed plugin does not exist".into()))?;
        if body.expected_plugin_revision != Some(revision as u64) {
            return Err(AppError::Conflict("Plugin revision changed".into()).into());
        }
    }
    let owner = UserId::from(user.id.to_string());
    let binding = match body.selection {
        AgentSelection::Preset { preset_id } => state.control_plane
            .resolve_plugin_authoring_session_binding(&owner, &preset_id, body.model.as_ref()).await?,
        AgentSelection::Template { template_key } => state.control_plane
            .resolve_plugin_authoring_template_binding(&owner, &template_key, body.model.as_ref()).await?,
    };
    let (_, _, snapshot) = state.control_plane.saved_binding_artifacts(&owner, &binding).await?;
    validate_scope(&snapshot)?;
    let id = super::nomi_core_session::create_plugin_authoring_session(&state, &owner, binding,
        body.reasoning_effort, &format!("plugin-authoring:{}", body.idempotency_key)).await?;
    if let Some((draft, source, revision)) = detach_source {
        let updated = nomifun_db::sqlx::query(
            "UPDATE plugin_drafts SET source_conversation_id=?,source_message_id=NULL,source_operation_key=NULL,source_request_digest=NULL,verification_json=json_object('edit_revision',revision+1),revision=revision+1,updated_at_ms=MAX(updated_at_ms,?) WHERE owner_user_id=? AND draft_id=? AND revision=? AND source_conversation_id IS ?")
            .bind(&id).bind(nomifun_common::now_ms()).bind(user.id.to_string()).bind(draft).bind(revision).bind(source)
            .execute(pool).await.map_err(|e| AppError::Internal(e.to_string()))?;
        if updated.rows_affected() != 1 {
            return Err(AppError::Conflict("The saved working copy changed while opening its new workbench".into()).into());
        }
    }
    Ok(Json(ApiResponse::ok(SessionReference { agent_session_id: id })))
}
