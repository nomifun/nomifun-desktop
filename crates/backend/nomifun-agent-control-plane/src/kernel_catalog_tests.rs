#[cfg(test)]
mod catalog_materialization_tests {
    use super::super::*;
    use crate::SharedPluginProductCatalogPublications;
    use nomifun_agent_contracts::{
        ArtifactId, CapabilityCatalogMaterialization, CapabilityCatalogMaterializer,
        CapabilityCatalogPublication, CapabilityContributions, CapabilityManifest, CapabilityOwner,
        CapabilityProvenance, CapabilityRef, ContributionId, ContributionSourceKind,
        LocalizedMetadata, McpBindingId, McpServerId, McpToolCapabilityMapping, McpToolKey,
        PluginProductCapabilityCatalogPublication, PluginProductCapabilityCatalogSink, PluginProductId,
        PluginReleaseId, PluginReleaseRef, PackageId, PackageRef, PlatformConstraint,
        PluginSourceKind, PluginSourceMetadata, StableSourceIdentity,
        capability_surface_declarations, digest_bytes,
    };
    use nomifun_agent_contracts::{
        CapabilityKind, ContributionLock, DigestHex, PluginMountId, StrictJsonValue, VersionString,
        digest_payload,
    };
    use nomifun_agent_kernel::MaterializationPolicy;
    use nomifun_agent_kernel::{
        InMemoryPluginStatePersistence, MaterializedCapability, MaterializedMcpTool,
    };
    use serde_json::json;

    #[test]
    fn managed_mcp_projection_preserves_exact_owner_mount_and_artifact() {
        let package = PackageRef {
            id: PackageId::from("managed.catalog"),
            version: VersionString::from("1.0.0"),
        };
        let mount_id = PluginMountId::from("managed-catalog-mount");
        let artifact_digest = DigestHex::from("a".repeat(64));
        let capability_ref = CapabilityRef {
            id: CapabilityId::from("managed.catalog.run"),
            version: VersionString::from("1.0.0"),
        };
        let manifest = CapabilityManifest {
            id: capability_ref.id.clone(),
            contribution_id: ContributionId::from("capability:managed.catalog.run"),
            version: capability_ref.version.clone(),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Managed Catalog".to_owned(),
                description: "Managed MCP Catalog fixture".to_owned(),
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
        let contract_digest = digest_payload(&manifest).unwrap();
        let binding_id = McpBindingId::from("managed.catalog.server:run");
        let server_id = McpServerId::from("managed.catalog.server");
        let tool_key = McpToolKey::from("run");
        let contribution_lock = ContributionLock {
            source_kind: nomifun_agent_contracts::ContributionSourceKind::McpBinding,
            source_identity: StableSourceIdentity::from("mcp:managed.catalog.server"),
            mount_id: Some(mount_id.clone()),
            plugin_product_id: None,
            mcp_binding_id: Some(binding_id.clone()),
            contribution_id: manifest.contribution_id.clone(),
            contract_digest: contract_digest.clone(),
        };
        let mut registry = nomifun_agent_kernel::MaterializedRegistry::empty();
        registry.capabilities.insert(
            capability_ref.id.clone(),
            MaterializedCapability {
                manifest,
                schema_digest: contract_digest.clone(),
                contribution_id: contribution_lock.contribution_id.clone(),
                contribution_lock: contribution_lock.clone(),
                target_artifact_digest: artifact_digest.clone(),
                mount_id: mount_id.clone(),
                source: PluginSourceMetadata {
                    source_kind: PluginSourceKind::ManagedLocal,
                    source_identity: mount_id.as_ref().to_owned(),
                    source_digest: Some(artifact_digest.clone()),
                },
            },
        );
        let mapping = McpToolCapabilityMapping {
            package: package.clone(),
            server_id: server_id.clone(),
            canonical_tool_key: tool_key.clone(),
            schema_digest: DigestHex::from("b".repeat(64)),
            capability: capability_ref.clone(),
            materialization_version: VersionString::from("1.0.0"),
        };
        registry.mcp_by_capability.insert(
            capability_ref.id.clone(),
            (server_id.clone(), tool_key.clone()),
        );
        registry.mcp_tools.insert(
            (server_id, tool_key),
            MaterializedMcpTool {
                mapping,
                binding_id: binding_id.clone(),
                contribution_lock: contribution_lock.clone(),
                target_artifact_digest: artifact_digest.clone(),
                mount_id: mount_id.clone(),
                source: PluginSourceMetadata {
                    source_kind: PluginSourceKind::ManagedLocal,
                    source_identity: mount_id.as_ref().to_owned(),
                    source_digest: Some(artifact_digest.clone()),
                },
            },
        );
        let snapshot = materialize_catalog_snapshot(&registry, &BTreeMap::new()).unwrap();
        let entry = snapshot
            .formal_capability_entries
            .get(&capability_ref)
            .unwrap();
        assert_eq!(entry.provenance.owner, CapabilityOwner::Package { package });
        assert_eq!(entry.provenance.mount_id.as_ref(), Some(&mount_id));
        assert_eq!(entry.provenance.mcp_binding_id.as_ref(), Some(&binding_id));
        assert_eq!(
            entry.provenance.artifact_digest.as_ref(),
            Some(&artifact_digest)
        );
        assert_eq!(entry.contract_digest, contract_digest);
        assert_eq!(
            entry
                .operation_lock(CapabilityConsumer::Agent)
                .unwrap()
                .contribution,
            contribution_lock
        );

        let mut missing_mount = snapshot;
        missing_mount.mcp_tools[0].contribution_lock.mount_id = None;
        assert!(missing_mount.validate().is_err());
    }

    #[test]
    fn plugin_product_publication_enters_the_same_shared_catalog_provider() {
        let package = PackageRef {
            id: PackageId::from("plugin-product.catalog"),
            version: VersionString::from("1.0.0"),
        };
        let plugin_product_id = PluginProductId::from("plugin-product-catalog");
        let artifact_digest = digest_bytes(b"plugin-product-artifact");
        let manifest = CapabilityManifest {
            id: CapabilityId::from("plugin-product.catalog.search"),
            contribution_id: ContributionId::from("capability:plugin-product.catalog.search"),
            version: VersionString::from("1.0.0"),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Plugin Product Search".to_owned(),
                description: "Search from an Active Plugin Product Release".to_owned(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions::default(),
        };
        let active_release = PluginReleaseRef {
            release_id: PluginReleaseId::from("release-plugin-product-search"),
            artifact_id: ArtifactId::from("artifact-plugin-product-search"),
            release_digest: artifact_digest.clone(),
            manifest_digest: digest_bytes(b"plugin-product-manifest"),
        };
        let entry = CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
            manifest: manifest.clone(),
            provenance: CapabilityProvenance {
                owner: CapabilityOwner::Package {
                    package: package.clone(),
                },
                source_kind: ContributionSourceKind::PluginProductActiveRelease,
                source_identity: StableSourceIdentity::from("plugin-product:plugin-product-catalog"),
                mount_id: None,
                plugin_product_id: Some(plugin_product_id.clone()),
                mcp_binding_id: None,
                artifact_digest: Some(artifact_digest),
            },
            release_state: CapabilityReleaseState::PublishedActive,
            availability: BTreeMap::from([
                (CapabilityConsumer::Agent, CatalogAvailability::Active),
                (CapabilityConsumer::Gateway, CatalogAvailability::Active),
            ]),
        })
        .unwrap();
        let mut publication = PluginProductCapabilityCatalogPublication {
            plugin_product_id: plugin_product_id.clone(),
            active_release,
            active_release_epoch: 1,
            catalog_digest: digest_bytes(b"uncomputed"),
            capabilities: vec![CapabilityCatalogPublication { manifest, entry }],
        };
        publication.catalog_digest = publication.computed_catalog_digest().unwrap();
        publication.validate().unwrap();
        let mut tampered = publication.clone();
        tampered.catalog_digest = digest_bytes(b"tampered-publication");
        assert!(tampered.validate().is_err());

        let store = Arc::new(SharedPluginProductCatalogPublications::new());
        store
            .replace_plugin_product_publication(
                nomifun_agent_contracts::PluginProductCapabilityCatalogPublicationUpdate {
                    owner_user_id: "00000000-0000-7000-8000-000000000001".into(),
                    plugin_product_id: plugin_product_id.clone(),
                    product_revision: 1,
                    pointer_revision: 1,
                    active_release_epoch: 1,
                    publication: Some(publication.clone()),
                },
            )
            .unwrap();
        let kernel = Arc::new(
            KernelRegistry::new(
                MaterializationPolicy::stable("1.0.0"),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        let provider =
            KernelCatalogProvider::new(kernel).with_plugin_product_publication_source(store.clone());
        let snapshot = provider.snapshot().unwrap();
        let reference = CapabilityRef {
            id: CapabilityId::from("plugin-product.catalog.search"),
            version: VersionString::from("1.0.0"),
        };
        assert_eq!(
            snapshot
                .capability_catalog_entry(&reference)
                .unwrap()
                .unwrap()
                .provenance
                .source_kind,
            ContributionSourceKind::PluginProductActiveRelease
        );
        assert_eq!(
            snapshot.find_capability(&reference).unwrap().package,
            package
        );
        assert!(snapshot.as_api().unwrap().capabilities.iter().any(|item| {
            item.capability.id == "plugin-product.catalog.search"
                && item.source_kind == "plugin_product_active_release"
        }));

        store
            .replace_plugin_product_publication(
                nomifun_agent_contracts::PluginProductCapabilityCatalogPublicationUpdate {
                    owner_user_id: "00000000-0000-7000-8000-000000000001".into(),
                    plugin_product_id: plugin_product_id.clone(),
                    product_revision: 2,
                    pointer_revision: 2,
                    active_release_epoch: 1,
                    publication: None,
                },
            )
            .unwrap();
        store
            .replace_plugin_product_publication(
                nomifun_agent_contracts::PluginProductCapabilityCatalogPublicationUpdate {
                    owner_user_id: "00000000-0000-7000-8000-000000000001".into(),
                    plugin_product_id: plugin_product_id.clone(),
                    product_revision: 1,
                    pointer_revision: 1,
                    active_release_epoch: 1,
                    publication: Some(publication),
                },
            )
            .unwrap();
        assert!(
            provider
                .snapshot()
                .unwrap()
                .capability_catalog_entry(&reference)
                .unwrap()
                .is_none()
        );
    }
}
