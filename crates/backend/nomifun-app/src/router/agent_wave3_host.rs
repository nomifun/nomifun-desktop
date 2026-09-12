//! Canonical Nomi composition for the Creative Studio Wave 3 capabilities.
//!
//! This is the only application seam that declares which Wave 3 bundled Tools
//! have real Nomi owners. Office remains a typed candidate and is intentionally
//! not admitted through this composition.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    CanonicalSchemaRef, CapabilityId, ResolvedCapability, StrictJsonValue,
};
use nomifun_agent_domain_wave3::{Wave3OwnerBindings, composed_host_port};
use nomifun_agent_kernel::PluginRegistration;
use nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver;
use nomifun_creation::CreationService;
use nomifun_miniapp_platform::MiniAppM1ApplicationService;
use nomifun_workshop::WorkshopService;

use super::agent_wave3_creation_host::Wave3CreationHost;
use super::agent_wave3_miniapp_host::NomiWave3MiniAppHost;
use super::agent_wave3_workshop_host::NomiWave3WorkshopHost;

pub(crate) const NOMI_WAVE3_TOOL_IDS: [&str; 14] = [
    "creation.text",
    "creation.image",
    "creation.image_edit",
    "creation.video",
    "creation.audio",
    "workshop.canvas.read",
    "workshop.canvas.edit",
    "workshop.asset.read",
    "workshop.asset.write",
    "workshop.template.run",
    "miniapp.read",
    "miniapp.edit",
    "miniapp.publish",
    "miniapp.serve",
];

pub(crate) fn approved_capability_ids() -> BTreeSet<CapabilityId> {
    NOMI_WAVE3_TOOL_IDS
        .into_iter()
        .map(CapabilityId::from)
        .collect()
}

/// Build host-backed registrations from the exact application service
/// singletons already used by the Creative Studio and MiniApp HTTP products.
pub(crate) fn registrations(
    creation: Arc<CreationService>,
    workshop: Arc<WorkshopService>,
    miniapp: Arc<MiniAppM1ApplicationService>,
) -> Result<Vec<PluginRegistration>, String> {
    let workshop_host = NomiWave3WorkshopHost::new(
        Arc::clone(&workshop),
        Arc::clone(&creation),
    );
    let host = composed_host_port(
        Wave3OwnerBindings::default()
            .with_creation(
                Wave3CreationHost::new(creation, Arc::clone(&workshop)).into_host_port(),
            )
            .with_workshop(workshop_host.into_port())
            .with_miniapp(NomiWave3MiniAppHost::new(miniapp).into_port()),
    );
    nomifun_agent_domain_wave3::registrations_with_host_port(host)
}

/// Resolve only the schema bytes belonging to the exact Wave 3 target locked
/// into a Snapshot. Source/package admission is enforced by the Nomi bundled
/// Tool bridge before calling this function.
pub(crate) fn resolve_schema(
    capability: &ResolvedCapability,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    if !approved_capability_ids().contains(&capability.capability.id) {
        return Err(format!(
            "{} is not an approved Nomi Wave 3 Tool",
            capability.capability.id.as_ref()
        ));
    }
    nomifun_agent_domain_wave3::resolve_action_schema(
        capability.capability.id.as_ref(),
        reference,
    )
}

pub(crate) struct NomiWave3SchemaResolver;

#[async_trait]
impl NomiPlatformBuiltinToolSchemaResolver for NomiWave3SchemaResolver {
    async fn resolve(
        &self,
        capability: &ResolvedCapability,
        reference: &CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        resolve_schema(capability, reference)
    }
}

pub(crate) fn schema_resolver() -> Arc<dyn NomiPlatformBuiltinToolSchemaResolver> {
    Arc::new(NomiWave3SchemaResolver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_set_is_exact_and_never_revives_director_or_office() {
        let approved = approved_capability_ids();
        assert_eq!(approved.len(), NOMI_WAVE3_TOOL_IDS.len());
        assert!(!approved.contains(&CapabilityId::from("workshop.director")));
        assert!(approved.iter().all(|id| {
            id.as_ref().starts_with("creation.")
                || id.as_ref().starts_with("workshop.")
                || id.as_ref().starts_with("miniapp.")
        }));
    }
}
