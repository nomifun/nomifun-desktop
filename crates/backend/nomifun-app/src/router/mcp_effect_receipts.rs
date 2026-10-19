//! Canonical MCP effect receipts shared by every Runtime consumer.

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, EventId,
    EventProducerId, IdempotencyKey, OperationId, PrincipalRef, ResolvedSnapshotRef,
    SessionEventPayloadRef, StrictJsonValue,
};
use nomifun_agent_session::{
    AgentSessionStore, EffectEventRequest, EffectStrategy, EffectTerminalState,
};
use nomifun_common::AppError;
use nomifun_db::SqlitePool;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const MCP_SERVER_RESOURCE_EFFECT: &str = "mcp.server";

#[derive(Clone)]
pub(crate) struct McpEffectReceipts {
    pool: SqlitePool,
}

pub(crate) struct McpEffectReceipt {
    request: EffectEventRequest,
    capability: String,
}

fn unavailable() -> AppError {
    AppError::Conflict("MCP durable effect authority is unavailable or fenced".into())
}

impl McpEffectReceipts {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn store(&self) -> Result<AgentSessionStore, AppError> {
        AgentSessionStore::from_pool(self.pool.clone())
            .await
            .map_err(|_| unavailable())
    }

    async fn owned_session(
        &self,
        store: &AgentSessionStore,
        user: &str,
        session: &str,
    ) -> Result<AgentSessionId, AppError> {
        let session_id = AgentSessionId::from(session.to_owned());
        let row = store
            .get_live_session(&session_id)
            .await
            .map_err(|_| unavailable())?;
        if row.owner_ref
            != (PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: user.to_owned(),
            })
        {
            return Err(unavailable());
        }
        Ok(session_id)
    }

    pub(crate) async fn begin(
        &self,
        user: &str,
        session: &str,
        operation: &str,
        capability: &str,
        turn_id: &OperationId,
        snapshot: &ResolvedSnapshotRef,
    ) -> Result<McpEffectReceipt, AppError> {
        if [user, session, operation, capability]
            .iter()
            .any(|value| value.is_empty() || value.len() > 1024)
            || !valid_effect_identity(capability)
        {
            return Err(unavailable());
        }
        let store = self.store().await?;
        let session_id = self.owned_session(&store, user, session).await?;
        let facts = store
            .chat_causality_facts(&session_id, turn_id)
            .await
            .map_err(|_| unavailable())?;
        if facts.head.active_turn_id.as_deref() != Some(turn_id.as_ref())
            || &facts.session.agent_binding.resolved_snapshot_ref != snapshot
        {
            return Err(unavailable());
        }
        let calls = facts
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/call-started")
            .filter_map(|event| {
                let payload = facts.event_payloads.get(event.event_id.as_ref())?;
                if payload.get("operation_id").and_then(Value::as_str) == Some(operation)
                    && payload.get("capability_id").and_then(Value::as_str) == Some(capability)
                {
                    Some((event, payload.get("action_id").and_then(Value::as_str)?))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let [(tool, action_id)] = calls.as_slice() else {
            return Err(unavailable());
        };
        let causation = store.native_tool_call_causation(
            &facts.session.owner_ref, &session_id, turn_id, &operation.into(),
            &capability.into(), &(*action_id).into(),
        ).await.map_err(|_| unavailable())?;
        if causation.tool_call.event_id != tool.event_id {
            return Err(unavailable());
        }
        let effect_id = format!("mcp:{}:{operation}", session_id.as_ref());
        let identity = format!("effect:{effect_id}");
        let request = EffectEventRequest {
            agent_session_id: session_id,
            effect_id: effect_id.clone(),
            turn_id: turn_id.clone(),
            operation_id: OperationId::from(operation.to_owned()),
            owner_domain: "mcp".to_owned(),
            capability_module: CapabilityId::from(capability.to_owned()),
            action_id: ActionId::from((*action_id).to_owned()),
            resource_binding_id: None,
            resource_key: None,
            input_digest: DigestHex::from(format!(
                "{:x}",
                Sha256::digest(format!("{session}:{operation}:{capability}").as_bytes())
            )),
            recorded_at: nomifun_common::now_ms(),
            event_id: EventId::from(format!("effect-started:{effect_id}")),
            producer_id: EventProducerId::from("capability_host"),
            idempotency_key: IdempotencyKey::from(identity),
            correlation_id: CorrelationId::from(effect_id),
            strategy: EffectStrategy::ExternalUncertainEffect,
            causation_event_id: Some(tool.event_id.clone()),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
        };
        let recorded = store
            .record_effect_started(request.clone())
            .await
            .map_err(|_| unavailable())?;
        if recorded.duplicate || !recorded.persisted {
            // A durable receipt never grants a second owner dispatch.
            return Err(unavailable());
        }
        Ok(McpEffectReceipt {
            request,
            capability: capability.to_owned(),
        })
    }

    pub(crate) async fn settle(
        &self,
        receipt: McpEffectReceipt,
        outcome: &nomifun_mcp::McpInvocationOutcome<StrictJsonValue>,
    ) -> Result<(), AppError> {
        let result = outcome.result.as_ref().ok().map(|value| &value.0);
        let mut observation = match result {
            Some(result) => {
                let projected = if receipt.capability == MCP_SERVER_RESOURCE_EFFECT {
                    super::engine_mcp_media::text_projection(result)?
                } else {
                    result.clone()
                };
                bounded_observation(&projected, 4096)?
            }
            None => Value::Null,
        };
        if let Some(result) = result.filter(|_| receipt.capability == MCP_SERVER_RESOURCE_EFFECT) {
            let failure = result.get("failure").filter(|failure| !failure.is_null());
            if failure.is_some_and(|value| !value.is_object() || value.to_string().len() > 1024) {
                return Err(unavailable());
            }
            observation = json!({
                "resource_outcome": {
                    "status": if failure.is_some() { "rejected" } else { "available" },
                    "failure": failure,
                    "rollback_proven": false,
                },
                "observation": observation,
            });
        } else if let Some(result) = result {
            if !result.is_object()
                || result.get("isError").is_some_and(|value| !value.is_boolean())
            {
                return Err(unavailable());
            }
            observation["isError"] = json!(
                result.get("isError").and_then(Value::as_bool).unwrap_or(false)
            );
        }
        let cleanup_unproven = matches!(outcome.cleanup.local, nomifun_mcp::McpCleanupStatus::Unproven)
            || matches!(outcome.cleanup.remote, nomifun_mcp::McpCleanupStatus::Unproven);
        let terminal_state = if cleanup_unproven {
            EffectTerminalState::Uncertain
        } else if outcome.result.is_ok() {
            EffectTerminalState::Succeeded
        } else if !outcome.execution.request_dispatched && !outcome.execution.local_process_started {
            EffectTerminalState::Failed
        } else {
            EffectTerminalState::Uncertain
        };
        let payload = json!({
            "observation": observation,
            "execution": {
                "request_dispatched":outcome.execution.request_dispatched,
                "local_process_started":outcome.execution.local_process_started,
                "cancellation_requested":outcome.execution.cancellation_requested,
                "returned":outcome.result.is_ok(),
                "error":outcome.result.as_ref().err().map(owner_error_projection),
            },
            "cleanup": {
                "local":cleanup_status(outcome.cleanup.local),
                "remote":cleanup_status(outcome.cleanup.remote),
                "error":outcome.cleanup.error.as_ref().map(owner_error_projection),
            },
        });
        if serde_json::to_vec(&payload).map_err(|_| unavailable())?.len() > 8192 {
            return Err(unavailable());
        }
        let store = self.store().await?;
        let mut terminal = receipt.request;
        terminal.producer_id = EventProducerId::from("owning_plugin");
        terminal.recorded_at = nomifun_common::now_ms();
        terminal.event_id = EventId::from(format!("effect-terminal:{}", terminal.effect_id));
        terminal.causation_event_id = Some(EventId::from(format!(
            "effect-started:{}",
            terminal.effect_id,
        )));
        terminal.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(payload));
        store
            .record_effect_terminal(terminal, terminal_state)
            .await
            .map_err(|_| unavailable())?;
        Ok(())
    }

    pub(crate) async fn ensure_settled(&self, user: &str, session: &str) -> Result<(), AppError> {
        let store = self.store().await?;
        let session_id = self.owned_session(&store, user, session).await?;
        if store.has_unsettled_effects(&session_id).await.map_err(|_| unavailable())? {
            return Err(AppError::Conflict(
                "MCP remote outcome or cleanup remains unknown; Session stays quarantined".into(),
            ));
        }
        Ok(())
    }
}

fn owner_error_projection(error: &nomifun_mcp::McpOwnerError) -> Value {
    let mut projected = json!({ "code": error.code(), "message": error.message() });
    if let Some(diagnostic) = error.auth_diagnostic() {
        projected["details"] = json!({ "auth_diagnostic": diagnostic });
    }
    projected
}

fn bounded_observation(value: &Value, full_limit: usize) -> Result<Value, AppError> {
    let raw = serde_json::to_string(value).map_err(|_| unavailable())?;
    if raw.len() <= full_limit {
        return Ok(value.clone());
    }
    Ok(json!({
        "truncated": true,
        "sha256": format!("{:x}", Sha256::digest(raw.as_bytes())),
        "serialized_preview": raw.chars().take(512).collect::<String>(),
    }))
}

fn cleanup_status(status: nomifun_mcp::McpCleanupStatus) -> &'static str {
    match status {
        nomifun_mcp::McpCleanupStatus::NotRequired => "not_required",
        nomifun_mcp::McpCleanupStatus::Confirmed => "confirmed",
        nomifun_mcp::McpCleanupStatus::Unsupported => "unsupported",
        nomifun_mcp::McpCleanupStatus::Unproven => "unproven",
    }
}

fn valid_effect_identity(value: &str) -> bool {
    value == MCP_SERVER_RESOURCE_EFFECT || nomifun_mcp::is_namespaced_mcp_tool_capability(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_diagnostic_remains_in_the_original_effect_error_projection() {
        let diagnostic = nomifun_api_types::McpAuthDiagnostic {
            code: nomifun_api_types::McpAuthErrorCode::ReauthorizationRequired,
            stage: nomifun_api_types::McpAuthStage::Refresh,
            retryable: false, user_action: nomifun_api_types::McpAuthUserAction::Reauthorize,
            trace_id: "effect-auth-trace".into(), message: "Sign in again".into(), details: None,
        };
        let error = nomifun_mcp::McpOwnerError::new("MCP_CREDENTIAL_EXPIRED", "Sign in again")
            .with_auth_diagnostic(diagnostic);
        let projection = owner_error_projection(&error);
        assert_eq!(projection["code"], "MCP_CREDENTIAL_EXPIRED");
        assert_eq!(projection["details"]["auth_diagnostic"]["stage"], "refresh");
        assert_eq!(projection["details"]["auth_diagnostic"]["trace_id"], "effect-auth-trace");
        assert!(projection.get("authentication_challenge").is_none());
    }

    #[test]
    fn effect_receipts_accept_only_binding_resources_or_namespaced_tools() {
        assert!(valid_effect_identity(MCP_SERVER_RESOURCE_EFFECT));
        let server = nomifun_api_types::McpServerId::parse(
            "0195f7c0-7b6a-7c21-8f4a-1234567890ab",
        )
        .unwrap();
        let tool = nomifun_mcp::canonical_mcp_tool_capability_id(&server, "lookup").unwrap();
        assert!(valid_effect_identity(&tool));
        for retired in nomifun_mcp::RETIRED_MCP_AUTHORING_CAPABILITY_IDS {
            assert!(!valid_effect_identity(retired));
        }
    }
}
