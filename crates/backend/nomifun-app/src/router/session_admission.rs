//! One typed read of the frozen binding proof on the canonical Turn claim.
//! Permanent input content is separate; older claims remain readable without
//! inventing the resource version that was not recorded at their admission.

use nomifun_agent_contracts::{AgentHandoffBindingRefV1, ResolvedSnapshotRef};
use nomifun_common::AppError;
use serde_json::Value;

pub(crate) fn canonical_admission_binding_ref(
    started: &Value,
) -> Result<Option<AgentHandoffBindingRefV1>, AppError> {
    let invalid =
        |detail: String| AppError::Conflict(format!("Canonical Turn admission: {detail}"));
    let admission = started
        .get("admission")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("frozen claim admission is missing".into()))?;
    let snapshot: ResolvedSnapshotRef = serde_json::from_value(
        admission
            .get("resolved_snapshot_ref")
            .cloned()
            .ok_or_else(|| invalid("frozen Snapshot reference is missing".into()))?,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let Some(binding) = admission.get("binding_ref") else {
        return Ok(None);
    };
    let binding: AgentHandoffBindingRefV1 =
        serde_json::from_value(binding.clone()).map_err(|error| invalid(error.to_string()))?;
    if binding.binding_version == 0 || binding.resolved_snapshot_ref != snapshot {
        return Err(invalid(
            "binding proof differs from its admitted Snapshot or has no resource version".into(),
        ));
    }
    Ok(Some(binding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn full_binding_claim_is_single_typed_proof_and_old_claims_do_not_invent_versions() {
        let binding = AgentHandoffBindingRefV1 {
            preset_revision_ref: nomifun_agent_contracts::PresetRevisionRef {
                preset_id: "preset".into(),
                revision: 1,
                revision_digest: "a".repeat(64).into(),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot".into(),
                snapshot_digest: "b".repeat(64).into(),
            },
            binding_version: 4,
        };
        let valid = json!({"admission":{"resolved_snapshot_ref":binding.resolved_snapshot_ref,"binding_ref":binding}});
        assert_eq!(
            canonical_admission_binding_ref(&valid).unwrap(),
            Some(binding.clone())
        );
        let mut mismatch = valid.clone();
        mismatch["admission"]["resolved_snapshot_ref"]["snapshot_id"] = json!("another");
        assert!(canonical_admission_binding_ref(&mismatch).is_err());
        let mut zero = valid;
        zero["admission"]["binding_ref"]["binding_version"] = json!(0);
        assert!(canonical_admission_binding_ref(&zero).is_err());
        assert!(
            canonical_admission_binding_ref(
                &json!({"admission":{"resolved_snapshot_ref":binding.resolved_snapshot_ref}})
            )
            .unwrap()
            .is_none()
        );
    }
}
