//! Canonical AgentSession adapter and authenticated HTTP surface for IDMM.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use nomifun_agent_contracts::{AgentSessionId, PrincipalRef};
use nomifun_auth::CurrentUser;
use nomifun_api_types::{
    ApiResponse, IdmmBypassModelRef, IdmmConfig, IdmmScanScope, IdmmState,
    SendMessageRequest,
};
use nomifun_common::AppError;
use nomifun_idmm::{
    IdmmBypassModelPort, IdmmService, IdmmSessionObservation, IdmmSessionPort,
    ObservedMessage, ObservedMessageRole, ObservedTurn, ObservedTurnState,
};
use serde_json::Value;
use uuid::Uuid;

use super::nomi_core_session::NomiCoreSessionOwner;

#[derive(Clone)]
pub(crate) struct IdmmRouterState {
    pub service: Arc<IdmmService>,
    session_owner: Arc<NomiCoreSessionOwner>,
}

impl IdmmRouterState {
    pub(crate) fn new(
        service: Arc<IdmmService>,
        session_owner: Arc<NomiCoreSessionOwner>,
    ) -> Self {
        Self {
            service,
            session_owner,
        }
    }

    async fn authorize(
        &self,
        owner: &CurrentUser,
        session_id: &str,
    ) -> Result<AgentSessionId, AppError> {
        let uuid = Uuid::parse_str(session_id)
            .map_err(|_| AppError::NotFound("AgentSession not found".into()))?;
        if uuid.get_version_num() != 7 || uuid.hyphenated().to_string() != session_id {
            return Err(AppError::NotFound("AgentSession not found".into()));
        }
        let session_id = AgentSessionId::from(session_id.to_owned());
        let session = self
            .session_owner
            .canonical()
            .store()
            .get_live_session(&session_id)
            .await
            .map_err(|error| {
                if error.code() == Some("SESSION_NOT_FOUND")
                    || error.is_session_deleted()
                {
                    AppError::NotFound("AgentSession not found".into())
                } else {
                    AppError::Internal(format!("read IDMM AgentSession: {error}"))
                }
            })?;
        if session.owner_ref
            != (PrincipalRef {
                principal_kind: "user".into(),
                principal_id: owner.id.as_str().to_owned(),
            })
        {
            return Err(AppError::Forbidden("AgentSession owner mismatch".into()));
        }
        Ok(session_id)
    }
}

pub(crate) fn idmm_routes(state: IdmmRouterState) -> Router {
    Router::new()
        .route(
            "/api/agent-sessions/{agent_session_id}/idmm",
            get(get_state).put(put_config),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/idmm/evaluate",
            post(evaluate_now),
        )
        .with_state(state)
}

async fn get_state(
    State(state): State<IdmmRouterState>,
    Extension(owner): Extension<CurrentUser>,
    Path(session_id): Path<String>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    state.authorize(&owner, &session_id).await?;
    Ok(Json(ApiResponse::ok(state.service.state(&session_id).await?)))
}

async fn put_config(
    State(state): State<IdmmRouterState>,
    Extension(owner): Extension<CurrentUser>,
    Path(session_id): Path<String>,
    Json(config): Json<IdmmConfig>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    state.authorize(&owner, &session_id).await?;
    let next = state.service.set_config(&session_id, config).await?;
    if let Err(error) = state.authorize(&owner, &session_id).await {
        let _ = state.service.remove(&session_id).await;
        return Err(error);
    }
    Ok(Json(ApiResponse::ok(next)))
}

async fn evaluate_now(
    State(state): State<IdmmRouterState>,
    Extension(owner): Extension<CurrentUser>,
    Path(session_id): Path<String>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    state.authorize(&owner, &session_id).await?;
    Ok(Json(ApiResponse::ok(
        state.service.evaluate_now(&session_id).await?,
    )))
}

pub(crate) fn build_idmm_service(
    owner_id: Arc<str>,
    pool: nomifun_db::SqlitePool,
    session_owner: Arc<NomiCoreSessionOwner>,
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    workspace: PathBuf,
    provider_lifecycle: nomifun_common::SharedProviderLifecycleBarrier,
) -> Arc<IdmmService> {
    Arc::new(IdmmService::new(
        owner_id,
        pool.clone(),
        Arc::new(CanonicalIdmmSessionPort {
            session_owner: Arc::downgrade(&session_owner),
            pool,
        }),
        Arc::new(ModelInvokeBypassPort {
            model_invoke,
            workspace,
        }),
        provider_lifecycle,
    ))
}

struct CanonicalIdmmSessionPort {
    session_owner: std::sync::Weak<NomiCoreSessionOwner>,
    pool: nomifun_db::SqlitePool,
}

impl CanonicalIdmmSessionPort {
    fn owner(&self) -> Result<Arc<NomiCoreSessionOwner>, AppError> {
        self.session_owner
            .upgrade()
            .ok_or_else(|| AppError::Conflict("AgentSession owner has shut down".into()))
    }
}

#[async_trait]
impl IdmmSessionPort for CanonicalIdmmSessionPort {
    async fn observe(
        &self,
        owner_id: &str,
        session_id: &str,
        scope: IdmmScanScope,
        max_messages: u32,
        max_chars: u32,
    ) -> Result<Option<IdmmSessionObservation>, AppError> {
        let session_id_typed = AgentSessionId::from(session_id.to_owned());
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner_id.to_owned(),
        };
        let owner = self.owner()?;
        let session = match owner
            .canonical()
            .store()
            .get_live_session(&session_id_typed)
            .await
        {
            Ok(session) => session,
            Err(error)
                if error.code() == Some("SESSION_NOT_FOUND")
                    || error.is_session_deleted() =>
            {
                return Ok(None)
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read IDMM AgentSession: {error}"
                )))
            }
        };
        if session.owner_ref != principal {
            return Err(AppError::Forbidden("AgentSession owner mismatch".into()));
        }
        let head = owner
            .canonical()
            .store()
            .head(&session_id_typed)
            .await
            .map_err(|error| AppError::Internal(format!("read IDMM Session head: {error}")))?;
        let turn: Option<(String, String, Option<String>, Option<String>)> =
            nomifun_db::sqlx::query_as(
                "SELECT turn.operation_id, turn.state, turn.error_json, event.inline_json \
                 FROM agent_turns turn \
                 LEFT JOIN agent_events event ON event.event_id = turn.started_event_id \
                 WHERE turn.session_id = ? ORDER BY turn.accepted_at DESC LIMIT 1",
            )
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| AppError::Internal(format!("read IDMM latest turn: {error}")))?;
        let latest_turn = turn.and_then(|(operation_id, state, error_json, started_json)| {
            let state = match state.as_str() {
                "accepted" | "running" => ObservedTurnState::Running,
                "completed" => ObservedTurnState::Completed,
                "failed" | "interrupted" => ObservedTurnState::Failed,
                "cancelled" => ObservedTurnState::Cancelled,
                _ => return None,
            };
            let error = error_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| {
                    value
                        .get("message")
                        .or_else(|| value.get("error"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            let origin = started_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .and_then(|value| {
                    value
                        .get("origin")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            Some(ObservedTurn {
                operation_id,
                state,
                error,
                origin,
            })
        });
        let limit = match scope {
            IdmmScanScope::LastTurn => 4_i64,
            IdmmScanScope::LastMessages => i64::from(max_messages.clamp(1, 100)),
            IdmmScanScope::FullSession => 500_i64,
        };
        let rows: Vec<(i64, String, String)> = nomifun_db::sqlx::query_as(
            "SELECT last_seq, semantic_digest, projection_json FROM agent_messages \
             WHERE session_id = ? AND presentation_intent = 'message' \
             ORDER BY last_seq DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Internal(format!("read IDMM message window: {error}")))?;
        let mut messages = Vec::new();
        let mut used_chars = 0_usize;
        let max_chars = max_chars as usize;
        for (sequence, fingerprint, projection_json) in rows {
            let Ok(projection) = serde_json::from_str::<Value>(&projection_json) else {
                continue;
            };
            let state = projection
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !matches!(state, "accepted" | "completed") {
                continue;
            }
            let content = projection
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            let remaining = max_chars.saturating_sub(used_chars);
            if remaining == 0 {
                break;
            }
            let bounded = take_chars(content, remaining);
            used_chars = used_chars.saturating_add(bounded.chars().count());
            messages.push(ObservedMessage {
                fingerprint,
                sequence: u64::try_from(sequence).unwrap_or_default(),
                role: if state == "accepted" {
                    ObservedMessageRole::User
                } else {
                    ObservedMessageRole::Assistant
                },
                content: bounded,
            });
        }
        messages.reverse();
        Ok(Some(IdmmSessionObservation {
            agent_session_id: session_id.to_owned(),
            active_turn_id: head.active_turn_id,
            latest_turn,
            messages,
        }))
    }

    async fn deliver(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let delivery = self
            .owner()?
            .send_session_message_idempotent(
                owner_id,
                session_id,
                idempotency_key,
                SendMessageRequest {
                    content: content.to_owned(),
                    files: Vec::new(),
                    inject_skills: Vec::new(),
                    hidden: false,
                    origin: Some("idmm".into()),
                    channel_platform: None,
                },
            )
            .await?;
        if delivery.completed && delivery.result_ok == Some(false) {
            return Err(AppError::BadGateway(
                delivery
                    .result_error
                    .unwrap_or_else(|| "IDMM recovery turn failed".into()),
            ));
        }
        Ok(())
    }

    async fn cancel_and_deliver(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let owner = self.owner()?;
        owner
            .cancel_session_for_idmm(owner_id, session_id)
            .await?;
        let typed = AgentSessionId::from(session_id.to_owned());
        for _ in 0..20 {
            let head = owner
                .canonical()
                .store()
                .head(&typed)
                .await
                .map_err(|error| {
                    AppError::Internal(format!("wait for IDMM cancellation: {error}"))
                })?;
            if head.active_turn_id.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.deliver(owner_id, session_id, idempotency_key, content)
            .await
    }
}

struct ModelInvokeBypassPort {
    model_invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    workspace: PathBuf,
}

#[async_trait]
impl IdmmBypassModelPort for ModelInvokeBypassPort {
    async fn validate(&self, model: &IdmmBypassModelRef) -> Result<(), AppError> {
        let (provider_id, model_name) = bypass_model_parts(model)?;
        nomifun_ai_agent::factory::provider_config::resolve_provider_config(
            self.model_invoke.as_ref(),
            provider_id,
            model_name,
            &self.workspace,
        )
        .await
        .map(|_| ())
    }

    async fn complete(
        &self,
        model: &IdmmBypassModelRef,
        system: &str,
        prompt: &str,
        max_output_bytes: usize,
    ) -> Result<String, AppError> {
        let (provider_id, model_name) = bypass_model_parts(model)?;
        let config = nomifun_ai_agent::factory::provider_config::resolve_provider_config(
            self.model_invoke.as_ref(),
            provider_id,
            model_name,
            &self.workspace,
        )
        .await?;
        nomifun_ai_agent::factory::provider_config::one_shot_completion_bounded(
            &config,
            system,
            vec![nomifun_ai_agent::factory::provider_config::user_message(prompt)],
            600,
            max_output_bytes,
        )
        .await
    }
}

fn bypass_model_parts(model: &IdmmBypassModelRef) -> Result<(&str, &str), AppError> {
    let provider_id = model.provider_id.as_deref().ok_or_else(|| {
            AppError::BadRequest("IDMM bypass provider is not configured".into())
        })?;
    let model_name = model.model.as_deref().ok_or_else(|| {
            AppError::BadRequest("IDMM bypass model is not configured".into())
        })?;
    Ok((provider_id, model_name))
}

fn take_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let boundary = value
        .char_indices()
        .rev()
        .nth(max_chars.saturating_sub(1))
        .map_or(value.len(), |(index, _)| index);
    value[boundary..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::take_chars;

    #[test]
    fn bounded_observation_keeps_the_newest_unicode_suffix() {
        assert_eq!(take_chars("前文甲乙问题？", 4), "乙问题？");
    }
}
