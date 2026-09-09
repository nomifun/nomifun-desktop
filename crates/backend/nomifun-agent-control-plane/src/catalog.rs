use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use nomifun_agent_contracts::{
    CanonicalErrorCode, CapabilityCatalogEntry,
    CapabilityConsumer, CapabilityId, CapabilityOwner, CapabilityRef,
    CatalogAvailability, ContributionSourceKind, McpBindingId, McpServerId,
    McpToolKey, MiniAppCapabilityCatalogPublication, MiniAppCapabilityCatalogSink,
    MiniAppCapabilityCatalogPublicationUpdate, MiniAppId, OfficialPresetKey,
    OfficialPresetSeedManifestPayload,
    PluginSourceKind, SkillRef, digest_payload,
    official_preset_seed_manifest_payload,
};
use nomifun_agent_kernel::{
    MaterializedCapability, MaterializedMcpTool, MaterializedSkill,
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
    pub capabilities: Vec<MaterializedCapability>,
    pub formal_capability_entries: BTreeMap<CapabilityRef, CapabilityCatalogEntry>,
    pub miniapp_publications:
        BTreeMap<MiniAppId, MiniAppCapabilityCatalogPublication>,
    pub skills: Vec<MaterializedSkill>,
    pub mcp_tools: Vec<MaterializedMcpTool>,
    pub unavailable_capabilities: BTreeMap<CapabilityId, CanonicalErrorCode>,
    pub service_key_diagnostics: Vec<String>,
}

/// Read-side source for durable MiniApp Active Release publications.
///
/// The source is deliberately narrower than `CatalogProvider`: it contributes
/// only MiniApp publications, while `KernelCatalogProvider` remains the single
/// assembled Catalog provider used by all consumers.
pub trait MiniAppCatalogPublicationSource: Send + Sync {
    fn publications(
        &self,
    ) -> Result<Vec<MiniAppCapabilityCatalogPublication>, ControlPlaneError>;
}

/// Shared in-process publication index used by the Desktop composition.
///
/// Each MiniApp is replaced as one immutable publication. This prevents a
/// consumer snapshot from observing capabilities from two different Active
/// Release identities.
#[derive(Clone, Default)]
pub struct SharedMiniAppCatalogPublications {
    publications:
        Arc<RwLock<BTreeMap<(nomifun_agent_contracts::UserId, MiniAppId), MiniAppCapabilityCatalogPublicationUpdate>>>,
}

impl SharedMiniAppCatalogPublications {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MiniAppCapabilityCatalogSink for SharedMiniAppCatalogPublications {
    fn replace_miniapp_publication(
        &self,
        update: MiniAppCapabilityCatalogPublicationUpdate,
    ) -> Result<(), String> {
        update.validate().map_err(|error| error.to_string())?;
        let mut publications = self
            .publications
            .write()
            .map_err(|_| "MiniApp Catalog publication index is poisoned".to_owned())?;
        let key = (update.owner_user_id.clone(), update.miniapp_id.clone());
        if let Some(current) = publications.get(&key) {
            let ordering = miniapp_publication_version(current)
                .cmp(&miniapp_publication_version(&update));
            match ordering {
                std::cmp::Ordering::Greater => return Ok(()),
                std::cmp::Ordering::Equal if current == &update => return Ok(()),
                std::cmp::Ordering::Equal => {
                    return Err(
                        "MiniApp Catalog update reuses a version with different publication facts"
                            .to_owned(),
                    );
                }
                std::cmp::Ordering::Less => {}
            }
        }
        publications.insert(key, update);
        Ok(())
    }
}

impl MiniAppCatalogPublicationSource for SharedMiniAppCatalogPublications {
    fn publications(
        &self,
    ) -> Result<Vec<MiniAppCapabilityCatalogPublication>, ControlPlaneError> {
        self.publications
            .read()
            .map_err(|_| catalog_invalid("MiniApp Catalog publication index is poisoned"))
            .and_then(|publications| {
                let mut active = BTreeMap::new();
                for update in publications.values() {
                    if let Some(publication) = &update.publication {
                        if active
                            .insert(publication.miniapp_id.clone(), publication.clone())
                            .is_some()
                        {
                            return Err(catalog_invalid(
                                "multiple owner-scoped MiniApp publications share one MiniApp identity",
                            ));
                        }
                    }
                }
                Ok(active.into_values().collect())
            })
    }
}

fn miniapp_publication_version(
    update: &MiniAppCapabilityCatalogPublicationUpdate,
) -> (u64, u64, u64) {
    (
        update.product_revision,
        update.pointer_revision,
        update.active_release_epoch,
    )
}

impl CatalogSnapshot {
    pub fn validate(&self) -> Result<(), ControlPlaneError> {
        let mut capabilities = BTreeSet::new();
        let mut contribution_ids = BTreeSet::new();
        for capability in &self.capabilities {
            let reference = capability_reference(capability);
            if !capabilities.insert(reference.clone()) {
                return Err(catalog_invalid(format!(
                    "duplicate materialized capability {}@{}",
                    reference.id.as_ref(),
                    reference.version.as_ref()
                )));
            }
            match capability.source.source_kind {
                PluginSourceKind::TestFixture => {
                    if self.formal_capability_entries.contains_key(&reference) {
                        return Err(catalog_invalid(format!(
                            "test-host capability {}@{} entered the formal Catalog",
                            reference.id.as_ref(),
                            reference.version.as_ref()
                        )));
                    }
                }
                PluginSourceKind::Bundled | PluginSourceKind::ManagedLocal => {
                    let entry = self
                        .formal_capability_entries
                        .get(&reference)
                        .ok_or_else(|| {
                            catalog_invalid(format!(
                                "materialized capability {}@{} has no formal Catalog entry",
                                reference.id.as_ref(),
                                reference.version.as_ref()
                            ))
                        })?;
                    validate_capability_entry(capability, entry)?;
                }
            }
            if !contribution_ids
                .insert(capability.contribution_id.clone())
            {
                return Err(catalog_invalid(format!(
                    "duplicate contribution identity {}",
                    capability.contribution_id.as_ref()
                )));
            }
        }
        for (miniapp_id, publication) in &self.miniapp_publications {
        publication
            .validate()
            .map_err(|error| catalog_invalid(error.to_string()))?;
            if miniapp_id != &publication.miniapp_id {
                return Err(catalog_invalid(
                    "MiniApp Catalog publication map key differs from its payload identity",
                ));
            }
            for capability in &publication.capabilities {
                let reference = capability.entry.capability.clone();
                if !capabilities.insert(reference.clone()) {
                    return Err(catalog_invalid(format!(
                        "duplicate materialized capability {}@{}",
                        reference.id.as_ref(),
                        reference.version.as_ref()
                    )));
                }
                if !contribution_ids.insert(capability.entry.contribution_id.clone()) {
                    return Err(catalog_invalid(format!(
                        "duplicate contribution identity {}",
                        capability.entry.contribution_id.as_ref()
                    )));
                }
                let entry = self
                    .formal_capability_entries
                    .get(&reference)
                    .ok_or_else(|| {
                        catalog_invalid(format!(
                            "MiniApp capability {}@{} has no formal Catalog entry",
                            reference.id.as_ref(),
                            reference.version.as_ref()
                        ))
                    })?;
                if entry != &capability.entry {
                    return Err(catalog_invalid(format!(
                        "MiniApp capability {}@{} differs from its formal Catalog entry",
                        reference.id.as_ref(),
                        reference.version.as_ref()
                    )));
                }
            }
        }
        if let Some(reference) = self
            .formal_capability_entries
            .keys()
            .find(|reference| !capabilities.contains(*reference))
        {
            return Err(catalog_invalid(format!(
                "formal Catalog entry {}@{} has no materialized publication",
                reference.id.as_ref(),
                reference.version.as_ref()
            )));
        }
        let derived_unavailable = self
            .formal_capability_entries
            .values()
            .filter_map(|entry| {
                entry
                    .availability_for(CapabilityConsumer::Agent)
                    .and_then(availability_code)
                    .map(|code| (entry.capability.id.clone(), code))
            })
            .collect::<BTreeMap<_, _>>();
        if self.unavailable_capabilities != derived_unavailable {
            return Err(catalog_invalid(
                "unavailable capability index differs from formal Catalog availability",
            ));
        }

        let mut skills = BTreeSet::new();
        for skill in &self.skills {
            let reference = SkillRef {
                id: skill.definition.id.clone(),
                version: skill.definition.version.clone(),
            };
            if !skills.insert(reference.clone()) {
                return Err(catalog_invalid(format!(
                    "duplicate materialized Skill {}@{}",
                    reference.id.as_ref(),
                    reference.version.as_ref()
                )));
            }
            validate_skill(skill)?;
            if !contribution_ids.insert(skill.contribution_id.clone()) {
                return Err(catalog_invalid(format!(
                    "duplicate contribution identity {}",
                    skill.contribution_id.as_ref()
                )));
            }
        }

        let mut mcp_tools = BTreeSet::new();
        let mut mcp_capabilities = BTreeSet::new();
        for mcp in &self.mcp_tools {
            let key = (
                mcp.mapping.server_id.clone(),
                mcp.mapping.canonical_tool_key.clone(),
            );
            if !mcp_tools.insert(key.clone()) {
                return Err(catalog_invalid(format!(
                    "duplicate materialized MCP binding {}/{}",
                    key.0.as_ref(),
                    key.1.as_ref()
                )));
            }
            if !mcp_capabilities.insert(mcp.mapping.capability.clone()) {
                return Err(catalog_invalid(format!(
                    "capability {}@{} has more than one MCP binding",
                    mcp.mapping.capability.id.as_ref(),
                    mcp.mapping.capability.version.as_ref()
                )));
            }
            let capability = self
                .materialized_capability(&mcp.mapping.capability)
                .ok_or_else(|| {
                    catalog_invalid(format!(
                        "MCP binding {}/{} targets a missing capability {}@{}",
                        key.0.as_ref(),
                        key.1.as_ref(),
                        mcp.mapping.capability.id.as_ref(),
                        mcp.mapping.capability.version.as_ref()
                    ))
                })?;
            validate_mcp_tool(mcp, capability)?;
        }
        if let Some(capability) = self.capabilities.iter().find(|capability| {
            capability.contribution_lock.source_kind
                == ContributionSourceKind::McpBinding
                && !mcp_capabilities.contains(&capability_reference(capability))
        }) {
            return Err(catalog_invalid(format!(
                "MCP-backed capability {}@{} has no materialized binding",
                capability.manifest.id.as_ref(),
                capability.manifest.version.as_ref()
            )));
        }
        Ok(())
    }

    pub fn materialized_capability(
        &self,
        reference: &CapabilityRef,
    ) -> Option<&MaterializedCapability> {
        self.capabilities.iter().find(|capability| {
            capability.manifest.id == reference.id
                && capability.manifest.version == reference.version
        })
    }

    pub fn find_capability(
        &self,
        reference: &CapabilityRef,
    ) -> Option<&nomifun_agent_contracts::CapabilityManifest> {
        if let Some(capability) = self.materialized_capability(reference) {
            if !capability
                .manifest
                .supports_consumer(CapabilityConsumer::Agent)
            {
                return None;
            }
            if capability.source.source_kind != PluginSourceKind::TestFixture
                && !self.formal_capability_entries.contains_key(reference)
            {
                return None;
            }
            return Some(&capability.manifest);
        }
        self.miniapp_publications
            .values()
            .flat_map(|publication| publication.capabilities.iter())
            .find(|publication| {
                publication.entry.capability == *reference
                    && publication
                        .entry
                        .supports_consumer(CapabilityConsumer::Agent)
            })
            .map(|publication| &publication.manifest)
    }

    pub fn capability_catalog_entry(
        &self,
        reference: &CapabilityRef,
    ) -> Result<Option<CapabilityCatalogEntry>, ControlPlaneError> {
        if let Some(capability) = self.materialized_capability(reference) {
            if capability.source.source_kind == PluginSourceKind::TestFixture {
                return Ok(None);
            }
            let entry = self
                .formal_capability_entries
                .get(reference)
                .ok_or_else(|| {
                    catalog_invalid(format!(
                        "materialized capability {}@{} has no formal Catalog entry",
                        reference.id.as_ref(),
                        reference.version.as_ref()
                    ))
                })?;
            validate_capability_entry(capability, entry)?;
            return Ok(Some(entry.clone()));
        }
        Ok(self
            .miniapp_publications
            .values()
            .flat_map(|publication| publication.capabilities.iter())
            .find(|publication| publication.entry.capability == *reference)
            .map(|publication| publication.entry.clone()))
    }

    pub fn materialized_skill(
        &self,
        reference: &SkillRef,
    ) -> Option<&MaterializedSkill> {
        self.skills
            .iter()
            .find(|skill| {
                skill.definition.id == reference.id
                    && skill.definition.version == reference.version
            })
    }

    pub fn find_skill(
        &self,
        reference: &SkillRef,
    ) -> Option<&nomifun_agent_contracts::SkillDefinition> {
        self.materialized_skill(reference)
            .map(|skill| &skill.definition)
    }

    pub fn materialized_mcp_tool(
        &self,
        server_id: &McpServerId,
        tool_key: &McpToolKey,
    ) -> Option<&MaterializedMcpTool> {
        self.mcp_tools.iter().find(|mcp| {
            &mcp.mapping.server_id == server_id
                && &mcp.mapping.canonical_tool_key == tool_key
        })
    }

    pub fn as_api(&self) -> Result<AgentCatalogResponse, ControlPlaneError> {
        self.validate()?;
        let capabilities = self
            .capabilities
            .iter()
            .filter(|capability| {
                capability
                    .manifest
                    .supports_consumer(CapabilityConsumer::Agent)
                    && capability.source.source_kind != PluginSourceKind::TestFixture
            })
            .map(|capability| {
                let manifest = &capability.manifest;
                let reference = CapabilityRef {
                    id: manifest.id.clone(),
                    version: manifest.version.clone(),
                };
                let entry = self
                    .capability_catalog_entry(&reference)?
                    .ok_or_else(|| {
                        ControlPlaneError::canonical(
                            "CAPABILITY_NOT_MATERIALIZED",
                            axum::http::StatusCode::NOT_FOUND,
                            format!(
                                "capability {}@{} is not materialized",
                                manifest.id.as_ref(),
                                manifest.version.as_ref()
                            ),
                        )
                    })?;
                capability_catalog_item(
                    manifest,
                    &entry,
                    wire_name(&capability.source.source_kind)?,
                )
            })
            .collect::<Result<Vec<_>, ControlPlaneError>>()?;
        let mut capabilities = capabilities;
        for publication in self.miniapp_publications.values() {
            for capability in &publication.capabilities {
                if capability
                    .entry
                    .supports_consumer(CapabilityConsumer::Agent)
                {
                    capabilities.push(capability_catalog_item(
                        &capability.manifest,
                        &capability.entry,
                        wire_name(&capability.entry.provenance.source_kind)?,
                    )?);
                }
            }
        }
        capabilities.sort_by(|left, right| {
            left.capability
                .id
                .cmp(&right.capability.id)
                .then_with(|| left.capability.version.cmp(&right.capability.version))
        });

        let skills = self
            .skills
            .iter()
            .filter(|skill| {
                skill.source.source_kind != PluginSourceKind::TestFixture
            })
            .map(|skill| {
                let definition = &skill.definition;
                Ok(SkillCatalogItemDto {
                    skill: ExactCatalogRefDto {
                        id: definition.id.as_ref().to_owned(),
                        version: definition.version.as_ref().to_owned(),
                    },
                    display_name: definition.display.name.clone(),
                    description: definition.display.description.clone(),
                    source_package: ExactCatalogRefDto {
                        id: definition.package.id.as_ref().to_owned(),
                        version: definition.package.version.as_ref().to_owned(),
                    },
                    source_kind: wire_name(&skill.source.source_kind)?,
                    required_capabilities: definition
                        .requires_capabilities
                        .iter()
                        .map(|reference| ExactCatalogRefDto {
                            id: reference.id.as_ref().to_owned(),
                            version: reference.version.as_ref().to_owned(),
                        })
                        .collect(),
                    supported_surfaces: definition.supported_surfaces.clone(),
                })
            })
            .collect::<Result<Vec<_>, ControlPlaneError>>()?;

        let mcp_tools = self
            .mcp_tools
            .iter()
            .filter(|mcp| {
                mcp.source.source_kind != PluginSourceKind::TestFixture
            })
            .map(|mcp| {
                let mapping = &mcp.mapping;
                McpToolCatalogItemDto {
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
            }})
            .collect();

        Ok(AgentCatalogResponse {
            capabilities,
            skills,
            mcp_tools,
        })
    }
}

fn capability_catalog_item(
    manifest: &nomifun_agent_contracts::CapabilityManifest,
    entry: &CapabilityCatalogEntry,
    source_kind: String,
) -> Result<CapabilityCatalogItemDto, ControlPlaneError> {
    let unavailable_code = match entry.availability_for(CapabilityConsumer::Agent) {
        Some(CatalogAvailability::Active) => None,
        Some(CatalogAvailability::Unavailable { reason })
        | Some(CatalogAvailability::Disabled { reason }) => Some(reason.clone()),
        Some(CatalogAvailability::NeedsRuntime { .. }) => {
            Some("CAPABILITY_NEEDS_RUNTIME".to_owned())
        }
        Some(CatalogAvailability::ContractMismatch { .. }) => {
            Some("CAPABILITY_CONTRACT_MISMATCH".to_owned())
        }
        None => Some("CAPABILITY_CONSUMER_UNSUPPORTED".to_owned()),
    };
    Ok(CapabilityCatalogItemDto {
        capability: ExactCatalogRefDto {
            id: manifest.id.as_ref().to_owned(),
            version: manifest.version.as_ref().to_owned(),
        },
        kind: wire_name(&manifest.kind)?,
        display_name: manifest.display.name.clone(),
        description: manifest.display.description.clone(),
        source_package: ExactCatalogRefDto {
            id: manifest.package.id.as_ref().to_owned(),
            version: manifest.package.version.as_ref().to_owned(),
        },
        source_kind,
        materialization_state: if unavailable_code.is_some() {
            CatalogMaterializationStateDto::Unavailable
        } else {
            CatalogMaterializationStateDto::Materialized
        },
        unavailable_code,
        supported_surfaces: entry.host_surfaces.clone(),
        required_runtime_features: manifest
            .requires_runtime_features
            .iter()
            .map(|feature| feature.id.as_ref().to_owned())
            .collect(),
        required_resource_kinds: manifest
            .contributions
            .resource_kinds
            .iter()
            .map(|kind| kind.as_ref().to_owned())
            .collect(),
        required_capabilities: manifest
            .requires
            .iter()
            .map(|reference| ExactCatalogRefDto {
                id: reference.id.as_ref().to_owned(),
                version: reference.version.as_ref().to_owned(),
            })
            .collect(),
        conflicting_capabilities: manifest
            .conflicts
            .iter()
            .map(|conflict| ExactCatalogRefDto {
                id: conflict.capability.id.as_ref().to_owned(),
                version: conflict.capability.version.as_ref().to_owned(),
            })
            .collect(),
        action_count: manifest.contributions.actions.len() as u32,
        context_contributor_count: manifest
            .contributions
            .context_schema_refs
            .len() as u32,
    })
}

fn capability_reference(capability: &MaterializedCapability) -> CapabilityRef {
    CapabilityRef {
        id: capability.manifest.id.clone(),
        version: capability.manifest.version.clone(),
    }
}

fn validate_capability_entry(
    capability: &MaterializedCapability,
    entry: &CapabilityCatalogEntry,
) -> Result<(), ControlPlaneError> {
    entry
        .validate()
        .map_err(|error| catalog_invalid(error.to_string()))?;
    capability
        .contribution_lock
        .validate()
        .map_err(|error| catalog_invalid(error.message))?;
    let owner_matches = matches!(
        &entry.provenance.owner,
        CapabilityOwner::Package { package }
            if package == &capability.manifest.package
    );
    if entry.capability != capability_reference(capability)
        || entry.contract_digest != capability.schema_digest
        || entry.contribution_id != capability.contribution_id
        || entry.provenance.source_kind
            != capability.contribution_lock.source_kind
        || entry.provenance.source_identity
            != capability.contribution_lock.source_identity
        || entry.provenance.mount_id
            != capability.contribution_lock.mount_id
        || entry.provenance.miniapp_id
            != capability.contribution_lock.miniapp_id
        || entry.provenance.mcp_binding_id
            != capability.contribution_lock.mcp_binding_id
        || entry.provenance.artifact_digest.as_ref()
            != Some(&capability.target_artifact_digest)
        || capability.contribution_lock.contract_digest
            != capability.schema_digest
        || !owner_matches
    {
        return Err(catalog_invalid(format!(
            "formal Catalog entry {}@{} differs from the exact Kernel materialization",
            capability.manifest.id.as_ref(),
            capability.manifest.version.as_ref()
        )));
    }
    Ok(())
}

fn validate_skill(skill: &MaterializedSkill) -> Result<(), ControlPlaneError> {
    skill
        .contribution_lock
        .validate()
        .map_err(|error| catalog_invalid(error.message))?;
    let contract_digest = digest_payload(&skill.definition)
        .map_err(|error| catalog_invalid(error.to_string()))?;
    let expected_contribution_id = format!("skill:{}", skill.definition.id.as_ref());
    let exact_source = match skill.source.source_kind {
        PluginSourceKind::ManagedLocal => {
            skill.contribution_lock.source_kind
                == ContributionSourceKind::PluginMount
                && skill.contribution_lock.mount_id.as_ref()
                    == Some(&skill.mount_id)
        }
        PluginSourceKind::Bundled | PluginSourceKind::TestFixture => {
            skill.contribution_lock.source_kind
                == ContributionSourceKind::PlatformBuiltin
                && skill.contribution_lock.mount_id.is_none()
        }
    };
    if skill.contribution_id.as_ref() != expected_contribution_id
        || skill.contribution_lock.contribution_id != skill.contribution_id
        || skill.contract_digest != contract_digest
        || skill.contribution_lock.contract_digest != contract_digest
        || skill.contribution_lock.source_identity.as_ref()
            != skill.source.source_identity
        || skill.contribution_lock.mcp_binding_id.is_some()
        || skill.contribution_lock.miniapp_id.is_some()
        || skill
            .source
            .source_digest
            .as_ref()
            .is_some_and(|digest| digest != &skill.target_artifact_digest)
        || !is_digest(&skill.target_artifact_digest)
        || !exact_source
    {
        return Err(catalog_invalid(format!(
            "materialized Skill {}@{} has inconsistent owner/provenance/contract/Artifact facts",
            skill.definition.id.as_ref(),
            skill.definition.version.as_ref()
        )));
    }
    Ok(())
}

fn validate_mcp_tool(
    mcp: &MaterializedMcpTool,
    capability: &MaterializedCapability,
) -> Result<(), ControlPlaneError> {
    mcp.contribution_lock
        .validate()
        .map_err(|error| catalog_invalid(error.message))?;
    let expected_binding = McpBindingId::from(format!(
        "{}:{}",
        mcp.mapping.server_id.as_ref(),
        mcp.mapping.canonical_tool_key.as_ref()
    ));
    let expected_mount = match mcp.source.source_kind {
        PluginSourceKind::ManagedLocal => Some(&mcp.mount_id),
        PluginSourceKind::Bundled | PluginSourceKind::TestFixture => None,
    };
    if mcp.mapping.package != capability.manifest.package
        || mcp.mapping.capability != capability_reference(capability)
        || mcp.binding_id != expected_binding
        || mcp.contribution_lock != capability.contribution_lock
        || mcp.target_artifact_digest
            != capability.target_artifact_digest
        || mcp.mount_id != capability.mount_id
        || mcp.source.source_kind != capability.source.source_kind
        || mcp.source.source_identity != capability.source.source_identity
        || mcp.source.source_digest != capability.source.source_digest
        || mcp.contribution_lock.source_kind
            != ContributionSourceKind::McpBinding
        || mcp.contribution_lock.mcp_binding_id.as_ref()
            != Some(&expected_binding)
        || mcp.contribution_lock.mount_id.as_ref() != expected_mount
        || !is_digest(&mcp.mapping.schema_digest)
        || !is_digest(&mcp.target_artifact_digest)
    {
        return Err(catalog_invalid(format!(
            "materialized MCP binding {}/{} differs from its exact Capability/Mount/Artifact facts",
            mcp.mapping.server_id.as_ref(),
            mcp.mapping.canonical_tool_key.as_ref()
        )));
    }
    Ok(())
}

fn is_digest(value: &nomifun_agent_contracts::DigestHex) -> bool {
    value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn catalog_invalid(message: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "CAPABILITY_CATALOG_INVALID",
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        message,
    )
}

fn availability_code(
    availability: &CatalogAvailability,
) -> Option<CanonicalErrorCode> {
    match availability {
        CatalogAvailability::Active => None,
        CatalogAvailability::Unavailable { reason }
        | CatalogAvailability::Disabled { reason } => {
            Some(CanonicalErrorCode::from(reason.clone()))
        }
        CatalogAvailability::NeedsRuntime { .. } => {
            Some(CanonicalErrorCode::from("CAPABILITY_NEEDS_RUNTIME"))
        }
        CatalogAvailability::ContractMismatch { .. } => Some(
            CanonicalErrorCode::from("CAPABILITY_CONTRACT_MISMATCH"),
        ),
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
        self.snapshot.validate()?;
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
