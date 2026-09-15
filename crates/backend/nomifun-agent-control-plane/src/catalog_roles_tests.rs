use super::*;
use nomifun_agent_contracts::{
    ExactRoleContractRef, ExactRoleProviderRef, LocalizedMetadata, PackageRef,
    PluginSourceMetadata, RoleContractKey, RoleContractManifest, RoleMemberContract,
    RoleMemberRequirement, RoleProviderContribution, RoleProviderMemberContribution,
};

fn fixture() -> (CatalogSnapshot, Vec<CapabilityCatalogItemDto>) {
    let member = CapabilityRef {
        id: "platform.search".into(),
        version: "1.0.0".into(),
    };
    let manifest = RoleContractManifest {
        key: RoleContractKey {
            role_id: "search".into(),
            contract_version: "1.0.0".into(),
        },
        members: vec![RoleMemberContract {
            capability: member.clone(),
            capability_manifest_digest: "a".repeat(64).into(),
            requirement: RoleMemberRequirement::Required,
        }],
        serialized_target_resource_kind: None,
    };
    let contract_digest = digest_payload(&manifest).unwrap();
    let role = ExactRoleContractRef {
        key: manifest.key.clone(),
        contract_digest: contract_digest.clone(),
    };
    let mut catalog = CatalogSnapshot {
        role_contracts: vec![MaterializedRoleContract {
            manifest,
            contract_digest,
            mount_id: "contract".into(),
        }],
        ..Default::default()
    };
    for (mount, source_kind) in [
        ("user", PluginSourceKind::ManagedLocal),
        ("builtin", PluginSourceKind::Bundled),
        ("test", PluginSourceKind::TestFixture),
    ] {
        let contribution = RoleProviderContribution {
            role: role.clone(),
            display: LocalizedMetadata {
                name: mount.into(),
                description: format!("{mount} search"),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            members: BTreeMap::from([(
                member.id.clone(),
                RoleProviderMemberContribution {
                    implementation: None,
                    supported_platforms: Vec::new(),
                    required_resource_kinds: BTreeSet::new(),
                },
            )]),
        };
        catalog.role_providers.push(MaterializedRoleProvider {
            provider: ExactRoleProviderRef {
                role: role.clone(),
                package: PackageRef {
                    id: mount.into(),
                    version: "1.0.0".into(),
                },
                mount_id: mount.into(),
                contribution_digest: digest_payload(&contribution).unwrap(),
            },
            contribution,
            source: PluginSourceMetadata {
                source_kind,
                source_identity: mount.into(),
                source_digest: None,
            },
        });
    }
    let capabilities = vec![CapabilityCatalogItemDto {
        capability: wire_cast(&member).unwrap(),
        kind: "tool".into(),
        display_name: "Search".into(),
        description: String::new(),
        source_package: ExactCatalogRefDto {
            id: "platform".into(),
            version: "1.0.0".into(),
        },
        source_kind: "bundled".into(),
        materialization_state: CatalogMaterializationStateDto::Materialized,
        unavailable_code: None,
        supported_surfaces: BTreeSet::from(["desktop".into()]),
        required_runtime_features: BTreeSet::new(),
        required_resource_kinds: BTreeSet::new(),
        required_capabilities: Vec::new(),
        conflicting_capabilities: Vec::new(),
        action_count: 1,
        context_contributor_count: 0,
    }];
    (catalog, capabilities)
}

#[test]
fn role_projection_exposes_exact_sorted_builtin_and_user_choices_not_test_fixtures() {
    let (catalog, capabilities) = fixture();
    let roles = catalog.roles_api(&capabilities).unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(
        roles[0].capabilities,
        vec![capabilities[0].capability.clone()]
    );
    assert_eq!(
        roles[0]
            .providers
            .iter()
            .map(|item| item.selection.provider_mount_id.as_str())
            .collect::<Vec<_>>(),
        vec!["builtin", "user"]
    );
    for candidate in &roles[0].providers {
        assert_eq!(candidate.selection.role, roles[0].role);
        assert_eq!(candidate.supported_capabilities, roles[0].capabilities);
        // Public choice is accepted by the existing canonical selection wire type.
        let selected: nomifun_agent_contracts::RoleProviderSelection =
            wire_cast(&candidate.selection).unwrap();
        assert_eq!(selected.role.key, catalog.role_contracts[0].manifest.key);
        assert_eq!(
            selected.role.contract_digest,
            catalog.role_contracts[0].contract_digest
        );
    }
}

#[test]
fn role_projection_uses_exact_visible_members_and_never_substitutes_a_different_contract() {
    let (mut catalog, mut capabilities) = fixture();
    assert!(catalog.roles_api(&[]).unwrap().is_empty());
    capabilities[0].capability.version = "2.0.0".into();
    assert!(catalog.roles_api(&capabilities).unwrap().is_empty());
    capabilities[0].capability.version = "1.0.0".into();
    catalog.role_providers[0].provider.role.contract_digest = "b".repeat(64).into();
    let roles = catalog.roles_api(&capabilities).unwrap();
    assert_eq!(roles[0].providers.len(), 1);
    assert_eq!(roles[0].providers[0].selection.provider_mount_id, "builtin");
}
