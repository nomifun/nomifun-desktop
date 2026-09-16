//! New Session-view declarations are rejected without disabling an existing App.
use super::*;

#[tokio::test]
async fn new_agent_view_build_is_rejected_and_existing_surface_remains_usable() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let plugin = install_ui(&router).await;
    let id = plugin["plugin_id"].as_str().unwrap();
    let base = format!("/api/plugins/runtimes/{id}");
    let current = get(&router, &format!("{base}/workshop")).await;
    let changed = post(&router,&format!("{base}/source/edit"),json!({
        "plugin_id":id,"expected_product_revision":current["plugin"]["product_revision"],
        "project_id":current["project_id"],"expected_project_revision":current["project_revision"],
        "expected_build_generation":current["build_generation"],"expected_source_snapshot_digest":current["source_snapshot_digest"],
        "path":"nomifun.plugin.json","content":json!({"agent_view":{"name":"Retired","description":"No Session access"}}).to_string()
    })).await;
    let (status,error) = request(&router,"POST",&format!("{base}/build"),json!({
        "plugin_id":id,"expected_product_revision":changed["plugin"]["product_revision"],
        "project_id":changed["project_id"],"expected_project_revision":changed["project_revision"],
        "expected_build_generation":changed["build_generation"],"expected_source_snapshot_digest":changed["source_snapshot_digest"],
        "expected_dependency_lock_digest":changed["dependency_lock_digest"]
    })).await;
    assert!(!status.is_success(), "{error}");
    assert!(error.to_string().contains("unsupported"), "{error}");
    let after = get(&router, &format!("{base}/workshop")).await;
    assert_eq!(
        after["plugin"]["releases"]["active"],
        current["plugin"]["releases"]["active"]
    );
    assert_eq!(after["plugin"]["lifecycle"], "enabled");
    assert!(
        post(
            &router,
            &format!("{base}/surface/open"),
            json!({"plugin_id":id})
        )
        .await["surface_capability"]
            .is_string()
    );
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

#[test]
fn retired_multi_consumer_view_is_filtered_without_changing_publication_digest() {
    use nomifun_agent_contracts::*;
    use std::collections::BTreeMap;
    const CAPABILITIES: &[nomifun_agent_domain_support::CapabilitySpec] =
        &[nomifun_agent_domain_support::CapabilitySpec::tool("legacy.echo", EffectClass::Pure, &[])];
    let registration =
        nomifun_agent_domain_support::registration(nomifun_agent_domain_support::PackageSpec {
            id: "legacy.mixed",
            mount_id: "legacy-mixed",
            display_name: "Legacy mixed",
            description: "Catalog projection regression",
            supported_surfaces: &["desktop"],
            capabilities: CAPABILITIES,
        })
        .unwrap();
    let tool = registration
        .metadata
        .manifest
        .payload
        .contributions
        .capabilities[0]
        .clone();
    let mut view = tool.clone();
    view.id = "legacy.session-view".into();
    view.contribution_id = "capability:legacy.session-view".into();
    view.kind = CapabilityKind::UiContribution;
    view.supported_surfaces = capability_surface_declarations(
        ["desktop"],
        [CapabilityConsumer::Agent, CapabilityConsumer::Ui],
    );
    view.contributions = CapabilityContributions {
        ui_slot: Some(UiContributionSlot::AgentSession),
        ..Default::default()
    };
    let release = PluginReleaseRef {
        release_id: "legacy-release".into(),
        artifact_id: "legacy-artifact".into(),
        release_digest: "a".repeat(64).into(),
        manifest_digest: "b".repeat(64).into(),
    };
    let capabilities = [tool, view]
        .into_iter()
        .map(|manifest| {
            let availability = manifest
                .supported_consumers()
                .unwrap()
                .into_iter()
                .map(|consumer| (consumer, CatalogAvailability::Active))
                .collect();
            let entry =
                CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
                    provenance: CapabilityProvenance {
                        owner: CapabilityOwner::Package {
                            package: manifest.package.clone(),
                        },
                        source_kind: ContributionSourceKind::PluginProductActiveRelease,
                        source_identity: "plugin:legacy-mixed".into(),
                        mount_id: None,
                        plugin_product_id: Some("legacy-mixed".into()),
                        mcp_binding_id: None,
                        artifact_digest: Some(release.release_digest.clone()),
                    },
                    manifest: manifest.clone(),
                    release_state: CapabilityReleaseState::PublishedActive,
                    availability,
                })
                .unwrap();
            CapabilityCatalogPublication { manifest, entry }
        })
        .collect();
    let mut publication = PluginProductCapabilityCatalogPublication {
        plugin_product_id: "legacy-mixed".into(),
        active_release: release,
        active_release_epoch: 1,
        catalog_digest: "0".repeat(64).into(),
        capabilities,
    };
    publication.catalog_digest = publication.computed_catalog_digest().unwrap();
    let expected = publication.catalog_digest.clone();
    let formal_capability_entries = publication.capabilities.iter()
        .map(|item| (item.entry.capability.clone(), item.entry.clone())).collect();
    let catalog = nomifun_agent_control_plane::CatalogSnapshot {
        formal_capability_entries,
        plugin_product_publications: BTreeMap::from([("legacy-mixed".into(), publication)]),
        ..Default::default()
    };
    let projected = catalog.as_api().unwrap();
    assert_eq!(
        projected
            .capabilities
            .iter()
            .map(|item| item.capability.id.as_str())
            .collect::<Vec<_>>(),
        ["legacy.echo"]
    );
    assert_eq!(
        catalog.plugin_product_publications[&PluginProductId::from("legacy-mixed")]
            .computed_catalog_digest()
            .unwrap(),
        expected
    );
}
