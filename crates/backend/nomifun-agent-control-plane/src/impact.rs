use std::sync::Arc;

use nomifun_agent_contracts::{
    CanonicalErrorCode, CatalogAvailability, CurrentContribution,
    CurrentContributionLifecycle, PluginSourceKind,
};

use crate::catalog::{CatalogProvider, CatalogSnapshot};
use crate::error::ControlPlaneError;

/// Read-only source of the formal current contribution catalog used by AP-5
/// impact reads. Owner domains may inject lifecycle-aware records without
/// changing immutable AgentPreset Revisions.
pub trait RevisionImpactCatalogProvider: Send + Sync {
    fn current_contributions(&self) -> Result<Vec<CurrentContribution>, ControlPlaneError>;
}

/// Default adapter for the currently composed control-plane CatalogProvider.
/// It preserves the exact source identity rules used by the current Compiler.
/// Owner-specific lifecycle providers can replace this adapter through
/// `AgentControlPlane::with_revision_impact_catalog_provider`.
pub struct ControlPlaneRevisionImpactCatalogProvider {
    catalog: Arc<dyn CatalogProvider>,
}

impl ControlPlaneRevisionImpactCatalogProvider {
    pub fn new(catalog: Arc<dyn CatalogProvider>) -> Self {
        Self { catalog }
    }
}

impl RevisionImpactCatalogProvider for ControlPlaneRevisionImpactCatalogProvider {
    fn current_contributions(&self) -> Result<Vec<CurrentContribution>, ControlPlaneError> {
        let snapshot = self.catalog.snapshot()?;
        current_contributions_from_catalog(snapshot.as_ref())
    }
}

#[derive(Clone, Default)]
pub struct StaticRevisionImpactCatalogProvider {
    contributions: Arc<Vec<CurrentContribution>>,
}

impl StaticRevisionImpactCatalogProvider {
    pub fn new(contributions: Vec<CurrentContribution>) -> Self {
        Self {
            contributions: Arc::new(contributions),
        }
    }
}

impl RevisionImpactCatalogProvider for StaticRevisionImpactCatalogProvider {
    fn current_contributions(&self) -> Result<Vec<CurrentContribution>, ControlPlaneError> {
        Ok(self.contributions.as_ref().clone())
    }
}

fn current_contributions_from_catalog(
    catalog: &CatalogSnapshot,
) -> Result<Vec<CurrentContribution>, ControlPlaneError> {
    catalog.validate()?;
    let mut contributions = Vec::new();

    for entry in catalog.formal_capability_entries.values() {
        contributions.push(CurrentContribution {
            source_kind: entry.provenance.source_kind,
            source_identity: entry.provenance.source_identity.clone(),
            mount_id: entry.provenance.mount_id.clone(),
            miniapp_id: entry.provenance.miniapp_id.clone(),
            mcp_binding_id: entry.provenance.mcp_binding_id.clone(),
            contribution_id: entry.contribution_id.clone(),
            contract_digest: entry.contract_digest.clone(),
            lifecycle: entry
                .availability_for(
                    nomifun_agent_contracts::CapabilityConsumer::Agent,
                )
                .map(availability_lifecycle)
                .unwrap_or(CurrentContributionLifecycle::Active),
        });
    }

    for skill in &catalog.skills {
        if skill.source.source_kind == PluginSourceKind::TestFixture {
            continue;
        }
        contributions.push(CurrentContribution {
            source_kind: skill.contribution_lock.source_kind,
            source_identity: skill.contribution_lock.source_identity.clone(),
            mount_id: skill.contribution_lock.mount_id.clone(),
            miniapp_id: skill.contribution_lock.miniapp_id.clone(),
            mcp_binding_id: skill.contribution_lock.mcp_binding_id.clone(),
            contribution_id: skill.contribution_id.clone(),
            contract_digest: skill.contract_digest.clone(),
            lifecycle: CurrentContributionLifecycle::Active,
        });
    }

    contributions.sort();
    for contribution in &contributions {
        contribution.validate().map_err(|error| {
            ControlPlaneError::canonical(
                "CAPABILITY_CATALOG_INVALID",
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            )
        })?;
    }
    Ok(contributions)
}

fn availability_lifecycle(
    availability: &CatalogAvailability,
) -> CurrentContributionLifecycle {
    match availability {
        CatalogAvailability::Active => CurrentContributionLifecycle::Active,
        CatalogAvailability::Disabled { reason } => {
            CurrentContributionLifecycle::Disabled {
                reason: reason.clone(),
            }
        }
        CatalogAvailability::Unavailable { reason } => {
            CurrentContributionLifecycle::Unavailable {
                code: CanonicalErrorCode::from(reason.clone()),
                reason: format!("current catalog reports {reason}"),
            }
        }
        CatalogAvailability::NeedsRuntime { .. } => {
            CurrentContributionLifecycle::Unavailable {
                code: CanonicalErrorCode::from("CAPABILITY_NEEDS_RUNTIME"),
                reason: "current catalog requires an unavailable runtime"
                    .to_owned(),
            }
        }
        CatalogAvailability::ContractMismatch { .. } => {
            CurrentContributionLifecycle::Unavailable {
                code: CanonicalErrorCode::from(
                    "CAPABILITY_CONTRACT_MISMATCH",
                ),
                reason: "current catalog reports a contract mismatch"
                    .to_owned(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        ArtifactId, CapabilityCatalogMaterialization,
        CapabilityCatalogMaterializer, CapabilityConsumer,
        CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityReleaseState, ContributionId, ContributionLock,
        ContributionSourceKind, DigestHex, LocalizedMetadata,
        LogicalArtifactRef, McpBindingId, McpServerId,
        McpToolCapabilityMapping, McpToolKey, PackageId, PackageRef,
        PluginMountId, PluginSourceKind, PluginSourceMetadata,
        SkillDefinition, SkillId, StableSourceIdentity, StrictJsonValue,
        VersionString, capability_surface_declarations, digest_payload,
    };
    use nomifun_agent_kernel::{
        MaterializedCapability, MaterializedMcpTool, MaterializedSkill,
    };
    use nomifun_api_types::{
        ContributionContractImpactDto, ContributionImpactDto, RevisionUseReadinessDto,
    };
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn digest(byte: char) -> DigestHex {
        DigestHex::from(byte.to_string().repeat(64))
    }

    fn package() -> PackageRef {
        PackageRef {
            id: PackageId::from("package.example"),
            version: VersionString::from("1.0.0"),
        }
    }

    #[test]
    fn default_adapter_preserves_managed_skill_and_mcp_backed_provenance() {
        let package = package();
        let mount_id = PluginMountId::from("mount-exact");
        let artifact_digest = digest('c');
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: "managed-source-exact".to_owned(),
            source_digest: Some(artifact_digest.clone()),
        };
        let capability = CapabilityManifest {
            id: CapabilityId::from("example.run"),
            contribution_id: ContributionId::from("capability:example.run"),
            version: VersionString::from("1.0.0"),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Example".into(),
                description: "Example capability".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: Vec::new(),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions::default(),
        };
        let capability_digest = digest_payload(&capability).unwrap();
        let binding_id = McpBindingId::from("server.example:tool.example");
        let capability_lock = ContributionLock {
            source_kind: ContributionSourceKind::McpBinding,
            source_identity: StableSourceIdentity::from(
                "mcp:server.example",
            ),
            mount_id: Some(mount_id.clone()),
            miniapp_id: None,
            mcp_binding_id: Some(binding_id.clone()),
            contribution_id: capability.contribution_id.clone(),
            contract_digest: capability_digest.clone(),
        };
        let materialized_capability = MaterializedCapability {
            manifest: capability.clone(),
            schema_digest: capability_digest.clone(),
            contribution_id: capability.contribution_id.clone(),
            contribution_lock: capability_lock.clone(),
            target_artifact_digest: artifact_digest.clone(),
            mount_id: mount_id.clone(),
            source: source.clone(),
        };
        let formal_capability = CapabilityCatalogMaterializer::materialize(
            CapabilityCatalogMaterialization {
                manifest: capability,
                provenance: CapabilityProvenance {
                    owner: CapabilityOwner::Package {
                        package: package.clone(),
                    },
                    source_kind: capability_lock.source_kind,
                    source_identity: capability_lock.source_identity.clone(),
                    mount_id: capability_lock.mount_id.clone(),
                    miniapp_id: None,
                    mcp_binding_id: Some(binding_id.clone()),
                    artifact_digest: Some(artifact_digest.clone()),
                },
                release_state: CapabilityReleaseState::PublishedActive,
                availability: BTreeMap::from([(
                    CapabilityConsumer::Agent,
                    CatalogAvailability::Unavailable {
                        reason: "CAPABILITY_NOT_ACTIVE".to_owned(),
                    },
                )]),
            },
        )
        .unwrap();
        let skill = SkillDefinition {
            id: SkillId::from("example.skill"),
            version: VersionString::from("1.0.0"),
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Skill".into(),
                description: "Example skill".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            body_ref: LogicalArtifactRef {
                artifact_id: ArtifactId::from("skill-body"),
                normalized_relative_path: "skill.md".into(),
                digest: digest('a'),
            },
            resources: Vec::new(),
            requires_capabilities: Vec::new(),
            supported_surfaces: BTreeSet::from(["agent".into()]),
        };
        let skill_digest = digest_payload(&skill).unwrap();
        let skill_contribution_id = ContributionId::from("skill:example.skill");
        let materialized_skill = MaterializedSkill {
            definition: skill,
            contribution_id: skill_contribution_id.clone(),
            contract_digest: skill_digest.clone(),
            contribution_lock: ContributionLock {
                source_kind: ContributionSourceKind::PluginMount,
                source_identity: StableSourceIdentity::from(
                    source.source_identity.clone(),
                ),
                mount_id: Some(mount_id.clone()),
                miniapp_id: None,
                mcp_binding_id: None,
                contribution_id: skill_contribution_id,
                contract_digest: skill_digest,
            },
            target_artifact_digest: artifact_digest.clone(),
            mount_id: mount_id.clone(),
            source: source.clone(),
        };
        let mapping = McpToolCapabilityMapping {
            package,
            server_id: McpServerId::from("server.example"),
            canonical_tool_key: McpToolKey::from("tool.example"),
            schema_digest: digest('d'),
            capability: formal_capability.capability.clone(),
            materialization_version: VersionString::from("1.0.0"),
        };
        let materialized_mcp = MaterializedMcpTool {
            mapping,
            binding_id: binding_id.clone(),
            contribution_lock: capability_lock,
            target_artifact_digest: artifact_digest,
            mount_id: mount_id.clone(),
            source,
        };
        let snapshot = CatalogSnapshot {
            capabilities: vec![materialized_capability],
            formal_capability_entries: BTreeMap::from([(
                formal_capability.capability.clone(),
                formal_capability,
            )]),
            skills: vec![materialized_skill],
            mcp_tools: vec![materialized_mcp],
            unavailable_capabilities: BTreeMap::from([(
                CapabilityId::from("example.run"),
                CanonicalErrorCode::from("CAPABILITY_NOT_ACTIVE"),
            )]),
            service_key_diagnostics: Vec::new(),
        };

        let current = current_contributions_from_catalog(&snapshot).expect("impact catalog");
        assert_eq!(current.len(), 2);
        let mcp_capability = current
            .iter()
            .find(|item| item.contribution_id.as_ref() == "capability:example.run")
            .unwrap();
        assert_eq!(
            mcp_capability.source_kind,
            ContributionSourceKind::McpBinding
        );
        assert_eq!(mcp_capability.mount_id.as_ref(), Some(&mount_id));
        assert_eq!(
            mcp_capability.mcp_binding_id.as_ref(),
            Some(&binding_id)
        );
        assert!(matches!(
            mcp_capability.lifecycle,
            CurrentContributionLifecycle::Unavailable { .. }
        ));
        let skill = current
            .iter()
            .find(|item| item.contribution_id.as_ref() == "skill:example.skill")
            .unwrap();
        assert_eq!(skill.source_kind, ContributionSourceKind::PluginMount);
        assert_eq!(skill.mount_id.as_ref(), Some(&mount_id));
    }

    #[test]
    fn contract_impact_projects_to_the_public_typed_dto() {
        let lock = ContributionLock {
            source_kind: ContributionSourceKind::PluginMount,
            source_identity: StableSourceIdentity::from("plugin.example"),
            mount_id: Some(PluginMountId::from("mount-1")),
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: ContributionId::from("capability:example.run"),
            contract_digest: digest('a'),
        };
        let current = CurrentContribution {
            source_kind: lock.source_kind,
            source_identity: lock.source_identity.clone(),
            mount_id: lock.mount_id.clone(),
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: lock.contribution_id.clone(),
            contract_digest: digest('a'),
            lifecycle: CurrentContributionLifecycle::Replaced {
                target_digest: Some(digest('b')),
            },
        };
        let diff = nomifun_agent_contracts::compare_revision_contribution_locks(
            &[lock],
            &[current],
        )
        .expect("impact diff");
        let api: Vec<ContributionImpactDto> =
            crate::wire::wire_cast(&diff.contributions).expect("public impact DTO");
        assert_eq!(api.len(), 1);
        assert_eq!(
            api[0].contract,
            ContributionContractImpactDto::Compatible
        );
        assert_eq!(api[0].new_use, RevisionUseReadinessDto::Ready);
    }
}
