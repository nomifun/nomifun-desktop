//! Audit coverage for the frozen pre-M1 `miniapps` table.
//!
//! Production code has no repository for this table. Migrations 028/029 and
//! the table remain immutable so existing database checksums and schema
//! validation continue to work.

use nomifun_common::ConversationId;
use nomifun_db::{
    IConversationRepository, SqliteConversationRepository, init_database_memory,
    installation_owner_id,
};

#[tokio::test]
async fn retired_table_remains_in_the_frozen_schema() {
    let database = init_database_memory().await.expect("database");

    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('miniapps') ORDER BY cid")
            .fetch_all(database.pool())
            .await
            .expect("legacy miniapps columns");
    assert_eq!(
        columns,
        [
            "id",
            "miniapp_id",
            "user_id",
            "name",
            "description",
            "icon",
            "html",
            "html_size",
            "source_conversation_id",
            "created_at",
            "updated_at",
            "published_at",
        ]
    );

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema
         WHERE type = 'index'
           AND tbl_name = 'miniapps'
           AND name NOT LIKE 'sqlite_autoindex_%'
         ORDER BY name",
    )
    .fetch_all(database.pool())
    .await
    .expect("legacy miniapps indexes");
    assert_eq!(
        indexes,
        [
            "idx_miniapps_source_conversation_id",
            "idx_miniapps_user_id",
        ]
    );

    nomifun_db::validate_id_schema_contract(database.pool())
        .await
        .expect("frozen schema remains registered");
}

#[tokio::test]
async fn deleting_a_conversation_keeps_retired_provenance_inert() {
    let database = init_database_memory().await.expect("database");
    let owner_user_id = installation_owner_id(database.pool())
        .await
        .expect("installation owner");
    let conversation_id = ConversationId::new().as_str().to_owned();
    let legacy_miniapp_id = "0190f5fe-7c00-7000-8000-000000000301";

    sqlx::query(
        "INSERT INTO conversations (
            conversation_id, user_id, name, type, created_at, updated_at
         ) VALUES (?, ?, 'retired MiniApp provenance', 'nomi', 1, 1)",
    )
    .bind(&conversation_id)
    .bind(&owner_user_id)
    .execute(database.pool())
    .await
    .expect("source conversation");
    sqlx::query(
        "INSERT INTO miniapps (
            miniapp_id, user_id, name, description, html, html_size,
            source_conversation_id, created_at, updated_at
         ) VALUES (?, ?, 'retired', '', '<p/>', 4, ?, 1, 1)",
    )
    .bind(legacy_miniapp_id)
    .bind(&owner_user_id)
    .bind(&conversation_id)
    .execute(database.pool())
    .await
    .expect("retired MiniApp audit fixture");

    SqliteConversationRepository::new(database.pool().clone())
        .delete(&conversation_id)
        .await
        .expect("delete conversation");

    let retained_source: Option<String> = sqlx::query_scalar(
        "SELECT source_conversation_id FROM miniapps WHERE miniapp_id = ?",
    )
    .bind(legacy_miniapp_id)
    .fetch_one(database.pool())
    .await
    .expect("retired provenance");
    assert_eq!(retained_source.as_deref(), Some(conversation_id.as_str()));

    nomifun_db::validate_id_data_contract(database.pool())
        .await
        .expect("KeepHistory permits the retired provenance token");
}
