//! Historical mixed releases remain usable as ordinary Apps and continuous Services.
use super::*;

#[tokio::test]
async fn historical_mixed_view_keeps_surface_continuous_service_and_catalog_digest() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IPluginRuntimeRepository> =
        Arc::new(SqlitePluginRuntimeRepository::new(database.pool().clone()));
    let root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeApplicationService::new_with_root(repository.clone(), root.path()).unwrap(),
    );
    let runtime = Arc::new(TestRuntime::new());
    application.install_service_runtime(runtime.clone()).await;

    // Build a real immutable Service Release with one Agent-callable
    // contribution using the same PluginReleaseV1 builder as M1 builds.
    let original = callable_service_artifact();
    let mut payload = original.manifest.payload.clone();
    payload.service.as_mut().unwrap().lifecycle = PluginServiceLifecycle::Continuous;
    let mut legacy_view = payload.contributions.capabilities[0].clone();
    legacy_view.id = "plugin.callable.legacy-view".into();
    legacy_view.contribution_id = "capability:plugin.callable.legacy-view".into();
    legacy_view.kind = CapabilityKind::UiContribution;
    legacy_view.supported_surfaces =
        capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui]);
    legacy_view.contributions = CapabilityContributions {
        ui_slot: Some(nomifun_agent_contracts::UiContributionSlot::AgentSession),
        ..Default::default()
    };
    payload.contributions.capabilities.push(legacy_view);
    let artifact = nomifun_agent_contracts::PluginReleaseArtifactV1::new(
        original.artifact_id,
        payload,
        original.files,
    )
    .unwrap();
    assert!(artifact.manifest.payload.service.is_some());
    assert_eq!(
        artifact.manifest.payload.contributions.capabilities.len(),
        2
    );
    let contributions = artifact.manifest.payload.contributions.clone();
    let fixture_release_store =
        PluginRuntimeReleaseStore::new(root.path().join("fixture-release-store")).unwrap();
    let stored = fixture_release_store
        .publish(PluginRuntimeReleasePublishRequest::service(
            PluginRuntimeSourceScope::new("fixture-owner", "fixture-plugin", "fixture-project")
                .unwrap(),
            digest("fixture-source"),
            artifact.manifest.payload.dependency_lock_digest.clone(),
            1,
            artifact.clone(),
            release_files(&artifact),
        ))
        .unwrap()
        .stored;

    // The application import path gives the prebuilt artifact a durable
    // Ready Release; the rest of the test exercises the actual Product
    // Publish -> Enable -> Start transitions.
    let share_root = root.path().join("callable-share");
    let bundle = PluginRuntimeShareBundleFilesystem::default()
        .export(
            PluginRuntimeShareBundleExport {
                bundle_id: PluginShareBundleId::from("callable-service-bundle"),
                source_plugin_product_id: Some(PluginProductId::from("fixture-plugin")),
                release: &stored,
                source: None,
                test_provenance: None,
            },
            &share_root,
        )
        .unwrap();
    let prebuilt_root = root.path().join("callable-prebuilt");
    copy_tree(&share_root.join("release"), &prebuilt_root);

    let imported = application
        .import_prebuilt(
            &owner,
            ImportPluginRuntimeArtifactRequest {
                expected_library_revision: 0,
                source_path: prebuilt_root.display().to_string(),
                expected_artifact_digest: bundle.release.artifact_digest.as_ref().to_owned(),
                display_name: "Callable Service".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(imported.plugin.kind, PluginRuntimeKindDto::Plugin);
    let ready = imported.ready.as_ref().expect("prebuilt Service is Ready");

    let denied = application
        .publish(
            &owner,
            PublishPluginRuntimeRequest {
                plugin_id: imported.plugin.plugin_id.clone(),
                expected_product_revision: imported.plugin.product_revision,
                expected_pointer_revision: imported.plugin.releases.pointer_revision,
                expected_active_release_epoch: imported.plugin.releases.active_release_epoch,
                ready_release_id: ready.release.release_id.clone(),
                expected_ready_release_digest: ready.release.release_digest.clone(),
                expected_active_release_digest: None,
                expected_service_test_receipt_id: None,
                acknowledge_test_warning: true,
            },
        )
        .await
        .unwrap_err();
    assert!(
        denied
            .to_string()
            .contains("Agent Session views are unsupported")
    );
    // Seed a pre-retirement Active pointer through the repository only in this
    // fixture. Production publication above must continue rejecting the declaration.
    let release = contract_release_ref(&ready.release);
    let catalog = nomifun_plugin_platform::runtime::plugin_catalog_digest(
        &imported.plugin.plugin_id,
        &release,
        &contributions,
    )
    .unwrap();
    repository
        .publish_ready_cas(&nomifun_db::PublishPluginRuntimeReadyParams {
            owner_user_id: owner.clone(),
            plugin_product_id: imported.plugin.plugin_id.clone(),
            expected_product_revision: imported.plugin.product_revision as i64,
            expected_pointer_revision: imported.plugin.releases.pointer_revision as i64,
            expected_active_release_epoch: imported.plugin.releases.active_release_epoch as i64,
            expected_ready_release_id: ready.release.release_id.clone(),
            expected_ready_release_digest: ready.release.release_digest.clone(),
            expected_active_release_digest: None,
            target_catalog_digest: catalog.as_ref().to_owned(),
            auto_publish_guard: None,
            updated_at: nomifun_common::now_ms(),
        })
        .await
        .unwrap();
    let active = application
        .workshop(&owner, &imported.plugin.plugin_id)
        .await
        .unwrap();
    let enabled = application
        .set_enabled(
            &owner,
            SetPluginRuntimeEnabledRequest {
                plugin_id: active.plugin.plugin_id.clone(),
                expected_product_revision: active.plugin.product_revision,
                expected_pointer_revision: active.plugin.releases.pointer_revision,
                expected_active_release_digest: Some(release.release_digest.as_ref().to_owned()),
                enabled: true,
            },
        )
        .await
        .unwrap();
    // Continuous startup belongs to bind_active/ensure_started, not the
    // explicit user-start wrapper tracked by TestRuntime.started.
    assert!(matches!(runtime.host.state(&PluginProductId::from(enabled.plugin.plugin_id.clone())).await,
        Some(nomifun_plugin_platform::runtime::PluginRuntimeServiceHostState::Running { .. })),
        "continuous Service must still be running after enable");
    let surface = application
        .open_surface(&owner, &enabled.plugin.plugin_id)
        .await
        .unwrap();
    let request = agent_invocation(
        &owner,
        &enabled.plugin.plugin_id,
        enabled.plugin.releases.active.as_ref().unwrap(),
        enabled.plugin.releases.active_release_epoch,
        catalog.clone(),
        [ActionId::from(CALLABLE_ACTION_ID)].into(),
        "legacy-mixed-tool",
    );
    assert_eq!(
        application
            .invoke_agent_capability(request)
            .await
            .unwrap()
            .0["payload"],
        json!({"value":7})
    );
    let stored = repository
        .get(&owner, &enabled.plugin.plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.product.materialized_catalog_digest, catalog.as_ref());
    assert!(
        application
            .open_surface_with_agent_session(
                &owner,
                &enabled.plugin.plugin_id,
                Some(("old-session", release.release_digest.as_ref()))
            )
            .await
            .is_err()
    );
    assert!(application.surface_bridge_request(&owner,&enabled.plugin.plugin_id,&surface.surface_capability,
        surface.active_release_epoch,&surface.expected_release_digest,
        serde_json::from_value(json!({"call_id":"blocked-session","target":{"target":"agent_session","request":{"operation":"observe","after_seq":0,"limit":10}}})).unwrap()
    ).await.is_err());
    application.shutdown_service_runtime(&owner).await.unwrap();
}
