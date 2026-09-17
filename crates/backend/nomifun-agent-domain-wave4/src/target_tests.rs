use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CapabilityId, CorrelationId, IdempotencyKey,
    OperationId, PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId,
    ResourceKind, ScopeKey, StrictJsonValue, TypedResourceBinding,
};

use super::*;
use nomifun_agent_kernel::{
    InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
};

fn context(
    module_id: &str,
    action_id: &str,
    kind: &str,
    operation: &str,
) -> Wave4HostContext {
    Wave4HostContext {
        principal: PrincipalRef {
            principal_kind: "user".into(),
            principal_id: "owner-a".into(),
        },
        agent_session_id: AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000001"),
        operation_id: OperationId::from("wave4-operation"),
        idempotency_key: IdempotencyKey::from("wave4-idempotency"),
        correlation_id: CorrelationId::from("wave4-correlation"),
        resolved_snapshot_ref: ResolvedSnapshotRef {
            snapshot_id: "wave4-snapshot".into(),
            snapshot_digest: "a".repeat(64).into(),
        },
        registry_generation: 1,
        capability_id: CapabilityId::from(module_id),
        action_id: ActionId::from(action_id),
        state_scope_key: ScopeKey::from("session:wave4"),
        resource_bindings: vec![TypedResourceBinding {
            binding_id: ResourceBindingId::from(format!("{kind}-binding")),
            resource_kind: ResourceKind::from(kind),
            resource_id: ResourceId::from(format!("{kind}-resource")),
            owner_id: "owner-a".into(),
            operations: BTreeSet::from([operation.to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        }],
    }
}

#[test]
fn registrations_publish_product_modules_without_scene_or_transport_fragments() {
    let registrations = registrations().unwrap();
    assert_eq!(registrations.len(), PACKAGE_IDS.len());
    let capabilities = registrations
        .iter()
        .flat_map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .iter()
        })
        .map(|capability| capability.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(capabilities, target_capability_ids());
    for retired in [
        CHANNEL_RECEIVE,
        "channel.reply",
        "channel.send",
        CHANNEL_PAIRING,
        CHANNEL_GROUP_POLICY,
        COMPANION_PERSONA,
        COMPANION_ROSTER,
        "companion.learn",
        "companion.evolve",
        CUSTOMER_SERVICE_DIALOGUE,
        "customer_service.notes.read",
        "customer_service.notes.write",
        "customer_service.handoff",
    ] {
        assert!(!capabilities.contains(&CapabilityId::from(retired)));
    }
}

#[test]
fn conversation_module_action_inventory_is_exact() {
    let channel = channel_registration().unwrap();
    let channel_module = &channel.metadata.manifest.payload.contributions.capabilities[0];
    assert_eq!(channel_module.id.as_ref(), CHANNEL_MESSAGING_MODULE_ID);
    assert_eq!(
        channel_module
            .contributions
            .actions
            .iter()
            .map(|action| action.action_id.as_ref())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            CHANNEL_MESSAGING_REPLY_ACTION_ID,
            CHANNEL_MESSAGING_SEND_ACTION_ID,
        ])
    );

    let companion = companion_registration().unwrap();
    assert_eq!(companion.metadata.manifest.payload.contributions.capabilities.len(), 1);
    let customer = customer_service_registration().unwrap();
    assert_eq!(customer.metadata.manifest.payload.contributions.capabilities.len(), 1);
}

#[test]
fn typed_operations_bind_module_action_and_resource_operation() {
    let request = Wave4HostRequest {
        context: context(
            CHANNEL_MESSAGING_MODULE_ID,
            CHANNEL_MESSAGING_REPLY_ACTION_ID,
            CHANNEL_RESOURCE_KIND,
            "reply",
        ),
        operation: operation_from_action_input(
            &CapabilityId::from(CHANNEL_MESSAGING_MODULE_ID),
            &ActionId::from(CHANNEL_MESSAGING_REPLY_ACTION_ID),
            StrictJsonValue(serde_json::json!({
                "destination_ref": "chat-a",
                "message_ref": "message-a",
                "text": "hello",
            })),
        )
        .unwrap(),
    };
    request.validate().unwrap();

    let mut wrong_owner = request.clone();
    wrong_owner.context.resource_bindings[0].owner_id = "owner-b".into();
    assert_eq!(
        wrong_owner.validate().unwrap_err().code,
        WAVE4_RESOURCE_OWNER_MISMATCH
    );

    let mut wrong_action = request;
    wrong_action.context.action_id = ActionId::from(CHANNEL_MESSAGING_SEND_ACTION_ID);
    assert_eq!(
        wrong_action.validate().unwrap_err().code,
        WAVE4_ACTION_OPERATION_MISMATCH
    );
}

#[test]
fn action_resource_contracts_are_exact_and_scene_authority_is_not_agent_authored() {
    assert_eq!(
        required_action_resource_operations(
            COMPANION_MODULE_ID,
            COMPANION_LEARN_ACTION_ID,
        ),
        Some(vec![(ResourceKind::from(COMPANION_RESOURCE_KIND), "write".into())])
    );
    assert_eq!(
        required_action_resource_operations(
            CUSTOMER_SERVICE_MODULE_ID,
            CUSTOMER_SERVICE_NOTES_READ_ACTION_ID,
        ),
        Some(vec![(ResourceKind::from(CUSTOMER_RESOURCE_KIND), "read".into())])
    );
    for retired in [CHANNEL_RECEIVE, CHANNEL_PAIRING, CHANNEL_GROUP_POLICY, COMPANION_PERSONA] {
        assert!(required_action_resource_operations(retired, "invoke").is_none());
    }
}

#[test]
fn canonical_schemas_resolve_for_every_conversation_action() {
    for registration in [
        channel_registration().unwrap(),
        companion_registration().unwrap(),
        customer_service_registration().unwrap(),
    ] {
        for capability in &registration.metadata.manifest.payload.contributions.capabilities {
            assert_eq!(capability.contributions.context_schema_refs.len(), 1);
            assert_eq!(
                capability.contributions.context_phase,
                nomifun_agent_contracts::ContextContributionPhase::BeforeTurn,
            );
            assert!(
                resolve_capability_schema(&capability.contributions.context_schema_refs[0])
                    .unwrap()
                    .is_some()
            );
            for action in &capability.contributions.actions {
                assert!(resolve_capability_schema(&action.input_schema).unwrap().is_some());
                assert!(resolve_capability_schema(&action.output_schema).unwrap().is_some());
            }
        }
    }

    for scene_id in [COMPANION_PERSONA, COMPANION_ROSTER, CUSTOMER_SERVICE_DIALOGUE] {
        let reference = scene_context_schema_ref(scene_id).unwrap().unwrap();
        assert!(resolve_capability_schema(&reference).unwrap().is_some());
        assert!(!target_capability_ids().contains(&CapabilityId::from(scene_id)));
    }
}

#[test]
fn conversation_modules_materialize_action_and_scene_context_together() {
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable(CONTRACT_VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    let snapshot = registry.replace_all(registrations().unwrap()).unwrap();
    for module_id in CONVERSATION_MODULE_IDS {
        let capability = snapshot
            .capabilities
            .get(&CapabilityId::from(module_id))
            .unwrap();
        assert!(!capability.manifest.contributions.actions.is_empty());
        assert_eq!(capability.manifest.contributions.context_schema_refs.len(), 1);
    }
}
