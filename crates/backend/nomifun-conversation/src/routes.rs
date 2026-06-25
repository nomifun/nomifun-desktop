use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};

use nomifun_api_types::{
    ActiveCountResponse, ApiResponse, ApprovalCheckQuery, ApprovalCheckResponse, CloneConversationRequest,
    ConfirmRequest, ConfirmationListResponse, ConversationArtifactListResponse, ConversationArtifactResponse,
    ConversationListResponse, ConversationResponse, CreateConversationRequest, ListConversationsQuery,
    ListMessagesQuery, MessageListResponse, MessageResponse, MessageSearchResponse, SearchMessagesQuery,
    SendMessageRequest, SendMessageResponse, UpdateConversationArtifactRequest, UpdateConversationRequest,
};
use nomifun_auth::{CurrentUser, LocalTrusted};
use nomifun_common::AppError;

use crate::state::ConversationRouterState;

/// Build the conversation router (CRUD + message flow + confirmation + extended operations).
///
/// All routes require authentication (applied by the caller).
pub fn conversation_routes(state: ConversationRouterState) -> Router {
    Router::new()
        .route("/api/conversations", post(create).get(list))
        .route("/api/conversations/{id}", get(get_one).patch(update).delete(delete_one))
        .route("/api/conversations/{id}/reset", post(reset))
        .route("/api/conversations/{id}/associated", get(associated))
        .route("/api/conversations/{id}/messages", get(list_msg).post(send_msg))
        .route("/api/conversations/{id}/messages/{messageId}", get(get_msg))
        .route("/api/conversations/{id}/artifacts", get(list_artifacts))
        .route("/api/conversations/{id}/artifacts/{artifactId}", patch(update_artifact))
        .route("/api/conversations/{id}/cancel", post(cancel))
        .route("/api/conversations/{id}/steer", post(steer))
        .route("/api/conversations/{id}/warmup", post(warmup))
        // Confirmation system
        .route("/api/conversations/{id}/confirmations", get(list_confirmations))
        .route("/api/conversations/{id}/confirmations/{callId}/confirm", post(confirm))
        .route("/api/conversations/{id}/approvals/check", get(check_approval))
        .route("/api/conversations/active-count", get(active_count))
        .route("/api/conversations/clone", post(clone))
        .route("/api/messages/search", get(search_messages))
        .with_state(state)
}

// ── Handlers ───────────────────────────────────────────────────────

/// `extra.desktopGateway` entitles a session to the Desktop Gateway MCP (full
/// desktop control: conversations, cron, memory, requirements). Only backend
/// code paths (channel master-agent sessions, companion companion threads) may set
/// it — strip both spellings from any extra JSON arriving over HTTP so a
/// client cannot self-authorize a session.
fn strip_desktop_gateway_flag(extra: &mut serde_json::Value) {
    if let Some(map) = extra.as_object_mut() {
        map.remove("desktopGateway");
        map.remove("desktop_gateway");
    }
}

/// Grant the Desktop Gateway to a session the BACKEND has decided is entitled.
/// Called after [`strip_desktop_gateway_flag`] (clients cannot self-authorize;
/// the backend re-grants). Ensures `extra` is an object first.
fn grant_desktop_gateway(extra: &mut serde_json::Value) {
    if !extra.is_object() {
        *extra = serde_json::json!({});
    }
    if let Some(map) = extra.as_object_mut() {
        map.insert("desktopGateway".to_owned(), serde_json::Value::Bool(true));
    }
}

async fn create(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    // Present only for locally-trusted requests (the desktop webview / NoAuth),
    // NOT for remote LAN browser sessions. See `grant` rationale below.
    local: Option<Extension<LocalTrusted>>,
    body: Result<Json<CreateConversationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<ConversationResponse>>), AppError> {
    let Json(mut req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    strip_desktop_gateway_flag(&mut req.extra);
    // The desktop is the owner's own machine, so EVERY locally-trusted session is
    // entitled to the Desktop Gateway by default — any conversation becomes a
    // semantic super-gateway over the whole platform (the product's "0-code, any
    // session does everything" goal). Granted by the backend AFTER the strip, so a
    // client still cannot self-authorize. Remote LAN browser sessions get no
    // `LocalTrusted` marker and are NOT granted here; companion/channel sessions
    // are granted on their own (service-direct) paths. What a granted session may
    // actually DO is still governed by the gateway's danger-tier × surface gate.
    if local.is_some() {
        grant_desktop_gateway(&mut req.extra);
    }
    let conversation = state.service.create(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(conversation))))
}

async fn list(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Query(query): Query<ListConversationsQuery>,
) -> Result<Json<ApiResponse<ConversationListResponse>>, AppError> {
    // 普通会话列表保留 companion 行(前端侧边栏自行过滤),不在此处排除。
    let result = state.service.list(&user.id, query, false).await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn clone(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CloneConversationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<ConversationResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let conversation = state.service.clone_create(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(conversation))))
}

async fn get_one(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ConversationResponse>>, AppError> {
    let conversation = state.service.get(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(conversation)))
}

async fn update(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<UpdateConversationRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ConversationResponse>>, AppError> {
    let Json(mut req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(extra) = req.extra.as_mut() {
        // `update` merges extra keys, so a client could otherwise smuggle the
        // gateway flag into an existing conversation.
        strip_desktop_gateway_flag(extra);
    }
    let conversation = state.service.update(&user.id, &id, req, &state.task_manager).await?;
    Ok(Json(ApiResponse::ok(conversation)))
}

async fn delete_one(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.delete(&user.id, &id).await?;
    Ok(Json(ApiResponse::success()))
}

async fn reset(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.reset(&user.id, &id).await?;
    Ok(Json(ApiResponse::success()))
}

async fn associated(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Vec<ConversationResponse>>>, AppError> {
    let items = state.service.list_associated(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(items)))
}

async fn list_msg(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<ApiResponse<MessageListResponse>>, AppError> {
    let result = state.service.list_messages(&user.id, &id, query).await?;
    Ok(Json(ApiResponse::ok(result)))
}

#[derive(serde::Deserialize)]
struct MessagePathParams {
    id: String,
    #[serde(rename = "messageId")]
    message_id: String,
}

async fn get_msg(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(params): Path<MessagePathParams>,
) -> Result<Json<ApiResponse<MessageResponse>>, AppError> {
    let result = state
        .service
        .get_message(&user.id, &params.id, &params.message_id)
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn send_msg(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<SendMessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<SendMessageResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let msg_id = state
        .service
        .send_message(&user.id, &id, req, &state.task_manager)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::ok(SendMessageResponse { msg_id })),
    ))
}

async fn steer(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<SendMessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<SendMessageResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let msg_id = state
        .service
        .steer_message(&user.id, &id, req, &state.task_manager)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::ok(SendMessageResponse { msg_id })),
    ))
}

async fn list_artifacts(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ConversationArtifactListResponse>>, AppError> {
    let result = state.service.list_artifacts(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(result)))
}

#[derive(serde::Deserialize)]
struct ArtifactPathParams {
    id: String,
    #[serde(rename = "artifactId")]
    artifact_id: String,
}

async fn update_artifact(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(params): Path<ArtifactPathParams>,
    body: Result<Json<UpdateConversationArtifactRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ConversationArtifactResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let artifact_id: i64 = params
        .artifact_id
        .parse()
        .map_err(|_| AppError::BadRequest(format!("invalid artifact id: {}", params.artifact_id)))?;
    let artifact = state
        .service
        .update_artifact(&user.id, &params.id, artifact_id, req)
        .await?;
    Ok(Json(ApiResponse::ok(artifact)))
}

async fn cancel(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.cancel(&user.id, &id, &state.task_manager).await?;
    Ok(Json(ApiResponse::success()))
}

async fn warmup(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.warmup(&user.id, &id, &state.task_manager).await?;
    Ok(Json(ApiResponse::success()))
}

async fn search_messages(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<ApiResponse<MessageSearchResponse>>, AppError> {
    let result = state.service.search_messages(&user.id, query).await?;
    Ok(Json(ApiResponse::ok(result)))
}

// ── Confirmation handlers ─────────────────────────────────────────

async fn list_confirmations(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ConfirmationListResponse>>, AppError> {
    let items = state
        .service
        .list_confirmations(&user.id, &id, &state.task_manager)
        .await?;
    Ok(Json(ApiResponse::ok(items)))
}

#[derive(serde::Deserialize)]
struct ConfirmPathParams {
    id: String,
    #[serde(rename = "callId")]
    call_id: String,
}

async fn confirm(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(params): Path<ConfirmPathParams>,
    body: Result<Json<ConfirmRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .service
        .confirm(&user.id, &params.id, &params.call_id, req, &state.task_manager)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn check_approval(
    State(state): State<ConversationRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<ApprovalCheckQuery>,
) -> Result<Json<ApiResponse<ApprovalCheckResponse>>, AppError> {
    if query.action.trim().is_empty() {
        return Err(AppError::BadRequest("action must not be empty".into()));
    }

    let result = state
        .service
        .check_approval(
            &user.id,
            &id,
            &query.action,
            query.command_type.as_deref(),
            &state.task_manager,
        )
        .await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn active_count(
    State(state): State<ConversationRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<ActiveCountResponse>>, AppError> {
    let count = state.task_manager.active_count();
    Ok(Json(ApiResponse::ok(ActiveCountResponse { count })))
}

#[cfg(test)]
mod tests {
    use super::strip_desktop_gateway_flag;
    use serde_json::json;

    #[test]
    fn strips_both_spellings_of_the_gateway_flag() {
        let mut extra = json!({
            "desktopGateway": true,
            "desktop_gateway": true,
            "companionSession": true,
            "backend": "claude",
        });
        strip_desktop_gateway_flag(&mut extra);
        assert!(extra.get("desktopGateway").is_none());
        assert!(extra.get("desktop_gateway").is_none());
        // Unrelated keys survive.
        assert_eq!(extra["companionSession"], json!(true));
        assert_eq!(extra["backend"], json!("claude"));
    }

    #[test]
    fn strip_is_a_noop_on_non_objects() {
        let mut extra = json!("not an object");
        strip_desktop_gateway_flag(&mut extra);
        assert_eq!(extra, json!("not an object"));
    }
}
