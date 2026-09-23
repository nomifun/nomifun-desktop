#[cfg(test)]
mod catalog_materialization_tests {
    use super::super::*;
    use nomifun_agent_contracts::{
        CapabilityContributions, CapabilityManifest, CapabilityOwner, CapabilityPublicationState,
        CapabilityRef, ContributionId,
        LocalizedMetadata, McpBindingId, McpServerId, McpToolCapabilityMapping, McpToolKey,
        PackageId, PackageRef,
        PluginSourceKind, PluginSourceMetadata, StableSourceIdentity,
        capability_surface_declarations,
    };
    use nomifun_agent_contracts::{
        CapabilityKind, ContributionLock, DigestHex, AgentModuleId, StrictJsonValue, VersionString,
        digest_payload,
    };
    use nomifun_agent_kernel::{MaterializedCapability, MaterializedMcpTool};
    use serde_json::json;

    #[test]
    fn managed_mcp_projection_preserves_exact_owner_mount_and_artifact() {
        let package = PackageRef {
            id: PackageId::from("managed.catalog"),
            version: VersionString::from("1.0.0"),
        };
        let mount_id = AgentModuleId::from("managed-catalog-mount");
        let artifact_digest = DigestHex::from("a".repeat(64));
        let capability_ref = CapabilityRef {
            id: CapabilityId::from("managed.catalog.run"),
        };
        let manifest = CapabilityManifest {
            id: capability_ref.id.clone(),
            contribution_id: ContributionId::from("capability:managed.catalog.run"),
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
            entry.publication_state,
            CapabilityPublicationState::Active
        );
        assert_eq!(
            entry
                .operation_lock(CapabilityConsumer::Agent)
                .unwrap()
                .contribution,
            contribution_lock
        );

        registry
            .capabilities
            .get_mut(&capability_ref.id)
            .unwrap()
            .source
            .source_kind = PluginSourceKind::TestFixture;
        assert!(
            materialize_capability_catalog_entries(&registry, &BTreeMap::new())
                .unwrap()
                .is_empty(),
            "test-only capabilities must not enter the formal catalog"
        );

        let mut missing_mount = snapshot;
        missing_mount.mcp_tools[0].contribution_lock.mount_id = None;
        assert!(missing_mount.validate().is_err());
    }


}
