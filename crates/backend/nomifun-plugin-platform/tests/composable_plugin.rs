use std::sync::Arc;
use nomifun_api_types::{BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest, PluginRuntimeKindDto, ReplacePluginRuntimeSourceFileRequest};
use nomifun_db::{init_database_memory, installation_owner_id, SqliteMiniAppM1Repository};
use nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationService;

#[tokio::test]
async fn source_build_preserves_headless_actions_and_rejects_foreign_owners() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let application = PluginRuntimeM1ApplicationService::new_with_root(
        Arc::new(SqliteMiniAppM1Repository::new(database.pool().clone())), root.path(),
    ).unwrap();
    let mut current = application.create(&owner, CreatePluginRuntimeProjectRequest {
        expected_library_revision: 0, display_name: "Background helper".into(), description: None,
        kind: PluginRuntimeKindDto::Service,
    }).await.unwrap();
    let manifest = serde_json::json!({"actions":[{
        "id":"echo", "name":"Echo", "description":"Return the supplied value", "effect":"pure",
        "input_schema":{"type":"object"}, "output_schema":{"type":"object"}
    }]});
    for (path, content) in [("ui/index.html", String::new()), ("nomifun.plugin.json", manifest.to_string())] {
        let request = ReplacePluginRuntimeSourceFileRequest {
            miniapp_id: current.miniapp.miniapp_id.clone(),
            expected_product_revision: current.miniapp.product_revision,
            project_id: current.project_id.clone(), expected_project_revision: current.project_revision,
            expected_build_generation: current.build_generation,
            expected_source_snapshot_digest: current.source_snapshot_digest.clone().unwrap(),
            path: path.into(), content,
        };
        assert!(application.replace_source_file(&uuid::Uuid::now_v7().to_string(), request.clone()).await.is_err());
        current = application.replace_source_file(&owner, request).await.unwrap();
    }
    let built = application.build(&owner, BuildPluginRuntimeRequest {
        miniapp_id: current.miniapp.miniapp_id.clone(), expected_product_revision: current.miniapp.product_revision,
        project_id: current.project_id.clone(), expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        expected_source_snapshot_digest: current.source_snapshot_digest.clone().unwrap(),
        expected_dependency_lock_digest: current.dependency_lock_digest.clone().unwrap(), service_lifecycle: None,
    }).await.unwrap();
    let ready = built.ready.unwrap();
    assert!(ready.service.is_some());
    assert!(!built.miniapp.surface_available);
    assert!(application.open_surface(&owner, &current.miniapp.miniapp_id).await.is_err());
    let record: String = sqlx::query_scalar("SELECT artifact_record_json FROM miniapp_release_artifacts WHERE artifact_id = ?")
        .bind(&ready.release.artifact_id).fetch_one(database.pool()).await.unwrap();
    let artifact: nomifun_agent_contracts::MiniAppReleaseArtifactV1 = serde_json::from_str(&record).unwrap();
    artifact.validate().unwrap();
    assert!(artifact.manifest.payload.ui.is_none());
    let capability = &artifact.manifest.payload.contributions.capabilities[0];
    assert_eq!(capability.id.as_ref(), format!("plugin.{}.echo", current.miniapp.miniapp_id));
    assert_eq!(capability.contributions.actions[0].action_id.as_ref(), "echo");
    assert_eq!(artifact.files.len(), 1);
    let destination = root.path().join("shared-background-plugin");
    let operation = application.export_share(&owner, nomifun_api_types::SharePluginRuntimeRequest {
        miniapp_id: built.miniapp.miniapp_id.clone(),
        expected_product_revision: built.miniapp.product_revision,
        expected_pointer_revision: built.miniapp.releases.pointer_revision,
        content: nomifun_api_types::PluginRuntimeShareContentDto::ReadyRelease,
        release_id: ready.release.release_id.clone(),
        expected_release_digest: ready.release.release_digest.clone(),
        destination_path: destination.to_string_lossy().into_owned(),
        include_source: true,
    }).await.unwrap();
    assert_eq!(operation.state, nomifun_api_types::DurableOperationStateDto::Succeeded);
    let exported: nomifun_agent_contracts::MiniAppReleaseArtifactV1 = serde_json::from_slice(
        &std::fs::read(destination.join("release/artifact.json")).unwrap(),
    ).unwrap();
    assert_eq!(exported.artifact_digest, artifact.artifact_digest);
    assert!(exported.manifest.payload.ui.is_none());
}
