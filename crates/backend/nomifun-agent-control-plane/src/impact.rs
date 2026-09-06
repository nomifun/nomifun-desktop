use std::sync::Arc;

use nomifun_agent_contracts::{
    CanonicalErrorCode, ContributionId, ContributionSourceKind, CurrentContribution,
    CurrentContributionLifecycle, McpBindingId, PluginMountId, StableSourceIdentity,
    digest_payload,
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
    let mut contributions = Vec::new();

    for capability in &catalog.capabilities {
        let (source_kind, source_identity, mount_id) =
            contribution_source_for_package(catalog, &capability.package);
        let contract_digest =
            digest_payload(capability).map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        contributions.push(CurrentContribution {
            source_kind,
            source_identity,
            mount_id,
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: ContributionId::from(format!(
                "capability:{}",
                capability.id.as_ref()
            )),
            contract_digest,
            lifecycle: catalog
                .unavailable_capabilities
                .get(&capability.id)
                .map(unavailable_lifecycle)
                .unwrap_or(CurrentContributionLifecycle::Active),
        });
    }

    for skill in &catalog.skills {
        let (source_kind, source_identity, mount_id) =
            contribution_source_for_package(catalog, &skill.package);
        contributions.push(CurrentContribution {
            source_kind,
            source_identity,
            mount_id,
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: ContributionId::from(format!("skill:{}", skill.id.as_ref())),
            contract_digest: skill.body_ref.digest.clone(),
            lifecycle: CurrentContributionLifecycle::Active,
        });
    }

    for mapping in &catalog.mcp_tools {
        let binding_id = format!(
            "{}:{}",
            mapping.server_id.as_ref(),
            mapping.canonical_tool_key.as_ref()
        );
        contributions.push(CurrentContribution {
            source_kind: ContributionSourceKind::McpBinding,
            source_identity: StableSourceIdentity::from(format!(
                "mcp:{}",
                mapping.server_id.as_ref()
            )),
            mount_id: None,
            miniapp_id: None,
            mcp_binding_id: Some(McpBindingId::from(binding_id.clone())),
            contribution_id: ContributionId::from(format!("mcp:{binding_id}")),
            contract_digest: mapping.schema_digest.clone(),
            lifecycle: CurrentContributionLifecycle::Active,
        });
    }

    contributions.sort();
    Ok(contributions)
}

fn contribution_source_for_package(
    catalog: &CatalogSnapshot,
    package: &nomifun_agent_contracts::PackageRef,
) -> (
    ContributionSourceKind,
    StableSourceIdentity,
    Option<PluginMountId>,
) {
    let source_identity = StableSourceIdentity::from(format!(
        "{}@{}",
        package.id.as_ref(),
        package.version.as_ref()
    ));
    match catalog.source_kind(package) {
        nomifun_agent_contracts::PluginSourceKind::ManagedLocal => (
            ContributionSourceKind::PluginMount,
            source_identity,
            Some(PluginMountId::from(format!(
                "package:{}@{}",
                package.id.as_ref(),
                package.version.as_ref()
            ))),
        ),
        _ => (ContributionSourceKind::PlatformBuiltin, source_identity, None),
    }
}

fn unavailable_lifecycle(code: &CanonicalErrorCode) -> CurrentContributionLifecycle {
    CurrentContributionLifecycle::Unavailable {
        code: code.clone(),
        reason: format!("current catalog reports {}", code.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        ArtifactId, CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
        ContributionLock, DigestHex, LocalizedMetadata, LogicalArtifactRef, PackageId, PackageRef,
        SkillDefinition, SkillId, StrictJsonValue, VersionString,
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
    fn default_adapter_emits_exact_server_owned_capability_and_skill_facts() {
        let package = package();
        let capability = CapabilityManifest {
            id: CapabilityId::from("example.run"),
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
            supported_surfaces: BTreeSet::from(["agent".into()]),
            requires_runtime_features: Vec::new(),
            supported_platforms: Vec::new(),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions::default(),
        };
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
        let snapshot = CatalogSnapshot {
            capabilities: vec![capability],
            skills: vec![skill],
            mcp_tools: Vec::new(),
            package_sources: BTreeMap::from([(
                package,
                nomifun_agent_contracts::PluginSourceKind::ManagedLocal,
            )]),
            unavailable_capabilities: BTreeMap::from([(
                CapabilityId::from("example.run"),
                CanonicalErrorCode::from("CAPABILITY_NOT_ACTIVE"),
            )]),
            service_key_diagnostics: Vec::new(),
        };

        let current = current_contributions_from_catalog(&snapshot).expect("impact catalog");
        assert_eq!(current.len(), 2);
        assert!(current.iter().all(|item| {
            item.source_kind == ContributionSourceKind::PluginMount
                && item.mount_id.as_ref().is_some_and(|id| {
                    id.as_ref() == "package:package.example@1.0.0"
                })
        }));
        assert!(matches!(
            current[0].lifecycle,
            CurrentContributionLifecycle::Unavailable { .. }
        ));
        current
            .iter()
            .for_each(|item| item.validate().expect("valid current contribution"));
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
