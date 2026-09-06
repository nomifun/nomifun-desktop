//! Disposable resume checkpoint contracts.

use serde::{Deserialize, Serialize};

use crate::engine::EngineBinding;
use crate::error::CodingEngineError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCheckpoint {
    pub agent_session_id: String,
    pub runtime_binding_id: String,
    pub engine_family_id: String,
    pub engine_build_id: String,
    pub engine_build_digest: String,
    pub snapshot_id: String,
    pub snapshot_digest: String,
    pub last_event_cursor: u64,
    pub partial_output: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointAdmission {
    Reusable,
    Discarded { reason: CheckpointDiscardReason },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointDiscardReason {
    SessionMismatch,
    BindingMismatch,
    EngineBuildMismatch,
    SnapshotMismatch,
    CursorAhead,
    InvalidIdentity,
}

impl CodingCheckpoint {
    pub fn from_binding(
        binding: &EngineBinding,
        last_event_cursor: u64,
        partial_output: impl Into<String>,
    ) -> Self {
        Self {
            agent_session_id: binding.agent_session_id().as_ref().to_owned(),
            runtime_binding_id: binding.runtime_binding_id().as_ref().to_owned(),
            engine_family_id: binding.family_id().as_ref().to_owned(),
            engine_build_id: binding.build_id().as_ref().to_owned(),
            engine_build_digest: binding.build_digest().as_ref().to_owned(),
            snapshot_id: binding
                .resolved_snapshot_ref()
                .snapshot_id
                .as_ref()
                .to_owned(),
            snapshot_digest: binding
                .resolved_snapshot_ref()
                .snapshot_digest
                .as_ref()
                .to_owned(),
            last_event_cursor,
            partial_output: partial_output.into(),
        }
    }

    pub fn admit(
        &self,
        binding: &EngineBinding,
        current_event_cursor: u64,
    ) -> CheckpointAdmission {
        if self.agent_session_id != binding.agent_session_id().as_ref() {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::SessionMismatch,
            };
        }
        if self.runtime_binding_id != binding.runtime_binding_id().as_ref() {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::BindingMismatch,
            };
        }
        if self.engine_family_id != binding.family_id().as_ref()
            || self.engine_build_id != binding.build_id().as_ref()
            || self.engine_build_digest != binding.build_digest().as_ref()
        {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::EngineBuildMismatch,
            };
        }
        if self.snapshot_id != binding.resolved_snapshot_ref().snapshot_id.as_ref()
            || self.snapshot_digest
                != binding.resolved_snapshot_ref().snapshot_digest.as_ref()
        {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::SnapshotMismatch,
            };
        }
        if self.last_event_cursor > current_event_cursor {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::CursorAhead,
            };
        }
        if self.agent_session_id.trim().is_empty()
            || self.runtime_binding_id.trim().is_empty()
            || self.engine_family_id.trim().is_empty()
            || self.engine_build_id.trim().is_empty()
            || !is_digest(&self.engine_build_digest)
            || !is_digest(&self.snapshot_digest)
        {
            return CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::InvalidIdentity,
            };
        }
        CheckpointAdmission::Reusable
    }

    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if self.agent_session_id.trim().is_empty()
            || self.runtime_binding_id.trim().is_empty()
            || self.engine_family_id.trim().is_empty()
            || self.engine_build_id.trim().is_empty()
            || !is_digest(&self.engine_build_digest)
            || !is_digest(&self.snapshot_digest)
        {
            return Err(CodingEngineError::Checkpoint(
                "checkpoint identity is incomplete".to_owned(),
            ));
        }
        Ok(())
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentSessionId, DigestHex, ResolvedSnapshotId, ResolvedSnapshotRef, RuntimeBindingId,
    };
    use crate::engine::{
        CodingEngine, CodingEngineBuild, CodingRuntimeProfile, EngineBuildId, EngineFamilyId,
    };

    pub(crate) fn binding() -> EngineBinding {
        CodingEngine::new(CodingEngineBuild {
            family_id: EngineFamilyId::from("nomifun.coding"),
            build_id: EngineBuildId::from("build-1"),
            build_digest: DigestHex::from("a".repeat(64)),
            display_name: "Coding".to_owned(),
            supported_profiles: vec![CodingRuntimeProfile::Coding],
        })
        .unwrap()
        .bind(
            AgentSessionId::from("session"),
            RuntimeBindingId::from("binding"),
            CodingRuntimeProfile::Coding,
            ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
        )
        .unwrap()
    }

    #[test]
    fn checkpoint_is_reusable_only_for_the_same_exact_binding_and_cursor() {
        let binding = binding();
        let checkpoint = CodingCheckpoint::from_binding(&binding, 4, "partial");
        assert_eq!(
            checkpoint.admit(&binding, 4),
            CheckpointAdmission::Reusable
        );
        assert_eq!(
            checkpoint.admit(&binding, 3),
            CheckpointAdmission::Discarded {
                reason: CheckpointDiscardReason::CursorAhead
            }
        );
    }
}
