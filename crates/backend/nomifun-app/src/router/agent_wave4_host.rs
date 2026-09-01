//! Application-owned Wave 4 capability host.
//!
//! Fresh-v4 does not mount the legacy Channel, Companion, Customer Service, or
//! Robot service graph. Until those package owners have v4-native resources,
//! every action fails closed through this typed port. The adapter does not
//! construct a fallback service, retry an operation, or manufacture success.

use std::future::Future;
use std::pin::Pin;

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_agent_domain_wave4::{
    Wave4HostPort, Wave4HostPortError, Wave4HostRequest,
};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Wave4ApplicationHost;

impl Wave4HostPort for Wave4ApplicationHost {
    fn invoke<'a>(
        &'a self,
        request: Wave4HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave4HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            Err(Wave4HostPortError::unavailable(format!(
                "Fresh-v4 has no native owner for {} resource action",
                request.context.capability_id.as_ref()
            )))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, CorrelationId, IdempotencyKey,
        OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceBindingId,
        ResourceId, ResourceKind, ScopeKey, StrictJsonValue,
        TypedResourceBinding,
    };
    use nomifun_agent_domain_wave4::{
        CHANNEL_REPLY, CHANNEL_REPLY_ACTION, CHANNEL_RESOURCE_KIND,
        Wave4CapabilityOperation, Wave4HostContext, Wave4HostRequest,
    };

    use super::*;

    #[tokio::test]
    async fn unavailable_wave4_owner_never_projects_success() {
        let host = Wave4ApplicationHost;
        let error = host
            .invoke(Wave4HostRequest {
                context: Wave4HostContext {
                    principal: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: "owner".to_owned(),
                    },
                    agent_session_id: AgentSessionId::from(
                        nomifun_common::generate_id(),
                    ),
                    operation_id: OperationId::from("wave4-operation"),
                    idempotency_key: IdempotencyKey::from("wave4-idempotency"),
                    correlation_id: CorrelationId::from("wave4-correlation"),
                    resolved_snapshot_ref: ResolvedSnapshotRef {
                        snapshot_id: "wave4-snapshot".into(),
                        snapshot_digest: "a".repeat(64).into(),
                    },
                    registry_generation: 1,
                    capability_id: CapabilityId::from(CHANNEL_REPLY),
                    action_id: ActionId::from(CHANNEL_REPLY_ACTION),
                    state_scope_key: ScopeKey::from("session:wave4"),
                    resource_bindings: vec![TypedResourceBinding {
                        binding_id: ResourceBindingId::from("channel"),
                        resource_kind: ResourceKind::from(
                            CHANNEL_RESOURCE_KIND,
                        ),
                        resource_id: ResourceId::from("channel-resource"),
                        owner_id: "owner".to_owned(),
                        operations: BTreeSet::from(["reply".to_owned()]),
                        connection_config_ref: None,
                        typed_parameters: BTreeMap::new(),
                    }],
                },
                operation: Wave4CapabilityOperation::ChannelReply {
                    input: StrictJsonValue(serde_json::json!({
                        "text": "hello"
                    })),
                },
            })
            .await
            .expect_err("an unowned Wave 4 action must fail closed");

        assert_eq!(error.code, "WAVE4_HOST_PORT_UNAVAILABLE");
        assert!(!error.message.contains("accepted"));
        assert!(!error.message.contains("completed"));
    }
}
