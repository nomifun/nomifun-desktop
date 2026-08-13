//! `/api/customer-service/*` route handlers (REST 面, per Interfaces spec).

use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::routing::get;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_db::models::{
    CsAgentRow, CsChannelBindingRow, CsDialogueRow, CsMessageRow, CsNoteRow,
};
use serde::Deserialize;

use crate::service::{CreateCsAgentInput, CreateCsNoteInput, CustomerServiceService, UpdateCsAgentInput};

/// Router state for the customer-service domain.
#[derive(Clone)]
pub struct CustomerServiceRouterState {
    pub service: Arc<CustomerServiceService>,
    /// Channel repository used ONLY to validate that binding targets name
    /// live bot rows (binding 的 plugin id 存在性由 route 层查渠道仓储).
    pub channel_repo: Arc<dyn nomifun_db::IChannelRepository>,
}

pub fn customer_service_routes(state: CustomerServiceRouterState) -> Router {
    Router::new()
        .route("/api/customer-service/agents", get(list_agents).post(create_agent))
        .route(
            "/api/customer-service/agents/{cs_agent_id}",
            get(get_agent).patch(update_agent).delete(delete_agent),
        )
        .route(
            "/api/customer-service/agents/{cs_agent_id}/bindings",
            get(list_bindings).put(replace_bindings),
        )
        .route("/api/customer-service/notes", get(list_notes).post(create_note))
        .route(
            "/api/customer-service/notes/{cs_note_id}",
            axum::routing::patch(update_note).delete(delete_note),
        )
        .route("/api/customer-service/dialogues", get(list_dialogues))
        .route(
            "/api/customer-service/dialogues/{cs_dialogue_id}/messages",
            get(list_dialogue_messages),
        )
        .with_state(state)
}

// ── agents ──────────────────────────────────────────────────────────

async fn list_agents(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<CsAgentRow>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list_agents().await?)))
}

async fn create_agent(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCsAgentInput>, JsonRejection>,
) -> Result<Json<ApiResponse<CsAgentRow>>, AppError> {
    let Json(input) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.create_agent(input).await?)))
}

async fn get_agent(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_agent_id): Path<String>,
) -> Result<Json<ApiResponse<CsAgentRow>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.get_agent(&cs_agent_id).await?)))
}

async fn update_agent(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_agent_id): Path<String>,
    body: Result<Json<UpdateCsAgentInput>, JsonRejection>,
) -> Result<Json<ApiResponse<CsAgentRow>>, AppError> {
    let Json(input) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state.service.update_agent(&cs_agent_id, input).await?,
    )))
}

async fn delete_agent(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_agent_id): Path<String>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    state.service.delete_agent(&cs_agent_id).await?;
    Ok(Json(ApiResponse::ok(true)))
}

// ── bindings ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ReplaceBindingsRequest {
    channel_plugin_ids: Vec<String>,
}

async fn list_bindings(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_agent_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<CsChannelBindingRow>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list_bindings(&cs_agent_id).await?)))
}

async fn replace_bindings(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_agent_id): Path<String>,
    body: Result<Json<ReplaceBindingsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<CsChannelBindingRow>>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    // Every listed plugin must name a live bot row owned by the
    // customer-service domain — companion-pool bots are never bindable here
    // (channel ownership is domain-exclusive since migration 020).
    for plugin_id in &req.channel_plugin_ids {
        let plugin = state
            .channel_repo
            .get_plugin(plugin_id)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .ok_or_else(|| {
                AppError::BadRequest(format!("channel plugin '{plugin_id}' not found"))
            })?;
        if plugin.owner_domain != "customer_service" {
            return Err(AppError::BadRequest(format!(
                "channel bot {plugin_id} belongs to the companion domain; \
                 create a customer-service bot instead"
            )));
        }
    }
    Ok(Json(ApiResponse::ok(
        state
            .service
            .replace_bindings(&cs_agent_id, req.channel_plugin_ids)
            .await?,
    )))
}

// ── notes ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListNotesQuery {
    #[serde(default)]
    cs_agent_id: Option<String>,
}

async fn list_notes(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListNotesQuery>,
) -> Result<Json<ApiResponse<Vec<CsNoteRow>>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.service.list_notes(query.cs_agent_id.as_deref()).await?,
    )))
}

async fn create_note(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateCsNoteInput>, JsonRejection>,
) -> Result<Json<ApiResponse<CsNoteRow>>, AppError> {
    let Json(input) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.create_note(input).await?)))
}

#[derive(Debug, Deserialize)]
struct UpdateNoteRequest {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

async fn update_note(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_note_id): Path<String>,
    body: Result<Json<UpdateNoteRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CsNoteRow>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state
            .service
            .update_note(&cs_note_id, req.kind.as_deref(), req.content.as_deref(), req.enabled)
            .await?,
    )))
}

async fn delete_note(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_note_id): Path<String>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    state.service.delete_note(&cs_note_id).await?;
    Ok(Json(ApiResponse::ok(true)))
}

// ── dialogues (monitoring read surface) ─────────────────────────────

#[derive(Debug, Deserialize)]
struct ListDialoguesQuery {
    cs_agent_id: String,
}

async fn list_dialogues(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListDialoguesQuery>,
) -> Result<Json<ApiResponse<Vec<CsDialogueRow>>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.service.repo().list_dialogues(&query.cs_agent_id).await?,
    )))
}

async fn list_dialogue_messages(
    State(state): State<CustomerServiceRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(cs_dialogue_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<CsMessageRow>>>, AppError> {
    Ok(Json(ApiResponse::ok(
        state.service.repo().list_messages(&cs_dialogue_id).await?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use nomifun_db::models::NewChannelPluginRow;
    use nomifun_db::{
        IChannelRepository, SqliteChannelRepository, SqliteCustomerServiceRepository,
    };

    async fn setup() -> (nomifun_db::Database, CustomerServiceRouterState) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let service = Arc::new(CustomerServiceService::new(Arc::new(
            SqliteCustomerServiceRepository::new(db.pool().clone()),
        )));
        let channel_repo: Arc<dyn IChannelRepository> =
            Arc::new(SqliteChannelRepository::new(db.pool().clone()));
        (db, CustomerServiceRouterState { service, channel_repo })
    }

    fn user() -> CurrentUser {
        CurrentUser {
            id: nomifun_common::UserId::new(),
            username: "tester".into(),
        }
    }

    async fn seed_bot(
        state: &CustomerServiceRouterState,
        name: &str,
        owner_domain: &str,
    ) -> String {
        let now = nomifun_common::now_ms();
        state
            .channel_repo
            .create_plugin(&NewChannelPluginRow {
                r#type: "telegram".into(),
                name: name.into(),
                enabled: false,
                config: "enc".into(),
                status: None,
                last_connected: None,
                companion_id: None,
                bot_key: None,
                owner_domain: owner_domain.into(),
                group_access_mode: nomifun_api_types::GroupAccessMode::default_for_owner_domain(
                    owner_domain,
                )
                .as_str()
                .into(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap()
            .channel_plugin_id
    }

    async fn seed_agent(state: &CustomerServiceRouterState) -> String {
        state
            .service
            .create_agent(CreateCsAgentInput {
                name: "客服A".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .cs_agent_id
    }

    async fn put_bindings(
        state: &CustomerServiceRouterState,
        cs_agent_id: &str,
        ids: Vec<String>,
    ) -> Result<Json<ApiResponse<Vec<CsChannelBindingRow>>>, AppError> {
        replace_bindings(
            State(state.clone()),
            Extension(user()),
            Path(cs_agent_id.to_owned()),
            Ok(Json(ReplaceBindingsRequest { channel_plugin_ids: ids })),
        )
        .await
    }

    #[tokio::test]
    async fn replace_bindings_rejects_companion_domain_bot() {
        let (_db, state) = setup().await;
        let agent = seed_agent(&state).await;
        let companion_bot = seed_bot(&state, "Companion Bot", "companion").await;

        let err = put_bindings(&state, &agent, vec![companion_bot.clone()])
            .await
            .unwrap_err();
        match err {
            AppError::BadRequest(message) => {
                assert!(
                    message.contains(&format!(
                        "channel bot {companion_bot} belongs to the companion domain"
                    )) && message.contains("create a customer-service bot instead"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
        assert!(
            state.service.list_bindings(&agent).await.unwrap().is_empty(),
            "a rejected PUT must not write any binding"
        );
    }

    #[tokio::test]
    async fn replace_bindings_rejects_missing_bot() {
        let (_db, state) = setup().await;
        let agent = seed_agent(&state).await;
        let missing = nomifun_common::ChannelPluginId::new().into_string();

        let err = put_bindings(&state, &agent, vec![missing]).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(message) if message.contains("not found")));
    }

    #[tokio::test]
    async fn replace_bindings_accepts_cs_domain_bot_and_same_domain_rebind() {
        let (_db, state) = setup().await;
        let agent_a = seed_agent(&state).await;
        let agent_b = seed_agent(&state).await;
        let cs_bot = seed_bot(&state, "CS Bot", "customer_service").await;
        assert_eq!(
            state
                .channel_repo
                .get_plugin(&cs_bot)
                .await
                .unwrap()
                .unwrap()
                .group_access_mode,
            "all_members",
            "new customer-service bots default open to group members"
        );

        let bound = put_bindings(&state, &agent_a, vec![cs_bot.clone()]).await.unwrap();
        assert_eq!(bound.0.data.as_ref().unwrap().len(), 1);

        // Same-domain re-bind moves the bot from A to B.
        let _ = put_bindings(&state, &agent_b, vec![cs_bot.clone()]).await.unwrap();
        assert!(state.service.list_bindings(&agent_a).await.unwrap().is_empty());
        assert_eq!(
            state
                .service
                .binding_for_plugin(&cs_bot)
                .await
                .unwrap()
                .as_deref(),
            Some(agent_b.as_str())
        );
    }
}
