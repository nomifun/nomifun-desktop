use std::fmt::Debug;
use std::path::{Component, Path, PathBuf};

use nomifun_agent_contracts::{
    CanonicalErrorCode, CheckpointDiscardReason, CheckpointRehydrateSource, DigestHex,
    RuntimeCheckpointBinding, RuntimeCheckpointValidationInput, RuntimeCheckpointValidationResult,
    SnapshotCompatibilityAdmissionInput, SnapshotCompatibilityAdmissionResult,
    SnapshotContractMismatch, SnapshotContractMismatchKind, SNAPSHOT_EXECUTOR_UNAVAILABLE,
};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::error::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointArtifactObservation {
    Missing,
    Present { digest: DigestHex },
    Corrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointDisposition {
    Resume(RuntimeCheckpointBinding),
    Discard {
        reasons: Vec<CheckpointDiscardReason>,
        rehydrate_from: Vec<CheckpointRehydrateSource>,
        checkpoint_converter_allowed: bool,
    },
}

#[derive(Clone, Debug)]
pub struct RuntimeCheckpointCache {
    root: PathBuf,
}

impl RuntimeCheckpointCache {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(RuntimeError::Protocol(
                "runtime checkpoint root must be absolute".to_owned(),
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn observe(
        &self,
        binding: &RuntimeCheckpointBinding,
    ) -> Result<CheckpointArtifactObservation, RuntimeError> {
        let path = match self.resolve_existing(binding).await? {
            Some(path) => path,
            None => return Ok(CheckpointArtifactObservation::Missing),
        };
        let mut file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CheckpointArtifactObservation::Missing);
            }
            Err(error) => return Err(RuntimeError::Process(error)),
        };
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        Ok(CheckpointArtifactObservation::Present {
            digest: DigestHex::from(hex_lower(&digest.finalize())),
        })
    }

    pub async fn discard(&self, binding: &RuntimeCheckpointBinding) -> Result<(), RuntimeError> {
        let Some(path) = self.resolve_existing(binding).await? else {
            return Ok(());
        };
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RuntimeError::Process(error)),
        }
    }

    pub async fn validate_or_discard(
        &self,
        input: &RuntimeCheckpointValidationInput,
    ) -> Result<CheckpointDisposition, RuntimeError> {
        let observation = self.observe(&input.checkpoint).await?;
        let disposition = checkpoint_disposition(input, observation);
        if matches!(disposition, CheckpointDisposition::Discard { .. }) {
            self.discard(&input.checkpoint).await?;
        }
        Ok(disposition)
    }

    fn resolve(&self, binding: &RuntimeCheckpointBinding) -> Result<PathBuf, RuntimeError> {
        let relative = Path::new(&binding.locator.normalized_relative_path);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(is_unsafe_component)
        {
            return Err(RuntimeError::Protocol(
                "checkpoint locator must be a normalized relative path".to_owned(),
            ));
        }
        Ok(self.root.join(relative))
    }

    async fn resolve_existing(
        &self,
        binding: &RuntimeCheckpointBinding,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        let lexical = self.resolve(binding)?;
        let canonical_root = tokio::fs::canonicalize(&self.root).await?;
        let canonical = match tokio::fs::canonicalize(&lexical).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RuntimeError::Process(error)),
        };
        if !canonical.starts_with(&canonical_root) {
            return Err(RuntimeError::Protocol(
                "checkpoint locator escapes the runtime checkpoint root".to_owned(),
            ));
        }
        Ok(Some(canonical))
    }
}

pub fn checkpoint_disposition(
    input: &RuntimeCheckpointValidationInput,
    artifact: CheckpointArtifactObservation,
) -> CheckpointDisposition {
    let reasons = match artifact {
        CheckpointArtifactObservation::Missing => vec![CheckpointDiscardReason::Missing],
        CheckpointArtifactObservation::Corrupt => vec![CheckpointDiscardReason::Corrupt],
        CheckpointArtifactObservation::Present { digest }
            if digest != input.checkpoint.locator.digest =>
        {
            vec![CheckpointDiscardReason::Corrupt]
        }
        CheckpointArtifactObservation::Present { .. } => checkpoint_mismatches(input),
    };

    if reasons.is_empty() {
        CheckpointDisposition::Resume(input.checkpoint.clone())
    } else {
        CheckpointDisposition::Discard {
            reasons,
            rehydrate_from: vec![
                CheckpointRehydrateSource::ExactSnapshot,
                CheckpointRehydrateSource::LatestCompletedCompaction,
                CheckpointRehydrateSource::SubsequentCanonicalEvents,
            ],
            checkpoint_converter_allowed: false,
        }
    }
}

pub fn validate_checkpoint(
    input: &RuntimeCheckpointValidationInput,
) -> RuntimeCheckpointValidationResult {
    let mismatches = checkpoint_mismatches(input);
    if mismatches.is_empty() {
        RuntimeCheckpointValidationResult::ExactMatch
    } else {
        RuntimeCheckpointValidationResult::Mismatch { mismatches }
    }
}

pub fn admit_snapshot_executor(
    input: &SnapshotCompatibilityAdmissionInput,
) -> SnapshotCompatibilityAdmissionResult {
    let required = &input.required_ceiling;
    let available = &input.available_executor;
    let mut mismatches = Vec::new();

    if !available
        .protocol_versions
        .contains(&required.protocol_version)
    {
        mismatches.push(mismatch(
            SnapshotContractMismatchKind::ProtocolVersion,
            "runtime_protocol",
            required.protocol_version.as_ref(),
            None,
        ));
    }
    if !available
        .protocol_schema_digests
        .contains(&required.protocol_schema_digest)
    {
        mismatches.push(mismatch(
            SnapshotContractMismatchKind::ProtocolSchema,
            "runtime_protocol_schema",
            required.protocol_schema_digest.as_ref(),
            None,
        ));
    }
    if !available
        .profile_digests
        .get(&required.profile_kind)
        .is_some_and(|digests| digests.contains(&required.profile_digest))
    {
        mismatches.push(mismatch(
            SnapshotContractMismatchKind::RuntimeProfile,
            format!("{:?}", required.profile_kind),
            required.profile_digest.as_ref(),
            None,
        ));
    }

    for feature in &required.native_features {
        if !available.native_features.contains(feature) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::NativeFeature,
                feature.as_ref(),
                "required",
                None,
            ));
        }
    }
    for action in &required.native_actions {
        if !available.native_actions.contains(action) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::NativeAction,
                action.as_ref(),
                "required",
                None,
            ));
        }
    }

    for (id, contract) in &required.initial_capabilities {
        if available.capabilities.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::InitialCapability,
                id.as_ref(),
                render(contract),
                available.capabilities.get(id).map(render),
            ));
        }
    }
    for (id, contract) in &required.on_demand_capabilities {
        if available.capabilities.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::OnDemandCapability,
                id.as_ref(),
                render(contract),
                available.capabilities.get(id).map(render),
            ));
        }
    }
    for (id, contract) in &required.packages {
        if available.packages.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::Package,
                id.as_ref(),
                render(contract),
                available.packages.get(id).map(render),
            ));
        }
    }
    for (id, contract) in &required.skills {
        if available.skills.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::Skill,
                id.as_ref(),
                render(contract),
                available.skills.get(id).map(render),
            ));
        }
    }
    for (id, contract) in &required.mcp_tools {
        if available.mcp_tools.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::McpTool,
                id.as_ref(),
                render(contract),
                available.mcp_tools.get(id).map(render),
            ));
        }
    }
    for (id, contract) in &required.model_routes {
        if available.model_routes.get(id) != Some(contract) {
            mismatches.push(mismatch(
                SnapshotContractMismatchKind::ModelRoute,
                id.as_ref(),
                render(contract),
                available.model_routes.get(id).map(render),
            ));
        }
    }
    if !available
        .typed_resource_contract_digests
        .contains(&required.typed_resource_contract_digest)
    {
        mismatches.push(mismatch(
            SnapshotContractMismatchKind::TypedResourceContract,
            "typed_resource_bindings",
            required.typed_resource_contract_digest.as_ref(),
            None,
        ));
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

#[cfg(windows)]
fn is_unsafe_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::Prefix(_)
            | Component::RootDir
            | Component::ParentDir
            | Component::CurDir
    )
}

#[cfg(not(windows))]
fn is_unsafe_component(component: Component<'_>) -> bool {
    matches!(
        component,
        Component::RootDir | Component::ParentDir | Component::CurDir
    )
}

fn checkpoint_mismatches(
    input: &RuntimeCheckpointValidationInput,
) -> Vec<CheckpointDiscardReason> {
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
    mismatches
}

fn mismatch(
    kind: SnapshotContractMismatchKind,
    subject: impl Into<String>,
    expected: impl Into<String>,
    actual: Option<String>,
) -> SnapshotContractMismatch {
    SnapshotContractMismatch {
        kind,
        subject: subject.into(),
        expected: expected.into(),
        actual,
    }
}

fn render(value: &impl Debug) -> String {
    format!("{value:?}")
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::RuntimeCheckpointMismatchFixture;

    use super::*;

    #[test]
    fn frozen_checkpoint_mismatch_fixture_is_reproduced_exactly() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/checkpoint-mismatch.json"
        );
        let fixture: RuntimeCheckpointMismatchFixture =
            serde_json::from_str(fixture).unwrap();
        assert_eq!(validate_checkpoint(&fixture.input), fixture.result);
    }

    #[test]
    fn missing_and_corrupt_artifacts_discard_without_converter() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/checkpoint-mismatch.json"
        );
        let fixture: RuntimeCheckpointMismatchFixture =
            serde_json::from_str(fixture).unwrap();

        for observation in [
            CheckpointArtifactObservation::Missing,
            CheckpointArtifactObservation::Corrupt,
        ] {
            let CheckpointDisposition::Discard {
                checkpoint_converter_allowed,
                rehydrate_from,
                ..
            } = checkpoint_disposition(&fixture.input, observation)
            else {
                panic!("checkpoint must be discarded");
            };
            assert!(!checkpoint_converter_allowed);
            assert_eq!(
                rehydrate_from,
                vec![
                    CheckpointRehydrateSource::ExactSnapshot,
                    CheckpointRehydrateSource::LatestCompletedCompaction,
                    CheckpointRehydrateSource::SubsequentCanonicalEvents,
                ]
            );
        }
    }

    #[tokio::test]
    async fn cache_discards_digest_mismatch_and_rejects_path_escape() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/checkpoint-mismatch.json"
        );
        let mut fixture: RuntimeCheckpointMismatchFixture =
            serde_json::from_str(fixture).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let cache = RuntimeCheckpointCache::new(directory.path()).unwrap();
        let checkpoint_path = directory
            .path()
            .join(&fixture.input.checkpoint.locator.normalized_relative_path);
        tokio::fs::create_dir_all(checkpoint_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&checkpoint_path, b"corrupt-checkpoint")
            .await
            .unwrap();

        let disposition = cache.validate_or_discard(&fixture.input).await.unwrap();
        assert!(matches!(
            disposition,
            CheckpointDisposition::Discard {
                reasons,
                checkpoint_converter_allowed: false,
                ..
            } if reasons == vec![CheckpointDiscardReason::Corrupt]
        ));
        assert!(!checkpoint_path.exists());

        fixture.input.checkpoint.locator.normalized_relative_path =
            "../outside.json".to_owned();
        assert!(cache.observe(&fixture.input.checkpoint).await.is_err());
    }
}
