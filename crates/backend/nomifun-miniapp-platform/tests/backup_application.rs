use std::sync::Arc;

use nomifun_api_types::{
    BuildMiniAppRequest, CreateMiniAppProjectRequest, ExportMiniAppBackupRequest,
    ImportMiniAppBackupRequest, MiniAppKindDto, MiniAppLifecycleDto,
};
use nomifun_agent_contracts::{
    MiniAppReleaseArtifactV1, MiniAppReleaseRef, PackageContributions, digest_payload,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_miniapp_platform::MiniAppM1ApplicationService;
use serde::Serialize;

#[tokio::test]
async fn whole_app_backup_roundtrips_disabled_ui_only_product_as_new_identity() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> =
        Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone()));
    let root = tempfile::tempdir().unwrap();
    let application =
        MiniAppM1ApplicationService::new_with_root(repository.clone(), root.path()).unwrap();

    let created = application
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Backup source".into(),
                description: Some("Backup test".into()),
                kind: MiniAppKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = application
        .build(
            &owner,
            BuildMiniAppRequest {
                miniapp_id: built_id(&created),
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

    let source_id = built.miniapp.miniapp_id.clone();
    let source_snapshot = repository
        .get(&owner, source_id.as_ref())
        .await
        .unwrap()
        .unwrap();
    repository
        .put_kv_cas(
            &owner,
            source_id.as_ref(),
            "surface",
            "theme",
            &serde_json::json!({"density": "compact"}),
            None,
            source_snapshot.product.updated_at + 1,
        )
        .await
        .unwrap();
    let disabled = application
        .workshop(&owner, source_id.as_ref())
        .await
        .unwrap();
    let destination = root.path().join("whole-app-backup");
    let exported = application
        .export_backup(
            &owner,
            ExportMiniAppBackupRequest {
                miniapp_id: source_id.clone(),
                expected_product_revision: disabled.miniapp.product_revision,
                expected_lifecycle: MiniAppLifecycleDto::Disabled,
                expected_pointer_revision: disabled.miniapp.releases.pointer_revision,
                expected_config_revision: disabled.config.config_revision,
                expected_credential_bindings_revision: disabled
                    .credential_bindings_revision,
                destination_path: destination.display().to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(exported.state, nomifun_api_types::DurableOperationStateDto::Succeeded);
    assert!(destination.join("metadata.json").is_file());
    assert!(destination.join("releases").is_dir());
    assert!(destination.join("storage/kv.json").is_file());
    assert!(!destination.join("credential.json").exists());

    let imported = application
        .import_backup(
            &owner,
            ImportMiniAppBackupRequest {
                expected_library_revision: application.library(&owner).await.unwrap().library_revision,
                source_path: destination.display().to_string(),
                expected_backup_metadata_digest: read_metadata_digest(&destination),
                display_name: "Backup copy".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(imported.miniapp.miniapp_id, source_id);
    assert_eq!(imported.miniapp.lifecycle, MiniAppLifecycleDto::Disabled);
    assert_eq!(
        imported.miniapp.releases.ready.as_ref().map(|release| {
            (
                release.artifact_id.clone(),
                release.release_digest.clone(),
                release.manifest_digest.clone(),
            )
        }),
        disabled.miniapp.releases.ready.as_ref().map(|release| {
            (
                release.artifact_id.clone(),
                release.release_digest.clone(),
                release.manifest_digest.clone(),
            )
        })
    );
    assert!(imported.credential_slots.is_empty());

    let imported_row = repository
        .get(&owner, imported.miniapp.miniapp_id.as_ref())
        .await
        .unwrap()
        .unwrap();
    let kv = repository
        .get_kv(
            &owner,
            imported.miniapp.miniapp_id.as_ref(),
            "surface",
            "theme",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kv.value_json, r#"{"density":"compact"}"#);
    assert!(imported_row.credential_bindings.is_empty());
}

#[tokio::test]
async fn whole_app_backup_import_recomputes_catalog_digest_for_new_identity() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> =
        Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone()));
    let root = tempfile::tempdir().unwrap();
    let application =
        MiniAppM1ApplicationService::new_with_root(repository.clone(), root.path()).unwrap();

    let created = application
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Catalog source".into(),
                description: None,
                kind: MiniAppKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = application
        .build(
            &owner,
            BuildMiniAppRequest {
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
    let published = application
        .publish(
            &owner,
            nomifun_api_types::PublishMiniAppRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: built.miniapp.product_revision,
                expected_pointer_revision: built.miniapp.releases.pointer_revision,
                expected_active_release_epoch: built.miniapp.releases.active_release_epoch,
                ready_release_id: ready.release.release_id.clone(),
                expected_ready_release_digest: ready.release.release_digest.clone(),
                expected_active_release_digest: None,
                expected_service_test_receipt_id: None,
                acknowledge_test_warning: false,
            },
        )
        .await
        .unwrap();
    let destination = root.path().join("catalog-backup");
    application
        .export_backup(
            &owner,
            ExportMiniAppBackupRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: published.miniapp.product_revision,
                expected_lifecycle: MiniAppLifecycleDto::Disabled,
                expected_pointer_revision: published.miniapp.releases.pointer_revision,
                expected_config_revision: published.config.config_revision,
                expected_credential_bindings_revision: published
                    .credential_bindings_revision,
                destination_path: destination.display().to_string(),
            },
        )
        .await
        .unwrap();
    let imported = application
        .import_backup(
            &owner,
            ImportMiniAppBackupRequest {
                expected_library_revision: application.library(&owner).await.unwrap().library_revision,
                source_path: destination.display().to_string(),
                expected_backup_metadata_digest: read_metadata_digest(&destination),
                display_name: "Catalog copy".into(),
            },
        )
        .await
        .unwrap();
    let active = imported.miniapp.releases.active.as_ref().unwrap();
    let artifact: MiniAppReleaseArtifactV1 =
        serde_json::from_slice(&std::fs::read(destination.join("releases/active/artifact.json")).unwrap())
            .unwrap();
    let active_ref = MiniAppReleaseRef {
        release_id: active.release_id.clone().into(),
        artifact_id: active.artifact_id.clone().into(),
        release_digest: active.release_digest.clone().into(),
        manifest_digest: active.manifest_digest.clone().into(),
    };
    let expected = digest_payload(&CatalogDigestInput {
        miniapp_id: &imported.miniapp.miniapp_id,
        active_release: &active_ref,
        contributions: &artifact.manifest.payload.contributions,
    })
    .unwrap()
    .as_ref()
    .to_owned();
    let imported_row = repository
        .get(&owner, &imported.miniapp.miniapp_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(imported_row.product.materialized_catalog_digest, expected);
}

#[derive(Serialize)]
struct CatalogDigestInput<'a> {
    miniapp_id: &'a str,
    active_release: &'a MiniAppReleaseRef,
    contributions: &'a PackageContributions,
}

fn built_id(workshop: &nomifun_api_types::MiniAppWorkshopDto) -> String {
    workshop.miniapp.miniapp_id.clone()
}

fn read_metadata_digest(root: &std::path::Path) -> String {
    let bytes = std::fs::read(root.join("metadata.json")).unwrap();
    let metadata: nomifun_agent_contracts::MiniAppWholeAppBackupMetadataV1 =
        serde_json::from_slice(&bytes).unwrap();
    metadata.metadata_digest().unwrap().as_ref().to_owned()
}
