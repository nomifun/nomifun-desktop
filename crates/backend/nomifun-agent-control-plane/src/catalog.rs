use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    CapabilityId, CapabilityManifest, CapabilityRef, CanonicalErrorCode,
    McpToolCapabilityMapping, OfficialPresetKey, OfficialPresetSeedManifestPayload, PackageRef,
    PluginSourceKind, SkillDefinition, SkillRef, official_preset_seed_manifest_payload,
};
use nomifun_api_types::{
    AgentCatalogResponse, CapabilityCatalogItemDto, CatalogMaterializationStateDto,
    ExactCatalogRefDto, McpToolCatalogItemDto, OfficialPresetRoleCoverageDto,
    OfficialPresetSeedDto, OfficialPresetTemplateDto, SkillCatalogItemDto,
};

use crate::error::ControlPlaneError;
use crate::wire::{wire_cast, wire_name};

#[derive(Clone, Debug, Default)]
pub struct CatalogSnapshot {
    pub capabilities: Vec<CapabilityManifest>,
    pub skills: Vec<SkillDefinition>,
    pub mcp_tools: Vec<McpToolCapabilityMapping>,
    pub package_sources: BTreeMap<PackageRef, PluginSourceKind>,
    pub unavailable_capabilities: BTreeMap<CapabilityId, CanonicalErrorCode>,
    pub service_key_diagnostics: Vec<String>,
}

impl CatalogSnapshot {
    pub fn find_capability(&self, reference: &CapabilityRef) -> Option<&CapabilityManifest> {
        self.capabilities.iter().find(|capability| {
            capability.id == reference.id && capability.version == reference.version
        })
    }

    pub fn find_skill(&self, reference: &SkillRef) -> Option<&SkillDefinition> {
        self.skills
            .iter()
            .find(|skill| skill.id == reference.id && skill.version == reference.version)
    }

    pub fn source_kind(&self, package: &PackageRef) -> PluginSourceKind {
        self.package_sources
            .get(package)
            .copied()
            .unwrap_or(PluginSourceKind::Bundled)
    }

    pub fn as_api(&self) -> Result<AgentCatalogResponse, ControlPlaneError> {
        let capabilities = self
            .capabilities
            .iter()
            .map(|capability| {
                let unavailable_code = self
                    .unavailable_capabilities
                    .get(&capability.id)
                    .map(|code| code.as_ref().to_owned());
                Ok(CapabilityCatalogItemDto {
                    capability: ExactCatalogRefDto {
                        id: capability.id.as_ref().to_owned(),
                        version: capability.version.as_ref().to_owned(),
                    },
                    kind: wire_name(&capability.kind)?,
                    display_name: capability.display.name.clone(),
                    description: capability.display.description.clone(),
                    source_package: ExactCatalogRefDto {
                        id: capability.package.id.as_ref().to_owned(),
                        version: capability.package.version.as_ref().to_owned(),
                    },
                    source_kind: wire_name(&self.source_kind(&capability.package))?,
                    materialization_state: if unavailable_code.is_some() {
                        CatalogMaterializationStateDto::Unavailable
                    } else {
                        CatalogMaterializationStateDto::Materialized
                    },
                    unavailable_code,
                    supported_surfaces: capability.supported_surfaces.clone(),
                    required_runtime_features: capability
                        .requires_runtime_features
                        .iter()
                        .map(|feature| feature.id.as_ref().to_owned())
                        .collect(),
                    required_resource_kinds: capability
                        .contributions
                        .resource_kinds
                        .iter()
                        .map(|kind| kind.as_ref().to_owned())
                        .collect(),
                    required_capabilities: capability
                        .requires
                        .iter()
                        .map(|reference| ExactCatalogRefDto {
                            id: reference.id.as_ref().to_owned(),
                            version: reference.version.as_ref().to_owned(),
                        })
                        .collect(),
                    conflicting_capabilities: capability
                        .conflicts
                        .iter()
                        .map(|conflict| ExactCatalogRefDto {
                            id: conflict.capability.id.as_ref().to_owned(),
                            version: conflict.capability.version.as_ref().to_owned(),
                        })
                        .collect(),
                    action_count: capability.contributions.actions.len() as u32,
                    context_contributor_count: capability
                        .contributions
                        .context_schema_refs
                        .len() as u32,
                })
            })
            .collect::<Result<Vec<_>, ControlPlaneError>>()?;

        let skills = self
            .skills
            .iter()
            .map(|skill| {
                Ok(SkillCatalogItemDto {
                    skill: ExactCatalogRefDto {
                        id: skill.id.as_ref().to_owned(),
                        version: skill.version.as_ref().to_owned(),
                    },
                    display_name: skill.display.name.clone(),
                    description: skill.display.description.clone(),
                    source_package: ExactCatalogRefDto {
                        id: skill.package.id.as_ref().to_owned(),
                        version: skill.package.version.as_ref().to_owned(),
                    },
                    source_kind: wire_name(&self.source_kind(&skill.package))?,
                    required_capabilities: skill
                        .requires_capabilities
                        .iter()
                        .map(|reference| ExactCatalogRefDto {
                            id: reference.id.as_ref().to_owned(),
                            version: reference.version.as_ref().to_owned(),
                        })
                        .collect(),
                    supported_surfaces: skill.supported_surfaces.clone(),
                })
            })
            .collect::<Result<Vec<_>, ControlPlaneError>>()?;

        let mcp_tools = self
            .mcp_tools
            .iter()
            .map(|mapping| McpToolCatalogItemDto {
                server_id: mapping.server_id.as_ref().to_owned(),
                canonical_tool_key: mapping.canonical_tool_key.as_ref().to_owned(),
                capability: ExactCatalogRefDto {
                    id: mapping.capability.id.as_ref().to_owned(),
                    version: mapping.capability.version.as_ref().to_owned(),
                },
                source_package: ExactCatalogRefDto {
                    id: mapping.package.id.as_ref().to_owned(),
                    version: mapping.package.version.as_ref().to_owned(),
                },
                schema_digest: mapping.schema_digest.as_ref().to_owned(),
                materialization_version: mapping.materialization_version.as_ref().to_owned(),
            })
            .collect();

        Ok(AgentCatalogResponse {
            capabilities,
            skills,
            mcp_tools,
        })
    }
}

pub trait CatalogProvider: Send + Sync {
    fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, ControlPlaneError>;
}

#[derive(Clone)]
pub struct StaticCatalogProvider {
    snapshot: Arc<CatalogSnapshot>,
}

impl StaticCatalogProvider {
    pub fn new(snapshot: CatalogSnapshot) -> Self {
        Self {
            snapshot: Arc::new(snapshot),
        }
    }
}

impl CatalogProvider for StaticCatalogProvider {
    fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, ControlPlaneError> {
        Ok(Arc::clone(&self.snapshot))
    }
}

#[derive(Clone, Debug)]
pub struct OfficialTemplateCatalog {
    manifest: OfficialPresetSeedManifestPayload,
}

impl OfficialTemplateCatalog {
    pub fn load() -> Result<Self, ControlPlaneError> {
        let manifest = official_preset_seed_manifest_payload();
        manifest
            .validate()
            .map_err(|violation| ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                violation.message,
            ))?;
        Ok(Self { manifest })
    }

    pub fn get(&self, key: OfficialPresetKey) -> Option<OfficialPresetTemplateDto> {
        let seed = self.manifest.templates.get(&key)?;
        let role_coverage = self.manifest.role_coverage.get(&key)?;
        Some(OfficialPresetTemplateDto {
            template_key: wire_cast(&key).ok()?,
            seed: wire_cast::<_, OfficialPresetSeedDto>(seed).ok()?,
            role_coverage: wire_cast::<_, OfficialPresetRoleCoverageDto>(role_coverage).ok()?,
            immutable: true,
            forkable: true,
        })
    }

    pub fn list(&self) -> Result<Vec<OfficialPresetTemplateDto>, ControlPlaneError> {
        OfficialPresetKey::ALL
            .into_iter()
            .map(|key| {
                self.get(key).ok_or_else(|| {
                    ControlPlaneError::canonical(
                        "OFFICIAL_PRESET_KEY_SET_MISMATCH",
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        format!("official template {} is missing", key.as_str()),
                    )
                })
            })
            .collect()
    }

    pub fn required_capability_ids(
        &self,
        key: OfficialPresetKey,
    ) -> Option<BTreeSet<CapabilityId>> {
        self.manifest
            .role_coverage
            .get(&key)
            .map(|coverage| coverage.required_capability_ids.clone())
    }

    pub fn required_runtime_features(
        &self,
        key: OfficialPresetKey,
    ) -> Option<BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>> {
        self.manifest
            .role_coverage
            .get(&key)
            .map(|coverage| coverage.required_runtime_features.clone())
    }

    pub fn seed(
        &self,
        key: OfficialPresetKey,
    ) -> Option<&nomifun_agent_contracts::OfficialPresetSeed> {
        self.manifest.templates.get(&key)
    }
}
