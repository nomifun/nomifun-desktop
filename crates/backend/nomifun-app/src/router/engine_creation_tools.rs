//! Conversation-owned adapter for canonical Creation Actions.
//!
//! The model sees only creative parameters. The current Conversation/Turn
//! target is injected by the host after tool-call admission, so a model can
//! never choose another Session or forge a message owner.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::{AgentSessionId, StrictJsonValue};
use nomifun_engine_core::{
    EngineToolError, EngineToolInvocation, EngineToolInvoker, EngineToolResult,
};
use tokio_util::sync::CancellationToken;

pub(super) const CREATION_CAPABILITY_ID: &str = "creation.media";

pub(super) fn is_creation_action(capability_id: &str, action_id: &str) -> bool {
    capability_id == CREATION_CAPABILITY_ID && action_id.starts_with("creation.media/")
}

/// Hide the host-owned target from the model-facing schema while retaining the
/// canonical schema reference in the compiled binding. The invocation adapter
/// below restores the exact target before the Kernel validates and dispatches
/// the canonical Action input.
pub(super) fn conversation_creation_schema(
    capability_id: &str,
    action_id: &str,
    mut schema: StrictJsonValue,
) -> StrictJsonValue {
    if !is_creation_action(capability_id, action_id) {
        return schema;
    }
    if let Some(properties) = schema
        .0
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        properties.remove("target");
    }
    if let Some(required) = schema
        .0
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
    {
        required.retain(|key| key.as_str() != Some("target"));
    }
    schema
}

pub(super) type CreationTurnRoot = Arc<Mutex<Option<String>>>;

pub(super) struct ConversationCreationTools {
    pub inner: Arc<dyn EngineToolInvoker>,
    pub session_id: AgentSessionId,
    pub turn_root: CreationTurnRoot,
}

fn tool_error(message: impl std::fmt::Display) -> EngineToolError {
    EngineToolError::ToolInvocation(format!("Conversation creation: {message}"))
}

#[async_trait]
impl EngineToolInvoker for ConversationCreationTools {
    async fn invoke(
        &self,
        mut invocation: EngineToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<EngineToolResult, EngineToolError> {
        if is_creation_action(
            invocation.binding.capability_id.as_ref(),
            invocation.binding.action_id.as_ref(),
        ) {
            let message_id = self
                .turn_root
                .lock()
                .map_err(|_| tool_error("turn identity lock poisoned"))?
                .clone()
                .ok_or_else(|| tool_error("no admitted active turn"))?;
            let arguments = invocation
                .call
                .arguments
                .0
                .as_object_mut()
                .ok_or_else(|| tool_error("Action input must be an object"))?;
            arguments.insert(
                "target".to_owned(),
                serde_json::json!({
                    "kind": "conversation_turn",
                    "conversation_id": self.session_id.as_ref(),
                    "message_id": message_id,
                }),
            );
        }
        self.inner.invoke(invocation, cancellation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        ActionId, CapabilityId, CorrelationId, DigestHex, IdempotencyKey, OperationId,
        PrincipalRef, ResolvedSnapshotRef,
    };
    use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ToolCallId};
    use nomifun_engine_core::{EngineEffectClass, EngineToolBinding};
    use std::collections::BTreeSet;

    struct Capture(Arc<Mutex<Option<serde_json::Value>>>);

    #[async_trait]
    impl EngineToolInvoker for Capture {
        async fn invoke(
            &self,
            invocation: EngineToolInvocation,
            _: CancellationToken,
        ) -> Result<EngineToolResult, EngineToolError> {
            *self.0.lock().unwrap() = Some(invocation.call.arguments.0);
            Ok(EngineToolResult::text(invocation.call.call_id, "ok", false))
        }
    }

    fn invocation() -> EngineToolInvocation {
        let input_schema = StrictJsonValue(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"prompt": {"type": "string"}},
            "required": ["prompt"]
        }));
        EngineToolInvocation {
            agent_session_id: AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002"),
            principal: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "a".repeat(64).into(),
            },
            active_set_generation: 0,
            turn_operation_id: OperationId::from("turn"),
            operation_id: OperationId::from("tool"),
            idempotency_key: IdempotencyKey::from("key"),
            correlation_id: CorrelationId::from("correlation"),
            call: ChatToolCall {
                call_id: ToolCallId::from("call"),
                name: "creation_image".into(),
                arguments: StrictJsonValue(serde_json::json!({"prompt": "fox"})),
                provider_metadata: Default::default(),
            },
            binding: EngineToolBinding {
                model_name: "creation_image".into(),
                definition: ChatToolDefinition {
                    name: "creation_image".into(),
                    description: "create image".into(),
                    input_schema,
                    deferred: false,
                },
                schema_digest: DigestHex::from("b".repeat(64)),
                canonical_input_schema_ref: "schema://creation.media/image/input".into(),
                capability_contract_digest: DigestHex::from("c".repeat(64)),
                capability_id: CapabilityId::from(CREATION_CAPABILITY_ID),
                action_id: ActionId::from("creation.media/image"),
                resource_binding_ids: BTreeSet::new(),
                effect_class: EngineEffectClass::ManagedEffect,
                parallel_safe: false,
            },
        }
    }

    #[test]
    fn model_schema_hides_the_host_owned_target() {
        let schema = conversation_creation_schema(
            CREATION_CAPABILITY_ID,
            "creation.media/image",
            StrictJsonValue(serde_json::json!({
                "type":"object",
                "additionalProperties":false,
                "properties":{"target":{},"prompt":{"type":"string"}},
                "required":["target","prompt"]
            })),
        );
        assert!(schema.0["properties"].get("target").is_none());
        assert_eq!(schema.0["required"], serde_json::json!(["prompt"]));
    }

    #[tokio::test]
    async fn host_injects_the_exact_conversation_turn_target() {
        let captured = Arc::new(Mutex::new(None));
        let message_id = uuid::Uuid::now_v7().to_string();
        let adapter = ConversationCreationTools {
            inner: Arc::new(Capture(captured.clone())),
            session_id: AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002"),
            turn_root: Arc::new(Mutex::new(Some(message_id.clone()))),
        };
        adapter
            .invoke(invocation(), CancellationToken::new())
            .await
            .unwrap();
        let captured = captured.lock().unwrap();
        let target = &captured.as_ref().unwrap()["target"];
        assert_eq!(target["kind"], "conversation_turn");
        assert_eq!(
            target["conversation_id"],
            "0190f5fe-7c00-7a00-8000-000000000002"
        );
        assert_eq!(target["message_id"], message_id);
    }
}
