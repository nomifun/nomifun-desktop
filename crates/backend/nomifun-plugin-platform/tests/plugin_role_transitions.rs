use std::sync::Arc;

use nomifun_api_types::*;
use nomifun_db::{SqlitePluginRuntimeRepository, init_database_memory, installation_owner_id};
use nomifun_plugin_platform::runtime::PluginRuntimeApplicationService;

async fn replace(
    app: &PluginRuntimeApplicationService, owner: &str, current: &PluginRuntimeWorkshopDto,
    path: &str, content: &str,
) -> PluginRuntimeWorkshopDto {
    app.replace_source_file(owner, ReplacePluginRuntimeSourceFileRequest {
        plugin_id: current.plugin.plugin_id.clone(),
        expected_product_revision: current.plugin.product_revision,
        project_id: current.project_id.clone(),
        expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        expected_source_snapshot_digest: current.source_snapshot_digest.clone().unwrap(),
        path: path.into(), content: content.into(),
    }).await.unwrap()
}

async fn build(app: &PluginRuntimeApplicationService, owner: &str, current: &PluginRuntimeWorkshopDto) -> PluginRuntimeWorkshopDto {
    app.build(owner, BuildPluginRuntimeRequest {
        plugin_id: current.plugin.plugin_id.clone(),
        expected_product_revision: current.plugin.product_revision,
        project_id: current.project_id.clone(),
        expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        expected_source_snapshot_digest: current.source_snapshot_digest.clone().unwrap(),
        expected_dependency_lock_digest: current.dependency_lock_digest.clone().unwrap(),
        service_lifecycle: None,
    }).await.unwrap()
}

fn publish_request(current: &PluginRuntimeWorkshopDto, acknowledge: bool) -> PublishPluginRuntimeRequest {
    let ready = current.ready.as_ref().unwrap();
    PublishPluginRuntimeRequest {
        plugin_id: current.plugin.plugin_id.clone(),
        expected_product_revision: current.plugin.product_revision,
        expected_pointer_revision: current.plugin.releases.pointer_revision,
        expected_active_release_epoch: current.plugin.releases.active_release_epoch,
        ready_release_id: ready.release.release_id.clone(),
        expected_ready_release_digest: ready.release.release_digest.clone(),
        expected_active_release_digest: current.plugin.releases.active.as_ref().map(|r| r.release_digest.clone()),
        expected_service_test_receipt_id: None,
        acknowledge_test_warning: acknowledge,
    }
}

#[tokio::test]
async fn one_product_adds_and_removes_service_roles_with_release_scoped_publish_guards() {
    let db = init_database_memory().await.unwrap();
    let owner = installation_owner_id(db.pool()).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let app = PluginRuntimeApplicationService::new_with_root(
        Arc::new(SqlitePluginRuntimeRepository::new(db.pool().clone())), root.path(),
    ).unwrap();
    let mut current = app.create(&owner, CreatePluginRuntimeProjectRequest {
        expected_library_revision: 0, display_name: "Composable product".into(),
        description: None, service_source: None,
    }).await.unwrap();
    let identity = current.plugin.plugin_id.clone();
    current = build(&app, &owner, &current).await;
    assert!(current.ready.as_ref().unwrap().service.is_none());
    current = app.publish(&owner, publish_request(&current, false)).await.unwrap();

    current = replace(&app, &owner, &current, "service/main.mjs",
        "export async function start() { return { async invoke({payload}) { return payload; }, async dispose() {} }; }").await;
    current = build(&app, &owner, &current).await;
    assert!(current.ready.as_ref().unwrap().service.is_some());
    assert!(!current.ready.as_ref().unwrap().can_auto_publish);
    assert!(app.publish(&owner, publish_request(&current, false)).await.is_err());
    current = app.publish(&owner, publish_request(&current, true)).await.unwrap();
    assert!(current.active_service.is_some());
    assert!(app.set_publish_mode(&owner, SetPluginRuntimePublishModeRequest {
        plugin_id: identity.clone(), expected_product_revision: current.plugin.product_revision,
        expected_pointer_revision: current.plugin.releases.pointer_revision,
        mode: PluginRuntimePublishModeDto::AutoUiOnly,
    }).await.is_err());

    current = replace(&app, &owner, &current, "service/main.mjs", "").await;
    current = build(&app, &owner, &current).await;
    assert!(current.ready.as_ref().unwrap().service.is_none());
    current = app.publish(&owner, publish_request(&current, false)).await.unwrap();
    assert!(current.active_service.is_none());
    assert_eq!(current.plugin.plugin_id, identity);
    assert_eq!(current.plugin.kind, PluginRuntimeKindDto::Plugin);

    // A product backup may retain releases with different roles. The Active UI
    // must not cause the Previous Service release to be rejected on import.
    let backup_path = root.path().join("mixed-role-backup");
    app.export_backup(&owner, ExportPluginRuntimeBackupRequest {
        plugin_id: identity.clone(),
        expected_product_revision: current.plugin.product_revision,
        expected_lifecycle: current.plugin.lifecycle,
        expected_pointer_revision: current.plugin.releases.pointer_revision,
        expected_config_revision: current.config.config_revision,
        expected_credential_bindings_revision: current.credential_bindings_revision,
        destination_path: backup_path.display().to_string(),
    }).await.unwrap();
    let metadata: nomifun_agent_contracts::PluginProductBackupMetadataV1 =
        serde_json::from_slice(&std::fs::read(backup_path.join("metadata.json")).unwrap()).unwrap();
    let imported = app.import_backup(&owner, ImportPluginRuntimeBackupRequest {
        expected_library_revision: app.library(&owner).await.unwrap().library_revision,
        source_path: backup_path.display().to_string(),
        expected_backup_metadata_digest: metadata.metadata_digest().unwrap().as_ref().to_owned(),
        display_name: "Mixed role copy".into(),
    }).await.unwrap();
    assert_ne!(imported.plugin.plugin_id, identity);
    assert!(imported.active_service.is_none());
    assert!(imported.plugin.releases.previous.is_some());

    let active = current.plugin.releases.active.as_ref().unwrap();
    let previous = current.plugin.releases.previous.as_ref().unwrap();
    current = app.rollback(&owner, RollbackPluginRuntimeRequest {
        plugin_id: identity.clone(), expected_product_revision: current.plugin.product_revision,
        expected_pointer_revision: current.plugin.releases.pointer_revision,
        expected_active_release_epoch: current.plugin.releases.active_release_epoch,
        expected_current_release_digest: active.release_digest.clone(),
        previous_release_id: previous.release_id.clone(),
        expected_previous_release_digest: previous.release_digest.clone(),
    }).await.unwrap();
    assert_eq!(current.plugin.plugin_id, identity);
    assert!(current.active_service.is_some());
}
