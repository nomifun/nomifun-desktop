use sqlx::sqlite::SqlitePoolOptions;
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

#[tokio::test]
async fn document_migration_preserves_draft_identity_data_and_revisions() {
    let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/093_miniapp_product_documents.sql")).execute(&pool).await.unwrap();
    let owner = "0190f5fe-7c00-7000-8000-000000000001";
    let plugin = "0190f5fe-7c00-7000-8000-000000000002";
    let content = serde_json::json!({"miniapp_id":plugin,"revision":7,"html":"user content","nested":{"miniapp_id":"immutable-provenance"}});
    sqlx::query("INSERT INTO miniapp_product_documents(owner_user_id, document_key, revision, content_json, updated_at) VALUES (?, 'draft:example', 7, ?, 42)")
        .bind(owner).bind(content.to_string()).execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/094_plugin_product_documents.sql")).execute(&pool).await.unwrap();
    let (revision, json, updated): (i64, String, i64) = sqlx::query_as("SELECT revision, content_json, updated_at FROM plugin_product_documents WHERE owner_user_id = ?")
        .bind(owner).fetch_one(&pool).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(revision, 7);
    assert_eq!(updated, 42);
    assert_eq!(value["revision"], 7);
    assert_eq!(value["plugin_id"], plugin);
    assert!(value.get("miniapp_id").is_none());
    assert_eq!(value["html"], "user content");
    assert_eq!(value["nested"]["miniapp_id"], "immutable-provenance");
    let aliases: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name = 'miniapp_product_documents'").fetch_one(&pool).await.unwrap();
    assert_eq!(aliases, 0);
}
