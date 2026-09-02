use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, CodexPinnedSource, DigestHex, FullAutoExecutionWire, RuntimeHelloPayload,
    RuntimeProfileKind, RuntimeRpcAllowlist, RuntimeTarget, VersionString, digest_payload,
};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

pub const FROZEN_CODEX_COMMIT: &str = "dc2ccc6843abb09c9d297862dc10b6bd12a3935d";
pub const FROZEN_PROTOCOL_VERSION: &str = "1.0.0";
pub const FROZEN_PROTOCOL_SCHEMA_DIGEST: &str =
    "f1c0422f04c9de923e18c7df40d814d3c9f5b2db5f1c5fef2745e77e6d62590f";

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContractPayload {
    pub contract_version: VersionString,
    pub pinned_source: CodexPinnedSource,
    pub fork_commit: String,
    pub tracked_upstream_commit: String,
    pub protocol_version: VersionString,
    pub protocol_schema_digest: DigestHex,
    pub rpc_allowlist: RuntimeRpcAllowlist,
    pub full_auto: FullAutoExecutionWire,
    pub supported_profiles: BTreeSet<RuntimeProfileKind>,
    pub runtime_targets: BTreeMap<String, RuntimeTarget>,
}

impl RuntimeContractPayload {
    fn pinned() -> Self {
        Self {
            contract_version: VersionString::from("1.0.0"),
            pinned_source: CodexPinnedSource::frozen_investigation_baseline(),
            fork_commit: FROZEN_CODEX_COMMIT.to_owned(),
            tracked_upstream_commit: FROZEN_CODEX_COMMIT.to_owned(),
            protocol_version: VersionString::from(FROZEN_PROTOCOL_VERSION),
            protocol_schema_digest: DigestHex::from(FROZEN_PROTOCOL_SCHEMA_DIGEST),
            rpc_allowlist: RuntimeRpcAllowlist::frozen(),
            full_auto: FullAutoExecutionWire::fixed(),
            supported_profiles: BTreeSet::from([
                RuntimeProfileKind::CodingNative,
                RuntimeProfileKind::ManagedMinimal,
            ]),
            runtime_targets: BTreeMap::from([
                (
                    "linux_desktop_x64".to_owned(),
                    RuntimeTarget::from("x86_64-unknown-linux-musl"),
                ),
                (
                    "linux_headless_x64".to_owned(),
                    RuntimeTarget::from("x86_64-unknown-linux-musl"),
                ),
                (
                    "macos_desktop_arm64".to_owned(),
                    RuntimeTarget::from("aarch64-apple-darwin"),
                ),
                (
                    "macos_desktop_x64".to_owned(),
                    RuntimeTarget::from("x86_64-apple-darwin"),
                ),
                (
                    "windows_desktop_x64".to_owned(),
                    RuntimeTarget::from("x86_64-pc-windows-msvc"),
                ),
            ]),
        }
    }
}

/// Source-controlled Runtime protocol contract.
///
/// This descriptor deliberately contains no Host, Sidecar, helper, package, or
/// legal artifact digest. Packaging and native validation record real artifact
/// digests in an external post-build `release-lock.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeReleaseDescriptor {
    pub contract: RuntimeContractPayload,
    pub contract_digest: DigestHex,
}

impl RuntimeReleaseDescriptor {
    pub fn pinned_contract() -> Result<Self, RuntimeError> {
        Self::from_contract(RuntimeContractPayload::pinned())
    }

    pub fn from_contract(contract: RuntimeContractPayload) -> Result<Self, RuntimeError> {
        validate_contract(&contract)?;
        let contract_digest = digest_payload(&contract)
            .map_err(|error| RuntimeError::ReleaseManifest(error.to_string()))?;
        Ok(Self {
            contract,
            contract_digest,
        })
    }

    pub fn pinned_source(&self) -> CodexPinnedSource {
        self.contract.pinned_source.clone()
    }

    pub fn expected_profiles(&self) -> BTreeSet<RuntimeProfileKind> {
        self.contract.supported_profiles.clone()
    }

    pub fn expected_full_auto(&self) -> FullAutoExecutionWire {
        self.contract.full_auto.clone()
    }

    pub fn runtime_target_for_target(
        &self,
        target_id: &str,
    ) -> Result<RuntimeTarget, RuntimeError> {
        self.contract
            .runtime_targets
            .get(target_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::ReleaseManifest(format!(
                    "target {target_id:?} is not supported by the pinned Runtime contract"
                ))
            })
    }

    pub fn hello_expectation(
        &self,
        runtime_build_digest: DigestHex,
        runtime_target: RuntimeTarget,
        native_features: BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
        native_actions: BTreeSet<ActionId>,
    ) -> RuntimeHelloExpectation {
        RuntimeHelloExpectation {
            runtime_release_digest: self.contract_digest.clone(),
            runtime_build_digest,
            fork_commit: self.contract.fork_commit.clone(),
            tracked_upstream_commit: self.contract.tracked_upstream_commit.clone(),
            protocol_version: self.contract.protocol_version.clone(),
            protocol_schema_digest: self.contract.protocol_schema_digest.clone(),
            runtime_target,
            supported_profiles: self.contract.supported_profiles.clone(),
            native_features,
            native_actions,
            full_auto: self.contract.full_auto.clone(),
            rpc_allowlist: self.contract.rpc_allowlist.clone(),
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

fn validate_contract(contract: &RuntimeContractPayload) -> Result<(), RuntimeError> {
    contract
        .pinned_source
        .validate_frozen_investigation_baseline()
        .map_err(|error| RuntimeError::ReleaseManifest(error.message))?;
    if contract.contract_version.as_ref() != "1.0.0"
        || contract.fork_commit != FROZEN_CODEX_COMMIT
        || contract.tracked_upstream_commit != FROZEN_CODEX_COMMIT
        || contract.protocol_version.as_ref() != FROZEN_PROTOCOL_VERSION
        || contract.protocol_schema_digest.as_ref() != FROZEN_PROTOCOL_SCHEMA_DIGEST
        || contract.full_auto != FullAutoExecutionWire::fixed()
        || contract.rpc_allowlist != RuntimeRpcAllowlist::frozen()
        || contract.supported_profiles
            != BTreeSet::from([
                RuntimeProfileKind::CodingNative,
                RuntimeProfileKind::ManagedMinimal,
            ])
    {
        return Err(RuntimeError::ReleaseManifest(
            "Runtime contract differs from the pinned source and protocol contract".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_contract_has_no_artifact_digest_source() {
        let descriptor = RuntimeReleaseDescriptor::pinned_contract().unwrap();
        assert_eq!(
            descriptor.contract.pinned_source.pinned_commit,
            FROZEN_CODEX_COMMIT
        );
        assert_eq!(
            descriptor.contract.rpc_allowlist,
            RuntimeRpcAllowlist::frozen()
        );
        assert_eq!(
            descriptor.contract.full_auto,
            FullAutoExecutionWire::fixed()
        );
        assert!(is_lower_hex(descriptor.contract_digest.as_ref(), 64));
        assert!(
            descriptor
                .runtime_target_for_target("windows_desktop_x64")
                .is_ok()
        );
        assert!(
            descriptor
                .runtime_target_for_target("windows_arm64")
                .is_err()
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
