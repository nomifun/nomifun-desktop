//! Engine-independent capability catalog projection from the shared Kernel.
//!
//! Catalog reads must not depend on a Session executor or an external runtime.

#[cfg(test)]
#[path = "kernel_catalog_tests.rs"]
mod tests;

use crate::{CatalogProvider, CatalogSnapshot, ControlPlaneError, PluginProductCatalogPublicationSource};
use nomifun_agent_contracts::{
    CapabilityCatalogContractError, CapabilityCatalogEntry, CapabilityCatalogMaterialization,
    CapabilityCatalogMaterializer, CapabilityConsumer, CapabilityId, CapabilityOwner,
    CapabilityProvenance, CapabilityReleaseState, CatalogAvailability,
};
use nomifun_agent_kernel::KernelRegistry;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock as StdRwLock};

pub fn materialize_capability_catalog_entries(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    unavailable_capabilities: &BTreeMap<CapabilityId, nomifun_agent_contracts::CanonicalErrorCode>,
) -> Result<Vec<CapabilityCatalogEntry>, ControlPlaneError> {
    let mut entries = Vec::new();
    for capability in registry.capabilities.values() {
        let manifest = &capability.manifest;
        let release_state = match capability.source.source_kind {
            nomifun_agent_contracts::PluginSourceKind::TestFixture => {
                CapabilityReleaseState::TestHost
            }
            nomifun_agent_contracts::PluginSourceKind::Bundled
            | nomifun_agent_contracts::PluginSourceKind::ManagedLocal => {
                CapabilityReleaseState::PublishedActive
            }
        };
        let lock = &capability.contribution_lock;
        let provenance = CapabilityProvenance {
            owner: CapabilityOwner::Package {
                package: manifest.package.clone(),
            },
            source_kind: lock.source_kind,
            source_identity: lock.source_identity.clone(),
            mount_id: lock.mount_id.clone(),
            plugin_product_id: lock.plugin_product_id.clone(),
            mcp_binding_id: lock.mcp_binding_id.clone(),
            artifact_digest: Some(capability.target_artifact_digest.clone()),
        };
        let consumers = manifest
            .supported_consumers()
            .map_err(|reason| ControlPlaneError::Wire(reason))?;
        let mut availability = consumers
            .iter()
            .copied()
            .map(|consumer| (consumer, CatalogAvailability::Active))
            .collect::<BTreeMap<_, _>>();
        if let Some(code) = unavailable_capabilities.get(&manifest.id)
            && consumers.contains(&CapabilityConsumer::Agent)
        {
            availability.insert(
                CapabilityConsumer::Agent,
                CatalogAvailability::Unavailable {
                    reason: code.as_ref().to_owned(),
                },
            );
        }
        match CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
            manifest: manifest.clone(),
            provenance,
            release_state,
            availability,
        }) {
            Ok(entry) => entries.push(entry),
            Err(CapabilityCatalogContractError::CandidateRejected { .. }) => {}
            Err(error) => {
                return Err(ControlPlaneError::Wire(error.to_string()));
            }
        }
    }
    entries.sort_by(|left, right| left.capability.cmp(&right.capability));
    Ok(entries)
}

pub fn materialize_catalog_snapshot(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    unavailable_capabilities: &BTreeMap<CapabilityId, nomifun_agent_contracts::CanonicalErrorCode>,
) -> Result<CatalogSnapshot, ControlPlaneError> {
    materialize_catalog_snapshot_with_plugin_products(registry, unavailable_capabilities, Vec::new())
}

/// Compatibility name: publications use the unified Plugin Product contract.
/// Catalog availability is not a Session permission grant.
pub fn materialize_catalog_snapshot_with_plugin_products(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    unavailable_capabilities: &BTreeMap<CapabilityId, nomifun_agent_contracts::CanonicalErrorCode>,
    plugin_product_publications: Vec<nomifun_agent_contracts::PluginProductCapabilityCatalogPublication>,
) -> Result<CatalogSnapshot, ControlPlaneError> {
    let mut formal_capability_entries =
        materialize_capability_catalog_entries(registry, unavailable_capabilities)?
            .into_iter()
            .map(|entry| (entry.capability.clone(), entry))
            .collect::<BTreeMap<_, _>>();
    let mut plugin_product_publication_map = BTreeMap::new();
    for publication in plugin_product_publications {
        publication
            .validate()
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if plugin_product_publication_map
            .insert(publication.plugin_product_id.clone(), publication.clone())
            .is_some()
        {
            return Err(ControlPlaneError::Wire(
                "duplicate Plugin Product Catalog publication".to_owned(),
            ));
        }
        for capability in publication.capabilities {
            if formal_capability_entries
                .insert(capability.entry.capability.clone(), capability.entry)
                .is_some()
            {
                return Err(ControlPlaneError::Wire(
                    "Plugin Product capability conflicts with an existing Catalog entry".to_owned(),
                ));
            }
        }
    }
    let derived_unavailable_capabilities = formal_capability_entries
        .values()
        .filter_map(|entry| {
            entry
                .availability_for(CapabilityConsumer::Agent)
                .and_then(|availability| match availability {
                    CatalogAvailability::Active => None,
                    CatalogAvailability::Unavailable { reason }
                    | CatalogAvailability::Disabled { reason } => Some(
                        nomifun_agent_contracts::CanonicalErrorCode::from(reason.clone()),
                    ),
                    CatalogAvailability::NeedsRuntime { .. } => {
                        Some(nomifun_agent_contracts::CanonicalErrorCode::from(
                            "CAPABILITY_NEEDS_RUNTIME",
                        ))
                    }
                    CatalogAvailability::ContractMismatch { .. } => {
                        Some(nomifun_agent_contracts::CanonicalErrorCode::from(
                            "CAPABILITY_CONTRACT_MISMATCH",
                        ))
                    }
                })
                .map(|code| (entry.capability.id.clone(), code))
        })
        .collect();
    let snapshot = CatalogSnapshot {
        role_contracts: registry.role_contracts.values().cloned().collect(),
        role_providers: registry.role_providers.values().cloned().collect(),
        capabilities: registry.capabilities.values().cloned().collect(),
        formal_capability_entries,
        plugin_product_publications: plugin_product_publication_map,
        skills: registry.skills.values().cloned().collect(),
        mcp_tools: registry.mcp_tools.values().cloned().collect(),
        unavailable_capabilities: derived_unavailable_capabilities,
        service_key_diagnostics: Vec::new(),
    };
    snapshot.validate()?;
    Ok(snapshot)
}

pub struct KernelCatalogProvider {
    registry: Arc<KernelRegistry>,
    unavailable_capabilities:
        StdRwLock<BTreeMap<CapabilityId, nomifun_agent_contracts::CanonicalErrorCode>>,
    plugin_product_publications: Option<Arc<dyn PluginProductCatalogPublicationSource>>,
}

impl KernelCatalogProvider {
    pub fn new(registry: Arc<KernelRegistry>) -> Self {
        Self {
            registry,
            unavailable_capabilities: StdRwLock::new(BTreeMap::new()),
            plugin_product_publications: None,
        }
    }

    /// Compatibility name for the unified Plugin Product publication source.
    pub fn with_plugin_product_publication_source(
        mut self,
        source: Arc<dyn PluginProductCatalogPublicationSource>,
    ) -> Self {
        self.plugin_product_publications = Some(source);
        self
    }

    /// Mark capability identities unavailable for a specific host composition
    /// without removing their canonical manifests from the catalog.  This is
    /// used by Nomi-core for capabilities whose declarative contract is known
    /// but whose current runtime has no safe capability owner port.
    pub fn with_unavailable_capabilities(
        mut self,
        unavailable: impl IntoIterator<
            Item = (CapabilityId, nomifun_agent_contracts::CanonicalErrorCode),
        >,
    ) -> Self {
        self.unavailable_capabilities = StdRwLock::new(unavailable.into_iter().collect());
        self
    }

    pub fn replace_unavailable_capabilities(
        &self,
        unavailable: impl IntoIterator<
            Item = (CapabilityId, nomifun_agent_contracts::CanonicalErrorCode),
        >,
    ) -> Result<(), ControlPlaneError> {
        *self.unavailable_capabilities.write().map_err(|_| {
            ControlPlaneError::Wire("Kernel Catalog availability registry is poisoned".to_owned())
        })? = unavailable.into_iter().collect();
        Ok(())
    }
}

impl CatalogProvider for KernelCatalogProvider {
    fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, ControlPlaneError> {
        let registry = self
            .registry
            .snapshot()
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let unavailable_capabilities = self
            .unavailable_capabilities
            .read()
            .map_err(|_| {
                ControlPlaneError::Wire(
                    "Kernel Catalog availability registry is poisoned".to_owned(),
                )
            })?
            .clone();
        let plugin_product_publications = self
            .plugin_product_publications
            .as_ref()
            .map(|source| source.publications())
            .transpose()?
            .unwrap_or_default();
        materialize_catalog_snapshot_with_plugin_products(
            &registry,
            &unavailable_capabilities,
            plugin_product_publications,
        )
        .map(Arc::new)
    }
}
