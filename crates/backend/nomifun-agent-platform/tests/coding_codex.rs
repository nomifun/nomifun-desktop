use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetId, AgentSessionDeletedState, AgentSessionId,
    AgentSessionLiveRecord, AgentSessionMetadata, CapabilityRef,
    CheckpointDiscardReason, CheckpointRehydrateSource, CorrelationId, DigestHex, EventId,
    EventProducerId, IdempotencyKey, NativeActionStartAckExchange, OperationId, PackageId,
    PackageRef, PrecomputedActivationPlan, PresetRevisionRef, PrincipalRef, ResolvedCapability,
    ResolvedSnapshotContent, ResolvedSnapshotId, ResolvedSnapshotRef, ResourceBindingId,
    ResourceId, ResourceKind, RuntimeCheckpointValidationResult,
    RuntimeProfileKind, SessionEventKind, SessionEventPayloadRef, SessionEventRecord,
    SnapshotCompatibilityAdmissionResult, StrictJsonValue, TypedResourceBinding,
    VersionString,
};
use nomifun_agent_platform::{
    CodingCodexContract, CodingReviewEvidence, CodingSurface, CodingWorkspaceEvidence,
};
use nomifun_agent_session::{
    CheckpointAdmission, DeleteResult, MessageProjection, SessionHeadProjection,
    SessionObservation,
};
use nomifun_codex_runtime::{
    CheckpointDisposition, DisposeRpcOutcome, PinnedRuntimeProfile, ProcessTreeDisposeReport,
    RuntimeDisposeReport,
};
use serde_json::json;

fn owner() -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: "coding-owner".to_owned(),
    }
}

fn snapshot_ref() -> ResolvedSnapshotRef {
    ResolvedSnapshotRef {
        snapshot_id: ResolvedSnapshotId::from("coding-snapshot"),
        snapshot_digest: DigestHex::from("coding-snapshot-digest"),
    }
}

fn workspace_binding() -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from("workspace"),
        resource_kind: ResourceKind::from("workspace"),
        resource_id: ResourceId::from("workspace-fixture"),
        owner_id: owner().principal_id,
        operations: BTreeSet::from([
            "execute".to_owned(),
            "read".to_owned(),
            "write".to_owned(),
        ]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
}

fn resolved_capability(reference: &CapabilityRef) -> ResolvedCapability {
    ResolvedCapability {
        capability: reference.clone(),
        source_package: PackageRef {
            id: PackageId::from(format!(
                "fixture.{}",
                reference.id.as_ref().replace('.', "-")
            )),
            version: reference.version.clone(),
        },
        schema_digest: DigestHex::from(format!(
            "{}-schema",
            reference.id.as_ref()
        )),
        dependency_path: vec![reference.id.clone()],
        required_runtime_features: BTreeSet::new(),
    }
}

fn resolved_content(contract: &CodingCodexContract) -> ResolvedSnapshotContent {
    let activation_plans = contract
        .on_demand_capabilities
        .iter()
        .map(|reference| {
            (
                reference.id.clone(),
                PrecomputedActivationPlan {
                    root_capability_id: reference.id.clone(),
                    capability_bundle: vec![reference.id.clone()],
                    tool_schema_refs: Vec::new(),
                    context_schema_refs: Vec::new(),
                    resource_binding_refs: vec![ResourceBindingId::from(
                        "workspace",
                    )],
                    model_route_refs: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let compact_on_demand_index = contract
        .on_demand_capabilities
        .iter()
        .map(|reference| nomifun_agent_contracts::CompactOnDemandCapabilityEntry {
            capability_id: reference.id.clone(),
            display_name: reference.id.as_ref().to_owned(),
            short_description: "Coding on-demand capability".to_owned(),
            search_terms: vec![reference.id.as_ref().to_owned()],
            activation_plan_digest: DigestHex::from(format!(
                "{}-plan",
                reference.id.as_ref()
            )),
        })
        .collect();
    ResolvedSnapshotContent {
        schema_version: VersionString::from("1.0.0"),
        resolver_version: VersionString::from("1.0.0"),
        preset_revision_ref: PresetRevisionRef {
            preset_id: AgentPresetId::from("coding.codex.fixture"),
            revision: 1,
            revision_digest: DigestHex::from("coding-revision"),
        },
        required_runtime_protocol_version: VersionString::from("1.0.0"),
        required_runtime_profile: RuntimeProfileKind::CodingNative,
        runtime_feature_inventory_digest: contract
            .runtime_feature_inventory_digest
            .clone(),
        required_runtime_features: contract.required_runtime_features.clone(),
        compiled_runtime_profile_digest: DigestHex::from("coding-profile"),
        model_route_refs: BTreeMap::new(),
        chat_route_identity: None,
        initial_capabilities: contract
            .initial_capabilities
            .iter()
            .map(resolved_capability)
            .collect(),
        on_demand_capabilities: contract
            .on_demand_capabilities
            .iter()
            .map(resolved_capability)
            .collect(),
        on_demand_activation_plans: activation_plans,
        compact_on_demand_index,
        capability_allowlist: contract.ceiling_ids(),
        skill_locks: Vec::new(),
        mcp_tool_locks: Vec::new(),
        resolved_role_providers: BTreeMap::new(),
        typed_resource_bindings: vec![workspace_binding()],
        canonical_schema_manifest_digest: DigestHex::from("schema"),
        target_contribution_manifest_digest: contract
            .target_contribution_manifest_digest
            .clone(),
    }
}

fn agent_binding() -> AgentBindingValue {
    AgentBindingValue {
        preset_revision_ref: PresetRevisionRef {
            preset_id: AgentPresetId::from("coding.codex.fixture"),
            revision: 1,
            revision_digest: DigestHex::from("coding-revision"),
        },
        resolved_snapshot_ref: snapshot_ref(),
        typed_resource_bindings: vec![workspace_binding()],
        binding_version: 1,
    }
}

fn event(
    session_id: &AgentSessionId,
    seq: u64,
    kind: &str,
    correlation: &str,
    payload: serde_json::Value,
) -> SessionEventRecord {
    SessionEventRecord {
        agent_session_id: session_id.clone(),
        seq,
        event_id: EventId::from(format!("event-{seq}")),
        producer_id: EventProducerId::from("coding-gate"),
        idempotency_key: IdempotencyKey::from(format!("idem-{seq}")),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        kind: SessionEventKind(kind.to_owned()),
        kind_version: 1,
        correlation_id: CorrelationId::from(correlation),
        causation_event_id: (seq > 1).then(|| EventId::from(format!("event-{}", seq - 1))),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
    }
}

fn projection(
    session_id: &AgentSessionId,
    intent: &str,
) -> MessageProjection {
    MessageProjection {
        session_id: session_id.clone(),
        projection_id: format!("{intent}:fixture"),
        first_seq: 1,
        last_seq: 1,
        presentation_intent: intent.to_owned(),
        projection: json!({"state": "fixture"}),
        semantic_digest: format!("{intent}-digest"),
    }
}

fn successful_observation() -> SessionObservation {
    let session_id =
        AgentSessionId::from("0199a8c0-0000-7000-8000-000000000100");
    let mut events = Vec::new();
    let mut push = |kind: &str, correlation: &str, payload| {
        let seq = events.len() as u64 + 1;
        events.push(event(&session_id, seq, kind, correlation, payload));
    };
    push("session/opening", "session", json!({}));
    push(
        "capability/active-set-committed",
        "capability",
        json!({"generation": 0}),
    );
    push("session/ready", "session", json!({}));
    push("runtime/bound", "runtime", json!({}));
    push("turn/started", "turn", json!({}));
    for surface in CodingSurface::ALL {
        let correlation = format!("tool-{}", surface.as_str());
        push(
            "tool/call-started",
            &correlation,
            json!({"coding_surface": surface.as_str()}),
        );
        push(
            "tool/result-recorded",
            &correlation,
            json!({"coding_surface": surface.as_str()}),
        );
        if CodingSurface::EFFECTFUL.contains(&surface) {
            let effect = format!("effect-{}", surface.as_str());
            push(
                "effect/started",
                &effect,
                json!({"coding_surface": surface.as_str()}),
            );
            push(
                "effect/succeeded",
                &effect,
                json!({"coding_surface": surface.as_str()}),
            );
        }
    }
    push("turn/completed", "turn", json!({}));
    push(
        "capability/active-set-committed",
        "capability",
        json!({"generation": 1}),
    );
    let last_seq = events.len() as u64;
    SessionObservation {
        session: AgentSessionLiveRecord {
            agent_session_id: session_id.clone(),
            owner_ref: owner(),
            metadata: AgentSessionMetadata {
                title: Some("Coding".to_owned()),
                archived: false,
                pinned: false,
            },
            agent_binding: agent_binding(),
            remote_binding_provenance: None,
            parent_session_id: None,
            fork_base_payload_id: None,
            next_seq: last_seq + 1,
        },
        head: SessionHeadProjection {
            session_id: session_id.clone(),
            status: "ready".to_owned(),
            active_turn_id: None,
            active_set_generation: 1,
            runtime_checkpoint_locator: None,
            runtime_checkpoint_digest: None,
            runtime_bound_event_id: Some("event-4".to_owned()),
            runtime_protocol_version: Some("1.0.0".to_owned()),
            snapshot_digest: Some("coding-snapshot-digest".to_owned()),
            checkpoint_through_seq: Some(4),
            last_seq,
            unread_count: 0,
        },
        events,
        messages: [
            "turn_status",
            "tool",
            "effect",
            "runtime",
            "capability",
        ]
        .into_iter()
        .map(|intent| projection(&session_id, intent))
        .collect(),
        next_cursor: nomifun_agent_contracts::SessionEventCursor {
            agent_session_id: session_id,
            seq: last_seq,
        },
    }
}

#[test]
fn frozen_coding_contract_validates_snapshot_profile_and_native_ack() {
    let contract = CodingCodexContract::frozen().unwrap();
    let content = resolved_content(&contract);
    contract.validate_snapshot(&content).unwrap();
    let profile = PinnedRuntimeProfile {
        kind: RuntimeProfileKind::CodingNative,
        runtime_protocol_version: VersionString::from("1.0.0"),
        profile_digest: content.compiled_runtime_profile_digest,
        enabled_runtime_features: contract.required_runtime_features.clone(),
        initial_capabilities: contract.initial_ids(),
        on_demand_capabilities: contract.on_demand_ids(),
        typed_resource_bindings: content.typed_resource_bindings,
    };
    contract.validate_runtime_profile(&profile).unwrap();

    let mut exchange: NativeActionStartAckExchange = serde_json::from_str(include_str!(
        "../../nomifun-agent-contracts/contracts/runtime/native-action-start-ack.json"
    ))
    .unwrap();
    let native_action = contract
        .native_actions
        .iter()
        .next()
        .cloned()
        .expect("frozen Coding inventory must contain native actions");
    exchange.start.action_id = native_action;
    exchange.ack.action_id = exchange.start.action_id.clone();
    contract
        .validate_native_action_ack(&exchange.start, &exchange.ack)
        .unwrap();
}

#[test]
fn coding_event_projection_cancel_checkpoint_dispose_and_delete_contracts_hold() {
    let contract = CodingCodexContract::frozen().unwrap();
    let observation = successful_observation();
    let evidence = contract
        .validate_success_observation(&observation)
        .unwrap();
    assert_eq!(
        evidence.tool_surfaces,
        CodingSurface::ALL.into_iter().collect()
    );
    contract
        .validate_workspace_preservation(&CodingWorkspaceEvidence {
            dirty_before: BTreeMap::from([
                ("user-dirty.txt".to_owned(), DigestHex::from("dirty-before")),
                ("agent-target.rs".to_owned(), DigestHex::from("target-before")),
            ]),
            content_after: BTreeMap::from([
                ("user-dirty.txt".to_owned(), DigestHex::from("dirty-before")),
                ("agent-target.rs".to_owned(), DigestHex::from("target-after")),
            ]),
            agent_touched_paths: BTreeSet::from(["agent-target.rs".to_owned()]),
        })
        .unwrap();
    contract
        .validate_review_workflow(&CodingReviewEvidence {
            diff_reviewed: true,
            review_comment_count: 2,
            review_fixes_applied: true,
            tests_run: true,
            tests_passed: true,
        })
        .unwrap();
    contract
        .validate_rebuild(&observation, &observation.clone())
        .unwrap();

    let mut cancelled = observation.clone();
    let seq = cancelled.events.len() as u64 + 1;
    cancelled.events.push(event(
        &cancelled.session.agent_session_id,
        seq,
        "turn/cancelled",
        "cancelled-turn",
        json!({}),
    ));
    contract.validate_cancel_observation(&cancelled).unwrap();

    let admission = CheckpointAdmission {
        validation: RuntimeCheckpointValidationResult::ExactMatch,
        compatibility: Some(SnapshotCompatibilityAdmissionResult::CompatibleExact {
            runtime_release_digest: DigestHex::from("release"),
            hello_payload_digest: DigestHex::from("hello"),
        }),
        checkpoint_reusable: true,
    };
    contract.validate_checkpoint_admission(&admission).unwrap();
    let discard = CheckpointDisposition::Discard {
        reasons: vec![CheckpointDiscardReason::SnapshotMismatch],
        rehydrate_from: vec![
            CheckpointRehydrateSource::ExactSnapshot,
            CheckpointRehydrateSource::LatestCompletedCompaction,
            CheckpointRehydrateSource::SubsequentCanonicalEvents,
        ],
        checkpoint_converter_allowed: false,
    };
    contract.validate_checkpoint_discard(&discard).unwrap();

    let dispose = RuntimeDisposeReport {
        agent_session_id: observation.session.agent_session_id.clone(),
        runtime_binding_id: nomifun_agent_contracts::RuntimeBindingId::from(
            "coding-binding",
        ),
        rpc: DisposeRpcOutcome::Acked,
        process_tree: ProcessTreeDisposeReport::default(),
    };
    contract
        .validate_dispose(
            &dispose,
            &observation.session.agent_session_id,
            true,
        )
        .unwrap();

    let delete = DeleteResult {
        tombstone: nomifun_agent_contracts::AgentSessionTombstone {
            agent_session_id: observation.session.agent_session_id.clone(),
            owner_ref: owner(),
            state: AgentSessionDeletedState::Deleted,
            deleted_at: 1,
        },
        operation_id: OperationId::from("delete-coding"),
    };
    contract
        .validate_delete(
            &delete,
            &observation.session.agent_session_id,
            &owner(),
        )
        .unwrap();
    contract.validate_deleted_error(Some("SESSION_DELETED")).unwrap();
}
