//! Canonical platform Capability Catalog contracts.
//!
//! A catalog entry describes a formally materialized contribution. Consumer
//! compatibility and current availability are separate facts: a contribution
//! may be understandable by a consumer while unavailable for that consumer.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::digest::digest_payload;
use crate::package::{
    CapabilityConsumer, CapabilityKind, CapabilityManifest, PackageRef,
};
use crate::{
    ArtifactEnvelope, CapabilityRef, ContributionId, ContributionLock,
    ContributionSourceKind, DigestHex, McpBindingId, MiniAppId, PluginMountId,
    ResourceKind, RuntimeFeatureId, StableSourceIdentity,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "owner_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityOwner {
    Package { package: PackageRef },
    BusinessDomain { domain_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProvenance {
    pub owner: CapabilityOwner,
    pub source_kind: ContributionSourceKind,
    pub source_identity: StableSourceIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<PluginMountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miniapp_id: Option<MiniAppId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_binding_id: Option<McpBindingId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<DigestHex>,
}

impl CapabilityProvenance {
    pub fn validate(&self) -> Result<(), CapabilityCatalogContractError> {
        validate_non_empty(&self.source_identity.0, "provenance.source_identity")?;
        match &self.owner {
            CapabilityOwner::Package { package } => {
                validate_non_empty(package.id.as_ref(), "provenance.owner.package.id")?;
                validate_non_empty(
                    package.version.as_ref(),
                    "provenance.owner.package.version",
                )?;
            }
            CapabilityOwner::BusinessDomain { domain_id } => {
                validate_non_empty(domain_id, "provenance.owner.domain_id")?;
            }
        }
        if let Some(digest) = &self.artifact_digest {
            validate_digest(digest, "provenance.artifact_digest")?;
        }

        let valid_source_identity = match self.source_kind {
            ContributionSourceKind::PlatformBuiltin => {
                self.mount_id.is_none()
                    && self.miniapp_id.is_none()
                    && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::PluginMount => {
                self.mount_id.is_some()
                    && self.miniapp_id.is_none()
                    && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::MiniAppActiveRelease => {
                self.miniapp_id.is_some() && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::McpBinding => {
                self.mcp_binding_id.is_some() && self.miniapp_id.is_none()
            }
        };
        if !valid_source_identity {
            return Err(CapabilityCatalogContractError::InvalidField {
                field: "provenance",
                reason: format!(
                    "source-specific identity is invalid for {:?}",
                    self.source_kind
                ),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogAvailability {
    Active,
    Unavailable { reason: String },
    Disabled { reason: String },
    NeedsRuntime {
        missing_runtime_features: BTreeSet<RuntimeFeatureId>,
    },
    ContractMismatch {
        expected_contract_digest: DigestHex,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actual_contract_digest: Option<DigestHex>,
    },
}

impl CatalogAvailability {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReleaseState {
    #[serde(rename = "published_active")]
    PublishedActive,
    ReadyCandidate,
    UnpublishedRelease,
    UnstartedService,
    ProjectSource,
    TestHost,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAdmissionRejection {
    ReadyCandidate,
    UnpublishedRelease,
    UnstartedService,
    ProjectSource,
    TestHost,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogAdmission {
    pub release_state: CapabilityReleaseState,
    pub admitted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<CapabilityAdmissionRejection>,
}

impl CapabilityCatalogAdmission {
    pub fn for_release(release_state: CapabilityReleaseState) -> Self {
        let rejection = match release_state {
            CapabilityReleaseState::PublishedActive => None,
            CapabilityReleaseState::ReadyCandidate => {
                Some(CapabilityAdmissionRejection::ReadyCandidate)
            }
            CapabilityReleaseState::UnpublishedRelease => {
                Some(CapabilityAdmissionRejection::UnpublishedRelease)
            }
            CapabilityReleaseState::UnstartedService => {
                Some(CapabilityAdmissionRejection::UnstartedService)
            }
            CapabilityReleaseState::ProjectSource => {
                Some(CapabilityAdmissionRejection::ProjectSource)
            }
            CapabilityReleaseState::TestHost => Some(CapabilityAdmissionRejection::TestHost),
        };
        Self {
            release_state,
            admitted: rejection.is_none(),
            rejection,
        }
    }

    pub fn admit(
        release_state: CapabilityReleaseState,
    ) -> Result<Self, CapabilityCatalogContractError> {
        let admission = Self::for_release(release_state);
        if admission.admitted {
            Ok(admission)
        } else {
            Err(CapabilityCatalogContractError::CandidateRejected { release_state })
        }
    }

    pub fn validate(&self) -> Result<(), CapabilityCatalogContractError> {
        let expected = Self::for_release(self.release_state);
        if self.admitted != expected.admitted || self.rejection != expected.rejection {
            return Err(CapabilityCatalogContractError::InvalidField {
                field: "admission",
                reason: "admitted and rejection do not match release_state".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogEntry {
    pub capability: CapabilityRef,
    pub contract_digest: DigestHex,
    pub contribution_id: ContributionId,
    pub provenance: CapabilityProvenance,
    pub host_surfaces: BTreeSet<String>,
    pub supported_consumers: BTreeSet<CapabilityConsumer>,
    pub contribution_kind: CapabilityKind,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub typed_resource_kinds: BTreeSet<ResourceKind>,
    pub availability: BTreeMap<CapabilityConsumer, CatalogAvailability>,
    pub admission: CapabilityCatalogAdmission,
}

pub type CapabilityCatalogEntryArtifact = ArtifactEnvelope<CapabilityCatalogEntry>;

impl CapabilityCatalogEntry {
    pub fn validate(&self) -> Result<(), CapabilityCatalogContractError> {
        validate_non_empty(self.capability.id.as_ref(), "capability.id")?;
        validate_non_empty(self.capability.version.as_ref(), "capability.version")?;
        validate_digest(&self.contract_digest, "contract_digest")?;
        validate_non_empty(self.contribution_id.as_ref(), "contribution_id")?;
        self.provenance.validate()?;
        self.admission.validate()?;
        if !self.admission.admitted {
            return Err(CapabilityCatalogContractError::CandidateRejected {
                release_state: self.admission.release_state,
            });
        }
        if self.supported_consumers.is_empty() {
            return Err(CapabilityCatalogContractError::InvalidField {
                field: "supported_consumers",
                reason: "a catalog entry must support at least one consumer".into(),
            });
        }
        let expected = self
            .supported_consumers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let actual = self.availability.keys().copied().collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(CapabilityCatalogContractError::AvailabilityCoverage);
        }
        for (consumer, availability) in &self.availability {
            validate_availability(availability, &self.contract_digest)?;
            if !self.supported_consumers.contains(consumer) {
                return Err(CapabilityCatalogContractError::AvailabilityCoverage);
            }
        }
        Ok(())
    }

    pub fn supports_consumer(&self, consumer: CapabilityConsumer) -> bool {
        self.supported_consumers.contains(&consumer)
    }

    pub fn availability_for(
        &self,
        consumer: CapabilityConsumer,
    ) -> Option<&CatalogAvailability> {
        self.availability.get(&consumer)
    }

    pub fn is_available_for(&self, consumer: CapabilityConsumer) -> bool {
        self.availability_for(consumer)
            .is_some_and(CatalogAvailability::is_active)
    }

    pub fn filter_for_consumer(
        entries: &[Self],
        consumer: CapabilityConsumer,
    ) -> Vec<&Self> {
        entries
            .iter()
            .filter(|entry| entry.supports_consumer(consumer))
            .collect()
    }

    pub fn filter_available_for_consumer(
        entries: &[Self],
        consumer: CapabilityConsumer,
    ) -> Vec<&Self> {
        entries
            .iter()
            .filter(|entry| entry.is_available_for(consumer))
            .collect()
    }

    pub fn digest(&self) -> Result<DigestHex, crate::CanonicalDigestError> {
        digest_payload(self)
    }

    pub fn operation_lock(
        &self,
        consumer: CapabilityConsumer,
    ) -> Result<CapabilityOperationLock, CapabilityCatalogResolveError> {
        if !self.supports_consumer(consumer) {
            return Err(CapabilityCatalogResolveError::UnsupportedConsumer {
                capability: self.capability.clone(),
                consumer,
            });
        }
        let availability = self
            .availability_for(consumer)
            .cloned()
            .ok_or_else(|| CapabilityCatalogResolveError::InvalidEntry {
                capability: self.capability.clone(),
                reason: "consumer availability is missing".into(),
            })?;
        if !availability.is_active() {
            return Err(CapabilityCatalogResolveError::Unavailable {
                capability: self.capability.clone(),
                consumer,
                availability,
            });
        }
        Ok(CapabilityOperationLock {
            capability: self.capability.clone(),
            consumer,
            contribution: ContributionLock {
                source_kind: self.provenance.source_kind,
                source_identity: self.provenance.source_identity.clone(),
                mount_id: self.provenance.mount_id.clone(),
                miniapp_id: self.provenance.miniapp_id.clone(),
                mcp_binding_id: self.provenance.mcp_binding_id.clone(),
                contribution_id: self.contribution_id.clone(),
                contract_digest: self.contract_digest.clone(),
            },
            target_artifact_digest: self.provenance.artifact_digest.clone(),
        })
    }
}

/// One admitted publication input for the shared Catalog materializer.
///
/// The Capability execution manifest owns host surfaces. Supported consumers,
/// release admission, provenance, and per-consumer availability are publication
/// facts and therefore stay in this separate typed envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityCatalogMaterialization {
    pub manifest: CapabilityManifest,
    pub provenance: CapabilityProvenance,
    pub release_state: CapabilityReleaseState,
    pub availability: BTreeMap<CapabilityConsumer, CatalogAvailability>,
}

pub struct CapabilityCatalogMaterializer;

impl CapabilityCatalogMaterializer {
    pub fn materialize(
        input: CapabilityCatalogMaterialization,
    ) -> Result<CapabilityCatalogEntry, CapabilityCatalogContractError> {
        let admission = CapabilityCatalogAdmission::admit(input.release_state)?;
        let supported_consumers = input
            .manifest
            .supported_consumers()
            .map_err(|reason| CapabilityCatalogContractError::InvalidField {
                field: "manifest.supported_surfaces",
                reason,
            })?;
        if supported_consumers.is_empty() {
            return Err(CapabilityCatalogContractError::InvalidField {
                field: "manifest.supported_surfaces",
                reason: "at least one consumer:<id> declaration is required".into(),
            });
        }
        let entry = CapabilityCatalogEntry {
            capability: CapabilityRef {
                id: input.manifest.id.clone(),
                version: input.manifest.version.clone(),
            },
            contract_digest: digest_payload(&input.manifest).map_err(|error| {
                CapabilityCatalogContractError::InvalidField {
                    field: "manifest",
                    reason: error.to_string(),
                }
            })?,
            contribution_id: input.manifest.contribution_id.clone(),
            provenance: input.provenance,
            host_surfaces: input
                .manifest
                .host_surfaces()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            supported_consumers,
            contribution_kind: input.manifest.kind,
            required_runtime_features: input
                .manifest
                .requires_runtime_features
                .iter()
                .map(|feature| feature.id.clone())
                .collect(),
            typed_resource_kinds: input.manifest.contributions.resource_kinds.clone(),
            availability: input.availability,
            admission,
        };
        entry.validate()?;
        Ok(entry)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOperationLock {
    pub capability: CapabilityRef,
    pub consumer: CapabilityConsumer,
    pub contribution: ContributionLock,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_artifact_digest: Option<DigestHex>,
}

impl CapabilityOperationLock {
    pub fn validate(&self) -> Result<(), CapabilityCatalogContractError> {
        validate_non_empty(self.capability.id.as_ref(), "capability.id")?;
        validate_non_empty(self.capability.version.as_ref(), "capability.version")?;
        self.contribution.validate().map_err(|violation| {
            CapabilityCatalogContractError::InvalidField {
                field: "contribution",
                reason: violation.message,
            }
        })?;
        if let Some(digest) = &self.target_artifact_digest {
            validate_digest(digest, "target_artifact_digest")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CapabilityCatalogResolver {
    entries: Arc<[CapabilityCatalogEntry]>,
}

impl CapabilityCatalogResolver {
    pub fn new(
        mut entries: Vec<CapabilityCatalogEntry>,
    ) -> Result<Self, CapabilityCatalogContractError> {
        entries.sort_by(|left, right| left.capability.cmp(&right.capability));
        let mut previous: Option<&CapabilityRef> = None;
        let mut contribution_ids = BTreeSet::new();
        for entry in &entries {
            entry.validate()?;
            if previous == Some(&entry.capability) {
                return Err(CapabilityCatalogContractError::DuplicateCapability {
                    capability: entry.capability.clone(),
                });
            }
            if !contribution_ids.insert(entry.contribution_id.clone()) {
                return Err(CapabilityCatalogContractError::DuplicateContribution {
                    contribution_id: entry.contribution_id.clone(),
                });
            }
            previous = Some(&entry.capability);
        }
        Ok(Self {
            entries: Arc::from(entries),
        })
    }

    pub fn entries(&self) -> &[CapabilityCatalogEntry] {
        &self.entries
    }

    pub fn entry(
        &self,
        capability: &CapabilityRef,
    ) -> Option<&CapabilityCatalogEntry> {
        self.entries
            .binary_search_by(|entry| entry.capability.cmp(capability))
            .ok()
            .map(|index| &self.entries[index])
    }

    pub fn resolve(
        &self,
        capability: &CapabilityRef,
        consumer: CapabilityConsumer,
    ) -> Result<CapabilityOperationLock, CapabilityCatalogResolveError> {
        let entry = self.entry(capability).ok_or_else(|| {
            CapabilityCatalogResolveError::NotMaterialized {
                capability: capability.clone(),
            }
        })?;
        entry.operation_lock(consumer)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CapabilityCatalogResolveError {
    #[error("capability {capability:?} is not materialized")]
    NotMaterialized { capability: CapabilityRef },
    #[error("capability {capability:?} does not support consumer {consumer:?}")]
    UnsupportedConsumer {
        capability: CapabilityRef,
        consumer: CapabilityConsumer,
    },
    #[error(
        "capability {capability:?} is unavailable for consumer {consumer:?}: {availability:?}"
    )]
    Unavailable {
        capability: CapabilityRef,
        consumer: CapabilityConsumer,
        availability: CatalogAvailability,
    },
    #[error("capability {capability:?} has an invalid Catalog entry: {reason}")]
    InvalidEntry {
        capability: CapabilityRef,
        reason: String,
    },
}

impl CapabilityCatalogResolveError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotMaterialized { .. } => "CAPABILITY_NOT_MATERIALIZED",
            Self::UnsupportedConsumer { .. } => "CAPABILITY_CONSUMER_UNSUPPORTED",
            Self::Unavailable { availability, .. } => match availability {
                CatalogAvailability::Active => "CAPABILITY_CATALOG_INVALID",
                CatalogAvailability::Unavailable { .. } => "CAPABILITY_UNAVAILABLE",
                CatalogAvailability::Disabled { .. } => "CAPABILITY_DISABLED",
                CatalogAvailability::NeedsRuntime { .. } => "CAPABILITY_NEEDS_RUNTIME",
                CatalogAvailability::ContractMismatch { .. } => {
                    "CAPABILITY_CONTRACT_MISMATCH"
                }
            },
            Self::InvalidEntry { .. } => "CAPABILITY_CATALOG_INVALID",
        }
    }
}

pub const CAPABILITY_CATALOG_ENTRY_JSON: &str =
    include_str!("../contracts/catalog/platform-capability-catalog-entry.v1.json");

pub fn capability_catalog_entry_fixture() -> CapabilityCatalogEntry {
    serde_json::from_str(CAPABILITY_CATALOG_ENTRY_JSON)
        .expect("platform Capability Catalog fixture must match CapabilityCatalogEntry")
}

fn validate_availability(
    availability: &CatalogAvailability,
    contract_digest: &DigestHex,
) -> Result<(), CapabilityCatalogContractError> {
    match availability {
        CatalogAvailability::Active => {}
        CatalogAvailability::Unavailable { reason }
        | CatalogAvailability::Disabled { reason } => {
            validate_non_empty(reason, "availability.reason")?;
        }
        CatalogAvailability::NeedsRuntime {
            missing_runtime_features,
        } => {
            if missing_runtime_features.is_empty() {
                return Err(CapabilityCatalogContractError::InvalidField {
                    field: "availability.missing_runtime_features",
                    reason: "needs_runtime requires at least one missing feature".into(),
                });
            }
        }
        CatalogAvailability::ContractMismatch {
            expected_contract_digest,
            actual_contract_digest,
        } => {
            validate_digest(expected_contract_digest, "availability.expected_contract_digest")?;
            if expected_contract_digest != contract_digest {
                return Err(CapabilityCatalogContractError::InvalidField {
                    field: "availability.expected_contract_digest",
                    reason: "must match the entry contract_digest".into(),
                });
            }
            if let Some(actual) = actual_contract_digest {
                validate_digest(actual, "availability.actual_contract_digest")?;
            }
        }
    }
    Ok(())
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), CapabilityCatalogContractError> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(CapabilityCatalogContractError::InvalidField {
            field,
            reason: "must be a non-empty canonical value".into(),
        });
    }
    Ok(())
}

fn validate_digest(
    value: &DigestHex,
    field: &'static str,
) -> Result<(), CapabilityCatalogContractError> {
    if value.as_ref().len() != 64
        || !value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CapabilityCatalogContractError::InvalidField {
            field,
            reason: "must be 64 lowercase hexadecimal characters".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapabilityCatalogContractError {
    #[error("{field} {reason}")]
    InvalidField {
        field: &'static str,
        reason: String,
    },
    #[error("catalog availability keys must exactly match supported_consumers")]
    AvailabilityCoverage,
    #[error("release state {release_state:?} cannot enter the formal Capability Catalog")]
    CandidateRejected { release_state: CapabilityReleaseState },
    #[error("capability {capability:?} is duplicated in the formal Capability Catalog")]
    DuplicateCapability { capability: CapabilityRef },
    #[error("contribution {contribution_id:?} is duplicated in the formal Capability Catalog")]
    DuplicateContribution { contribution_id: ContributionId },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> CapabilityCatalogEntry {
        capability_catalog_entry_fixture()
    }

    #[test]
    fn catalog_fixture_is_a_published_typed_entry() {
        let entry = fixture();
        entry.validate().expect("catalog fixture");
        assert!(entry.supports_consumer(CapabilityConsumer::Agent));
        assert!(entry.is_available_for(CapabilityConsumer::Agent));
        assert_eq!(
            entry.provenance.source_kind,
            ContributionSourceKind::PlatformBuiltin
        );
    }

    #[test]
    fn catalog_entry_rejects_unknown_fields() {
        let mut value = serde_json::to_value(fixture()).expect("serialize catalog fixture");
        value
            .as_object_mut()
            .expect("catalog entry object")
            .insert("unexpected".into(), json!(true));
        assert!(
            serde_json::from_value::<CapabilityCatalogEntry>(value).is_err(),
            "catalog contract must fail closed on unknown fields"
        );
    }

    #[test]
    fn catalog_filters_non_agent_only_entries_without_copying_the_catalog() {
        let agent_entry = fixture();
        let mut gateway_only = fixture();
        gateway_only.supported_consumers = BTreeSet::from([CapabilityConsumer::Gateway]);
        gateway_only.availability =
            BTreeMap::from([(CapabilityConsumer::Gateway, CatalogAvailability::Active)]);
        gateway_only.validate().expect("gateway-only entry");

        let entries = vec![agent_entry, gateway_only];
        let agent_entries =
            CapabilityCatalogEntry::filter_for_consumer(&entries, CapabilityConsumer::Agent);
        assert_eq!(agent_entries.len(), 1);
        assert_eq!(
            agent_entries[0].capability.id.as_ref(),
            "knowledge.search"
        );
        assert_eq!(
            CapabilityCatalogEntry::filter_available_for_consumer(
                &entries,
                CapabilityConsumer::Agent
            )
            .len(),
            1
        );
    }

    #[test]
    fn catalog_keeps_materialization_and_consumer_availability_separate() {
        let mut entry = fixture();
        entry.availability.insert(
            CapabilityConsumer::Gateway,
            CatalogAvailability::Disabled {
                reason: "gateway disabled in this installation".into(),
            },
        );
        entry.supported_consumers.insert(CapabilityConsumer::Gateway);
        entry.validate().expect("mixed availability entry");
        assert!(entry.supports_consumer(CapabilityConsumer::Gateway));
        assert!(!entry.is_available_for(CapabilityConsumer::Gateway));
        assert!(entry.is_available_for(CapabilityConsumer::Agent));
    }

    #[test]
    fn catalog_rejects_candidate_and_unpublished_release_admission() {
        for release_state in [
            CapabilityReleaseState::ReadyCandidate,
            CapabilityReleaseState::UnpublishedRelease,
            CapabilityReleaseState::UnstartedService,
            CapabilityReleaseState::ProjectSource,
            CapabilityReleaseState::TestHost,
        ] {
            let admission = CapabilityCatalogAdmission::for_release(release_state);
            assert!(!admission.admitted);
            assert!(CapabilityCatalogAdmission::admit(release_state).is_err());

            let mut entry = fixture();
            entry.admission = admission;
            assert!(matches!(
                entry.validate(),
                Err(CapabilityCatalogContractError::CandidateRejected { .. })
            ));
        }
    }

    #[test]
    fn catalog_rejects_unknown_consumer_availability_and_bad_provenance() {
        let mut missing = fixture();
        missing.availability.clear();
        assert!(matches!(
            missing.validate(),
            Err(CapabilityCatalogContractError::AvailabilityCoverage)
        ));

        let mut bad_provenance = fixture();
        bad_provenance.provenance.mount_id = Some("mount-1".into());
        assert!(bad_provenance.validate().is_err());
    }

    #[test]
    fn resolver_lock_reuses_formal_catalog_provenance_and_digest() {
        let entry = fixture();
        let expected_provenance = entry.provenance.clone();
        let expected_digest = entry.contract_digest.clone();
        let resolver =
            CapabilityCatalogResolver::new(vec![entry.clone()]).expect("resolver catalog");
        let lock = resolver
            .resolve(&entry.capability, CapabilityConsumer::Agent)
            .expect("agent operation lock");
        assert_eq!(lock.contribution.source_kind, expected_provenance.source_kind);
        assert_eq!(
            lock.contribution.source_identity,
            expected_provenance.source_identity
        );
        assert_eq!(lock.contribution.contract_digest, expected_digest);
        assert_eq!(
            lock.contribution.contribution_id,
            entry.contribution_id
        );
        lock.validate().expect("resolved operation lock");
    }

    #[test]
    fn catalog_rejects_duplicate_contribution_identity() {
        let first = fixture();
        let mut second = fixture();
        second.capability.id = crate::CapabilityId::from("example.second");
        assert!(matches!(
            CapabilityCatalogResolver::new(vec![first, second]),
            Err(CapabilityCatalogContractError::DuplicateContribution { .. })
        ));
    }

}
