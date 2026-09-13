use std::sync::Arc;

use nomifun_agent_contracts::MiniAppShareBundleV1;
use nomifun_api_types::{
    BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest, ImportPluginRuntimeArtifactRequest,
    ImportPluginRuntimeShareRequest, PluginRuntimeKindDto, PluginRuntimeProjectSourceStateDto,
    PluginRuntimeShareContentDto, SharePluginRuntimeRequest,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationService;

#[tokio::test]
async fn share_and_prebuilt_import_create_distinct_disabled_ready_products() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> =
        Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone()));
    let root = tempfile::tempdir().unwrap();
    let application =
        PluginRuntimeM1ApplicationService::new_with_root(repository, root.path()).unwrap();
    let created = application
        .create(
            &owner,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: "Share source".into(),
                description: None,
                kind: PluginRuntimeKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = application
        .build(
            &owner,
            BuildPluginRuntimeRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: created.miniapp.product_revision,
                project_id: created.project_id.clone(),
                expected_project_revision: created.project_revision,
                expected_build_generation: created.build_generation,
                expected_source_snapshot_digest: created.source_snapshot_digest.unwrap(),
                expected_dependency_lock_digest: created.dependency_lock_digest.unwrap(),
                service_lifecycle: None,
            },
        )
        .await
        .unwrap();
    let ready = built.ready.as_ref().unwrap();
    let destination = root.path().join("shared-miniapp");
    let exported = application
        .export_share(
            &owner,
            SharePluginRuntimeRequest {
                miniapp_id: built.miniapp.miniapp_id.clone(),
                expected_product_revision: built.miniapp.product_revision,
                expected_pointer_revision: built.miniapp.releases.pointer_revision,
                content: PluginRuntimeShareContentDto::ReadyRelease,
                release_id: ready.release.release_id.clone(),
                expected_release_digest: ready.release.release_digest.clone(),
                destination_path: destination.display().to_string(),
                include_source: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(exported.state, nomifun_api_types::DurableOperationStateDto::Succeeded);
    let bundle: MiniAppShareBundleV1 =
        serde_json::from_slice(&std::fs::read(destination.join("bundle.json")).unwrap()).unwrap();

    let imported = application
        .import_share(
            &owner,
            ImportPluginRuntimeShareRequest {
                expected_library_revision: application.library(&owner).await.unwrap().library_revision,
                source_path: destination.display().to_string(),
                expected_bundle_digest: bundle.bundle_digest.as_ref().to_owned(),
                expected_release_digest: bundle.release.artifact_digest.as_ref().to_owned(),
                display_name: "Imported source".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(imported.miniapp.miniapp_id, built.miniapp.miniapp_id);
    assert_eq!(imported.miniapp.lifecycle, nomifun_api_types::PluginRuntimeLifecycleDto::Disabled);
    assert_eq!(imported.source_state, PluginRuntimeProjectSourceStateDto::Editable);
    assert!(imported.ready.is_some());
    assert!(imported.miniapp.releases.active.is_none());

    let prebuilt = application
        .import_prebuilt(
            &owner,
            ImportPluginRuntimeArtifactRequest {
                expected_library_revision: application.library(&owner).await.unwrap().library_revision,
                source_path: destination.join("release").display().to_string(),
                expected_artifact_digest: bundle.release.artifact_digest.as_ref().to_owned(),
                display_name: "Imported runtime".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(prebuilt.miniapp.miniapp_id, imported.miniapp.miniapp_id);
    assert_eq!(prebuilt.source_state, PluginRuntimeProjectSourceStateDto::RuntimeOnly);
    assert!(prebuilt.ready.is_some());
    assert!(prebuilt.miniapp.releases.active.is_none());
}
