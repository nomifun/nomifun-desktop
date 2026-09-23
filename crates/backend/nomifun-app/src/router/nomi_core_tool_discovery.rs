//! Bundled discovery Role in the existing Nomi Catalog. All execution and
//! schema ownership stays behind the nomifun-ai-agent seam.
use nomifun_agent_contracts::*;
use nomifun_agent_domain_support::{CapabilitySpec, PackageSpec};
use nomifun_agent_kernel::PluginRegistration;
use nomifun_ai_agent::tool_discovery::{self, CAPABILITY_ID, PACKAGE_ID, ROLE_ID};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const TOOL_DISCOVERY_MOUNT_ID: &str = "nomifun-tool-discovery";

pub(crate) fn validate_snapshot(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), nomifun_agent_control_plane::ControlPlaneError> {
    tool_discovery::validate_selection(registry, &snapshot.content)
        .map_err(|error| {
        nomifun_agent_control_plane::ControlPlaneError::canonical(
            "CAPABILITY_UNAVAILABLE",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            error.to_string(),
        )
    })
}

pub(crate) fn registration() -> anyhow::Result<PluginRegistration> {
    const CAPABILITIES: &[CapabilitySpec] =
        &[CapabilitySpec::tool(CAPABILITY_ID, EffectClass::Pure, &[])];
    let base = nomifun_agent_domain_support::registration(PackageSpec {
        id: PACKAGE_ID,
        mount_id: TOOL_DISCOVERY_MOUNT_ID,
        display_name: "Tool discovery",
        description: "Select a pure discovery/ranking policy for the Session's authorized tools.",
        capabilities: CAPABILITIES,
        supported_surfaces: &["desktop", "headless"],
    })?;
    let mut metadata = base.metadata;
    let manifest = &mut metadata.manifest.payload;
    let capability = &mut manifest.contributions.capabilities[0];
    capability.display.name = "Tool discovery policy".into();
    capability.display.description = "Select how ToolSearch discovers authorized tools; schemas and activation remain host-owned.".into();
    capability.contributions.actions = vec![tool_discovery::action()];
    let role = RoleContractManifest {
        key: RoleContractKey {
            role_id: ROLE_ID.into(),
            contract_version: "1.0.0".into(),
        },
        members: vec![RoleMemberContract {
            capability: CapabilityRef {
                id: CAPABILITY_ID.into(),
            },
            capability_manifest_digest: digest_payload(capability)?,
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    manifest.contributions.role_providers = vec![RoleProviderContribution {
        role: ExactRoleContractRef {
            key: role.key.clone(),
            contract_digest: digest_payload(&role)?,
        },
        display: manifest.display.clone(),
        members: BTreeMap::from([(
            CAPABILITY_ID.into(),
            RoleProviderMemberContribution {
                implementation: None,
                supported_platforms: vec![PlatformConstraint::Any],
                required_resource_kinds: BTreeSet::new(),
            },
        )]),
    }];
    manifest.contributions.role_contracts = vec![role];
    metadata.manifest = ArtifactEnvelope::new(metadata.manifest.payload)?;
    metadata.registrar.declared_role_ids.insert(ROLE_ID.into());
    metadata
        .registrar
        .allowed_operations
        .insert(PluginRegistrarOperation::ContributeRoleProvider);
    let mut registration = PluginRegistration::new(metadata);
    registration.add_role_action_handler(
        ROLE_ID.into(),
        CAPABILITY_ID.into(),
        Arc::new(tool_discovery::BuiltinDiscovery),
    )?;
    Ok(registration)
}

/// Select the bundled discovery policy for every host boot. Agents may declare
/// Tool Discovery without asking the user to configure an internal Role; an
/// explicitly selected compatible provider can still replace this default.
pub(crate) fn installation_binding(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
) -> anyhow::Result<BTreeMap<ExecutionRoleId, InstallationRoleBinding>> {
    let role_id = ExecutionRoleId::from(ROLE_ID);
    let mount_id = AgentModuleId::from(TOOL_DISCOVERY_MOUNT_ID);
    let installed = registry.role_provider(&role_id, &mount_id).ok_or_else(|| {
        anyhow::anyhow!("Bundled Tool Discovery Role Provider is missing from the host registry")
    })?;
    anyhow::ensure!(
        installed.provider.role.key.role_id == role_id,
        "Bundled Tool Discovery Provider has the wrong Role contract"
    );
    anyhow::ensure!(
        installed.source.source_kind == PluginSourceKind::Bundled,
        "Bundled Tool Discovery Provider must use the bundled source"
    );
    Ok(BTreeMap::from([(
        role_id,
        InstallationRoleBinding {
            selection: RoleProviderSelection {
                role: installed.provider.role.clone(),
                provider_mount_id: installed.provider.mount_id.clone(),
            },
            binding_version: 1,
            updated_at_ms: 0,
        },
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy,
    };

    #[test]
    fn bundled_discovery_has_one_real_hidden_role_export() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable("1.0.0"),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap();
        let materialized = registry.replace_all(vec![registration().unwrap()]).unwrap();
        let capability = materialized.capability(&CAPABILITY_ID.into()).unwrap();
        assert!(tool_discovery::supports(&capability.manifest));
        assert_eq!(
            capability.manifest.contributions.actions[0]
                .action_id
                .as_ref(),
            tool_discovery::ACTION_ID
        );
        assert_eq!(materialized.role_providers.len(), 1);
        assert_eq!(tool_discovery::schemas().len(), 2);
        let binding = installation_binding(&materialized).unwrap();
        assert_eq!(binding.len(), 1);
        assert_eq!(
            binding[&ExecutionRoleId::from(ROLE_ID)]
                .selection
                .provider_mount_id
                .as_ref(),
            TOOL_DISCOVERY_MOUNT_ID,
        );
    }
}
