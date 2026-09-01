use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, CodexPinnedSource, CodexRuntimeReleaseManifestPayload,
    DigestHex, FullAutoExecutionWire, RuntimeHelloPayload, RuntimeProfileKind,
    RuntimeReleaseTargetPayload, RuntimeRpcAllowlist, RuntimeTarget, VersionString,
    digest_payload,
};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

pub const FROZEN_CODEX_COMMIT: &str = "dc2ccc6843abb09c9d297862dc10b6bd12a3935d";
pub const FROZEN_PROTOCOL_VERSION: &str = "1.0.0";
pub const FROZEN_PROTOCOL_SCHEMA_DIGEST: &str =
    "f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f";
pub const FROZEN_RUNTIME_RELEASE_DIGEST: &str =
    "b9dce00732f6d1c45cb20fc30e7a286518d505d7faeb2d94b6cc70d9e107289d";

pub const RUNTIME_HELLO_METHOD: &str = "runtime/hello";
pub const RPC_METHODS: [&str; 8] = [
    "create",
    "resume",
    "fork",
    "start_turn",
    "steer",
    "follow_up",
    "cancel",
    "session_dispose",
];

pub const FROZEN_RELEASE_INPUT_JSON: &str =
    include_str!("../../../../vendor/codex-runtime/release-input.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReleaseDescriptor {
    pub payload: CodexRuntimeReleaseManifestPayload,
    pub payload_digest: DigestHex,
}

impl RuntimeReleaseDescriptor {
    pub fn from_payload(payload: CodexRuntimeReleaseManifestPayload) -> Result<Self, RuntimeError> {
        validate_release_payload(&payload)?;
        let payload_digest = digest_payload(&payload)
            .map_err(|error| RuntimeError::ReleaseManifest(error.to_string()))?;
        Ok(Self {
            payload,
            payload_digest,
        })
    }

    pub fn from_envelope(
        envelope: ArtifactEnvelope<CodexRuntimeReleaseManifestPayload>,
    ) -> Result<Self, RuntimeError> {
        if envelope.digest_algorithm != "sorted-json-sha256-v1" {
            return Err(RuntimeError::ReleaseManifest(
                "unsupported release digest algorithm".to_owned(),
            ));
        }
        if !envelope
            .verify()
            .map_err(|error| RuntimeError::ReleaseManifest(error.to_string()))?
        {
            return Err(RuntimeError::ReleaseManifest(
                "release envelope payload digest does not verify".to_owned(),
            ));
        }
        let descriptor = Self::from_payload(envelope.payload)?;
        if descriptor.payload_digest != envelope.payload_digest {
            return Err(RuntimeError::ReleaseManifest(
                "release envelope digest does not match its payload".to_owned(),
            ));
        }
        Ok(descriptor)
    }

    pub fn frozen_from_fixture() -> Result<Self, RuntimeError> {
        let payload = serde_json::from_str::<CodexRuntimeReleaseManifestPayload>(
            FROZEN_RELEASE_INPUT_JSON,
        )?;
        Self::from_payload(payload)
    }

    pub fn pinned_source(&self) -> CodexPinnedSource {
        self.payload.pinned_source.clone()
    }

    pub fn expected_profiles(&self) -> BTreeSet<RuntimeProfileKind> {
        self.payload.supported_profiles.clone()
    }

    pub fn expected_full_auto(&self) -> FullAutoExecutionWire {
        self.payload.full_auto.clone()
    }

    pub fn sidecar_digest_for_target(&self, target_id: &str) -> Result<DigestHex, RuntimeError> {
        match self.payload.target_matrix.get(target_id) {
            Some(RuntimeReleaseTargetPayload::Required {
                sidecar_artifact,
                ..
            }) => Ok(sidecar_artifact.digest.clone()),
            Some(_) => Err(RuntimeError::ReleaseManifest(format!(
                "target {target_id:?} has no local runtime artifact"
            ))),
            None => Err(RuntimeError::ReleaseManifest(format!(
                "target {target_id:?} is absent from the release matrix"
            ))),
        }
    }

    pub fn hello_expectation(
        &self,
        runtime_build_digest: DigestHex,
        runtime_target: RuntimeTarget,
        native_features: BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
        native_actions: BTreeSet<ActionId>,
    ) -> RuntimeHelloExpectation {
        RuntimeHelloExpectation {
            runtime_release_digest: self.payload_digest.clone(),
            runtime_build_digest,
            fork_commit: self.payload.fork_commit.clone(),
            tracked_upstream_commit: self.payload.tracked_upstream_commit.clone(),
            protocol_version: self.payload.protocol_version.clone(),
            protocol_schema_digest: self.payload.protocol_schema_digest.clone(),
            runtime_target,
            supported_profiles: self.payload.supported_profiles.clone(),
            native_features,
            native_actions,
            full_auto: self.payload.full_auto.clone(),
            rpc_allowlist: self.payload.rpc_allowlist.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHelloExpectation {
    pub runtime_release_digest: DigestHex,
    pub runtime_build_digest: DigestHex,
    pub fork_commit: String,
    pub tracked_upstream_commit: String,
    pub protocol_version: VersionString,
    pub protocol_schema_digest: DigestHex,
    pub runtime_target: RuntimeTarget,
    pub supported_profiles: BTreeSet<RuntimeProfileKind>,
    pub native_features: BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub full_auto: FullAutoExecutionWire,
    pub rpc_allowlist: RuntimeRpcAllowlist,
}

impl RuntimeHelloExpectation {
    pub fn from_payload(payload: RuntimeHelloPayload) -> Self {
        Self {
            runtime_release_digest: payload.runtime_release_digest,
            runtime_build_digest: payload.runtime_build_digest,
            fork_commit: payload.fork_commit,
            tracked_upstream_commit: payload.tracked_upstream_commit,
            protocol_version: payload.protocol_version,
            protocol_schema_digest: payload.protocol_schema_digest,
            runtime_target: payload.runtime_target,
            supported_profiles: payload.supported_profiles,
            native_features: payload.native_features,
            native_actions: payload.native_actions,
            full_auto: payload.full_auto,
            rpc_allowlist: payload.rpc_allowlist,
        }
    }

    pub fn validate(&self, actual: &RuntimeHelloPayload) -> Result<(), RuntimeError> {
        let exact_profiles = BTreeSet::from([
            RuntimeProfileKind::CodingNative,
            RuntimeProfileKind::ManagedMinimal,
        ]);
        if actual.fork_commit != FROZEN_CODEX_COMMIT
            || actual.tracked_upstream_commit != FROZEN_CODEX_COMMIT
            || actual.supported_profiles != exact_profiles
            || actual.full_auto != FullAutoExecutionWire::fixed()
            || actual.rpc_allowlist != RuntimeRpcAllowlist::frozen()
        {
            return Err(RuntimeError::HelloRejected(
                "hello violates the frozen source, profile, FullAuto, or RPC contract".to_owned(),
            ));
        }
        let actual_digest = digest_payload(actual)
            .map_err(|error| RuntimeError::HelloRejected(error.to_string()))?;
        let expected_digest = digest_payload(&RuntimeHelloPayload {
            runtime_release_digest: self.runtime_release_digest.clone(),
            runtime_build_digest: self.runtime_build_digest.clone(),
            fork_commit: self.fork_commit.clone(),
            tracked_upstream_commit: self.tracked_upstream_commit.clone(),
            protocol_version: self.protocol_version.clone(),
            protocol_schema_digest: self.protocol_schema_digest.clone(),
            runtime_target: self.runtime_target.clone(),
            supported_profiles: self.supported_profiles.clone(),
            native_features: self.native_features.clone(),
            native_actions: self.native_actions.clone(),
            full_auto: self.full_auto.clone(),
            rpc_allowlist: self.rpc_allowlist.clone(),
        })
        .map_err(|error| RuntimeError::HelloRejected(error.to_string()))?;

        if actual_digest != expected_digest {
            return Err(RuntimeError::HelloRejected(
                "hello payload differs from the pinned expectation".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_release_payload(
    payload: &CodexRuntimeReleaseManifestPayload,
) -> Result<(), RuntimeError> {
    payload
        .validate()
        .map_err(|error| RuntimeError::ReleaseManifest(error.message))?;

    if payload.pinned_source.repository_alias != "../codex"
        || payload.pinned_source.pinned_commit != FROZEN_CODEX_COMMIT
        || payload.fork_commit != FROZEN_CODEX_COMMIT
        || payload.tracked_upstream_commit != FROZEN_CODEX_COMMIT
    {
        return Err(RuntimeError::ReleaseManifest(
            "release must pin the frozen Codex fork and tracked upstream commit".to_owned(),
        ));
    }
    if payload.protocol_version.as_ref() != FROZEN_PROTOCOL_VERSION {
        return Err(RuntimeError::ReleaseManifest(
            "release protocol version is not the frozen version".to_owned(),
        ));
    }
    if payload.protocol_schema_digest.as_ref() != FROZEN_PROTOCOL_SCHEMA_DIGEST {
        return Err(RuntimeError::ReleaseManifest(
            "release protocol schema digest is not the frozen digest".to_owned(),
        ));
    }
    if payload.full_auto != FullAutoExecutionWire::fixed()
        || payload.rpc_allowlist != RuntimeRpcAllowlist::frozen()
    {
        return Err(RuntimeError::ReleaseManifest(
            "release must use the fixed FullAuto and RPC contracts".to_owned(),
        ));
    }
    if payload.supported_profiles
        != BTreeSet::from([
            RuntimeProfileKind::CodingNative,
            RuntimeProfileKind::ManagedMinimal,
        ])
    {
        return Err(RuntimeError::ReleaseManifest(
            "release must expose exactly the two supported runtime profiles".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_release_input_is_the_frozen_contract() {
        let descriptor = RuntimeReleaseDescriptor::frozen_from_fixture().unwrap();
        assert_eq!(
            descriptor.payload.pinned_source.pinned_commit,
            FROZEN_CODEX_COMMIT
        );
        assert_eq!(
            descriptor.payload.rpc_allowlist,
            RuntimeRpcAllowlist::frozen()
        );
        assert_eq!(
            descriptor.payload.full_auto,
            FullAutoExecutionWire::fixed()
        );
        assert_eq!(
            descriptor.payload_digest.as_ref(),
            FROZEN_RUNTIME_RELEASE_DIGEST
        );
        assert!(
            descriptor
                .sidecar_digest_for_target("windows_desktop_x64")
                .is_ok()
        );
        assert!(
            descriptor
                .sidecar_digest_for_target("windows_arm64")
                .is_err()
        );
        assert_eq!(
            descriptor
                .sidecar_digest_for_target("macos_desktop_arm64")
                .unwrap()
                .as_ref(),
            "7863db3a77545eec8966483f26fb5b493aea6e285ac35b5c29d0920342438060"
        );
        assert_eq!(
            descriptor
                .sidecar_digest_for_target("macos_desktop_x64")
                .unwrap()
                .as_ref(),
            "aa072d7500db44f6f905d9208551c3ee9c1cd37dabaffff0ab69d503c2a18446"
        );
    }

    #[test]
    fn hello_cannot_widen_profiles_or_rpc_methods() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/hello-rpc-allowlist.json"
        );
        let hello: RuntimeHelloPayload = serde_json::from_str(fixture).unwrap();
        let expectation = RuntimeHelloExpectation::from_payload(hello.clone());
        expectation.validate(&hello).unwrap();

        let mut widened = hello;
        widened
            .rpc_allowlist
            .experimental_methods
            .insert("thread/search".to_owned());
        assert!(expectation.validate(&widened).is_err());
    }
}
