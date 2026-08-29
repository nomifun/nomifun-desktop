use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    CanonicalErrorCode, CheckpointDiscardReason, RuntimeCheckpointValidationInput,
    RuntimeCheckpointValidationResult, SNAPSHOT_EXECUTOR_UNAVAILABLE,
    SnapshotCompatibilityAdmissionInput, SnapshotCompatibilityAdmissionResult,
    SnapshotContractMismatch, SnapshotContractMismatchKind,
};

pub fn validate_checkpoint(
    input: &RuntimeCheckpointValidationInput,
) -> RuntimeCheckpointValidationResult {
    let mut mismatches = Vec::new();

    if input.checkpoint.runtime_bound_event_id != input.expected_runtime_bound_event_id
        || input.referenced_runtime_build_digest != input.expected_runtime_build_digest
    {
        mismatches.push(CheckpointDiscardReason::RuntimeBoundEventMismatch);
    }
    if input.checkpoint.protocol_version != input.expected_protocol_version {
        mismatches.push(CheckpointDiscardReason::ProtocolMismatch);
    }
    if input.checkpoint.resolved_snapshot_ref != input.expected_snapshot_ref {
        mismatches.push(CheckpointDiscardReason::SnapshotMismatch);
    }
    if input.checkpoint.through_seq != input.expected_through_seq {
        mismatches.push(CheckpointDiscardReason::ThroughSeqMismatch);
    }

    if mismatches.is_empty() {
        RuntimeCheckpointValidationResult::ExactMatch
    } else {
        RuntimeCheckpointValidationResult::Mismatch { mismatches }
    }
}

pub fn evaluate_snapshot_compatibility(
    input: &SnapshotCompatibilityAdmissionInput,
) -> SnapshotCompatibilityAdmissionResult {
    let required = &input.required_ceiling;
    let available = &input.available_executor;
    let mut mismatches = Vec::new();

    if !available
        .protocol_versions
        .contains(&required.protocol_version)
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::ProtocolVersion,
            "runtime_protocol",
            required.protocol_version.as_ref(),
            available
                .protocol_versions
                .iter()
                .map(|value| value.as_ref())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if !available
        .protocol_schema_digests
        .contains(&required.protocol_schema_digest)
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::ProtocolSchema,
            "runtime_protocol_schema",
            required.protocol_schema_digest.as_ref(),
            available
                .protocol_schema_digests
                .iter()
                .map(|value| value.as_ref())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if !available
        .profile_digests
        .get(&required.profile_kind)
        .is_some_and(|digests| digests.contains(&required.profile_digest))
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::RuntimeProfile,
            format!("{:?}", required.profile_kind),
            required.profile_digest.as_ref(),
            available
                .profile_digests
                .get(&required.profile_kind)
                .map(|digests| {
                    digests
                        .iter()
                        .map(|value| value.as_ref())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default(),
        );
    }

    for feature in required
        .native_features
        .difference(&available.native_features)
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::NativeFeature,
            feature.as_ref(),
            "required",
            "missing",
        );
    }
    for action in required
        .native_actions
        .difference(&available.native_actions)
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::NativeAction,
            action.as_ref(),
            "required",
            "missing",
        );
    }

    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::InitialCapability,
        &required.initial_capabilities,
        &available.capabilities,
    );
    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::OnDemandCapability,
        &required.on_demand_capabilities,
        &available.capabilities,
    );
    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::Package,
        &required.packages,
        &available.packages,
    );
    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::Skill,
        &required.skills,
        &available.skills,
    );
    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::McpTool,
        &required.mcp_tools,
        &available.mcp_tools,
    );
    compare_map(
        &mut mismatches,
        SnapshotContractMismatchKind::ModelRoute,
        &required.model_routes,
        &available.model_routes,
    );

    if !available
        .typed_resource_contract_digests
        .contains(&required.typed_resource_contract_digest)
    {
        mismatch(
            &mut mismatches,
            SnapshotContractMismatchKind::TypedResourceContract,
            "typed_resources",
            required.typed_resource_contract_digest.as_ref(),
            available
                .typed_resource_contract_digests
                .iter()
                .map(|value| value.as_ref())
                .collect::<Vec<_>>()
                .join(","),
        );
    }

    if mismatches.is_empty() {
        SnapshotCompatibilityAdmissionResult::CompatibleExact {
            runtime_release_digest: available.runtime_release_digest.clone(),
            hello_payload_digest: available.hello_payload_digest.clone(),
        }
    } else {
        SnapshotCompatibilityAdmissionResult::ExecutorUnavailable {
            error_code: CanonicalErrorCode::from(SNAPSHOT_EXECUTOR_UNAVAILABLE),
            mismatches,
        }
    }
}

fn compare_map<K, V>(
    mismatches: &mut Vec<SnapshotContractMismatch>,
    kind: SnapshotContractMismatchKind,
    required: &BTreeMap<K, V>,
    available: &BTreeMap<K, V>,
) where
    K: Ord + AsRef<str>,
    V: PartialEq + serde::Serialize,
{
    for (key, expected) in required {
        let Some(actual) = available.get(key) else {
            mismatch(
                mismatches,
                kind,
                key.as_ref(),
                canonical(expected),
                "missing",
            );
            continue;
        };
        if actual != expected {
            mismatch(
                mismatches,
                kind,
                key.as_ref(),
                canonical(expected),
                canonical(actual),
            );
        }
    }
}

fn mismatch(
    mismatches: &mut Vec<SnapshotContractMismatch>,
    kind: SnapshotContractMismatchKind,
    subject: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
) {
    let actual = actual.into();
    mismatches.push(SnapshotContractMismatch {
        kind,
        subject: subject.into(),
        expected: expected.into(),
        actual: (!actual.is_empty()).then_some(actual),
    });
}

fn canonical<T: serde::Serialize>(value: &T) -> String {
    nomifun_agent_contracts::canonical_json_bytes(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| "<unserializable>".to_owned())
}
