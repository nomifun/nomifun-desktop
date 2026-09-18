use nomifun_db::{IPluginN1Repository, PluginProductDocuments, SqlitePluginN1Repository, init_database_memory, installation_owner_id};

#[tokio::test]
async fn workspace_uses_provider_ownership_and_purges_membership_in_the_delete_transaction() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let plugin = "0190f5fe-7c00-7000-8000-000000000002";
    sqlx::query("INSERT INTO plugin_mounts(mount_id, package_id, data_dir_path, revision, delete_pending, created_at, updated_at) VALUES (?, 'plugin.example', ?, 1, 1, 1, 1)")
        .bind(plugin).bind(plugin).execute(database.pool()).await.unwrap();
    let documents = PluginProductDocuments::new(database.pool().clone());
    assert_eq!(documents.owned_plugin_ids(&owner).await.unwrap(), vec![plugin.to_owned()]);
    assert!(documents.owned_plugin_ids("0190f5fe-7c00-7000-8000-000000000099").await.unwrap().is_empty());
    let content = serde_json::json!({"revision":1,"items":{plugin:{"pinned":true}}});
    documents.put(&owner, "library", 0, &content.to_string()).await.unwrap();
    let repository = SqlitePluginN1Repository::new(database.pool().clone());
    assert!(repository.complete_mount_data_delete(plugin).await.unwrap());
    let (revision, json) = documents.get(&owner, "library").await.unwrap().unwrap();
    assert_eq!(revision, 2);
    let content: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(content["items"].get(plugin).is_none());
    assert_eq!(content["revision"], 2);
}
