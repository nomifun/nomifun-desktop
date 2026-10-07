//! Canonical MCP effect receipts shared by every Runtime consumer.

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, CorrelationId, DigestHex, EventId,
    EventProducerId, IdempotencyKey, OperationId, PrincipalRef,
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
        let head = store.head(&session_id).await.map_err(|_| unavailable())?;
        let turn_id = OperationId::from(head.active_turn_id.ok_or_else(unavailable)?);
        let facts = store
            .chat_causality_facts(&session_id, &turn_id)
            .await
            .map_err(|_| unavailable())?;
        let (tool, action_id) = facts
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/call-started")
            .find_map(|event| {
                let payload = facts.event_payloads.get(event.event_id.as_ref())?;
                (payload.get("operation_id").and_then(Value::as_str) == Some(operation)
                    && payload.get("capability_id").and_then(Value::as_str) == Some(capability))
                    .then(|| {
                        (
                            event,
                            payload
                                .get("action_id")
                                .and_then(Value::as_str)
                                .unwrap_or("mcp/invoke")
                                .to_owned(),
                        )
                    })
            })
            .ok_or_else(unavailable)?;
        let effect_id = format!("mcp:{}:{operation}", session_id.as_ref());
        let identity = format!("effect:{effect_id}");
        let request = EffectEventRequest {
            agent_session_id: session_id,
            effect_id: effect_id.clone(),
            turn_id,
            operation_id: OperationId::from(operation.to_owned()),
            owner_domain: "mcp".to_owned(),
            capability_module: CapabilityId::from(capability.to_owned()),
            action_id: ActionId::from(action_id),
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
        store
            .record_effect_started(request.clone())
            .await
            .map_err(|_| unavailable())?;
        Ok(McpEffectReceipt {
            request,
            capability: capability.to_owned(),
        })
    }

    pub(crate) async fn settle(
        &self,
        receipt: McpEffectReceipt,
        result: &Value,
    ) -> Result<(), AppError> {
        let projected = if receipt.capability == MCP_SERVER_RESOURCE_EFFECT {
            super::engine_mcp_media::text_projection(result)?
        } else {
            result.clone()
        };
        let mut observation = bounded_observation(&projected, 4096)?;
        if receipt.capability == MCP_SERVER_RESOURCE_EFFECT {
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
        } else {
            if !result.is_object()
                || result.get("isError").is_some_and(|value| !value.is_boolean())
            {
                return Err(unavailable());
            }
            observation["isError"] = json!(
                result.get("isError").and_then(Value::as_bool).unwrap_or(false)
            );
        }
        if serde_json::to_vec(&observation).map_err(|_| unavailable())?.len() > 8192 {
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
        terminal.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "observation": observation,
        })));
        store
            .record_effect_terminal(terminal, EffectTerminalState::Succeeded)
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

fn valid_effect_identity(value: &str) -> bool {
    value == MCP_SERVER_RESOURCE_EFFECT || nomifun_mcp::is_namespaced_mcp_tool_capability(value)
}

#[cfg(test)]
mod tests {
    use super::*;

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
