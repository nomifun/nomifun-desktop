//! Migration 033 group-chat policy, authorization-kind, and chat-kind tests.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const COMPANION_PLUGIN: &str = "0190f5fe-7c00-7a00-8abc-0123456789a1";
const CUSTOMER_SERVICE_PLUGIN: &str = "0190f5fe-7c00-7a00-8abc-0123456789a2";
const CHANNEL_USER: &str = "0190f5fe-7c00-7a00-8abc-0123456789b1";
const CUSTOMER_SERVICE_CHANNEL_USER: &str = "0190f5fe-7c00-7a00-8abc-0123456789b2";
const CHANNEL_SESSION: &str = "0190f5fe-7c00-7a00-8abc-0123456789c1";

async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    for migration in MIGRATOR.iter() {
        if migration.version <= max_version && !applied.contains(&migration.version) {
            conn.apply(migration).await.unwrap();
        }
    }
}

async fn seed_pre_033(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 32).await;
    for (plugin_id, owner_domain) in [
        (COMPANION_PLUGIN, "companion"),
        (CUSTOMER_SERVICE_PLUGIN, "customer_service"),
    ] {
        sqlx::query(
            "INSERT INTO channel_plugins \
                (channel_plugin_id, type, name, enabled, config, owner_domain, created_at, updated_at) \
             VALUES (?, 'lark', 'bot', 0, 'enc', ?, 1, 1)",
        )
        .bind(plugin_id)
        .bind(owner_domain)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO channel_users \
            (channel_user_id, platform_user_id, platform_type, channel_plugin_id, authorized_at) \
         VALUES (?, 'ou_legacy', 'lark', ?, 1)",
    )
    .bind(CHANNEL_USER)
    .bind(COMPANION_PLUGIN)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_users \
            (channel_user_id, platform_user_id, platform_type, channel_plugin_id, authorized_at) \
         VALUES (?, 'ou_legacy_guest', 'lark', ?, 1)",
    )
    .bind(CUSTOMER_SERVICE_CHANNEL_USER)
    .bind(CUSTOMER_SERVICE_PLUGIN)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO channel_sessions \
            (channel_session_id, channel_user_id, agent_type, chat_id, channel_plugin_id, \
             created_at, last_activity) \
         VALUES (?, ?, 'acp', 'legacy-chat', ?, 1, 1)",
    )
    .bind(CHANNEL_SESSION)
    .bind(CHANNEL_USER)
    .bind(COMPANION_PLUGIN)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn backfill_preserves_existing_authorization_and_customer_service_access() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_033(&pool).await;

    migrate_to(&pool, 33).await;

    let companion_mode: String = sqlx::query_scalar(
        "SELECT group_access_mode FROM channel_plugins WHERE channel_plugin_id = ?",
    )
    .bind(COMPANION_PLUGIN)
    .fetch_one(&pool)
    .await
    .unwrap();
    let customer_service_mode: String = sqlx::query_scalar(
        "SELECT group_access_mode FROM channel_plugins WHERE channel_plugin_id = ?",
    )
    .bind(CUSTOMER_SERVICE_PLUGIN)
    .fetch_one(&pool)
    .await
    .unwrap();
    let authorization_kind: String = sqlx::query_scalar(
        "SELECT authorization_kind FROM channel_users WHERE channel_user_id = ?",
    )
    .bind(CHANNEL_USER)
    .fetch_one(&pool)
    .await
    .unwrap();
    let customer_service_authorization_kind: String = sqlx::query_scalar(
        "SELECT authorization_kind FROM channel_users WHERE channel_user_id = ?",
    )
    .bind(CUSTOMER_SERVICE_CHANNEL_USER)
    .fetch_one(&pool)
    .await
    .unwrap();
    let chat_kind: String = sqlx::query_scalar(
        "SELECT chat_kind FROM channel_sessions WHERE channel_session_id = ?",
    )
    .bind(CHANNEL_SESSION)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(companion_mode, "allowlist");
    assert_eq!(customer_service_mode, "all_members");
    assert_eq!(authorization_kind, "approved");
    assert_eq!(customer_service_authorization_kind, "auto_group");
    assert_eq!(chat_kind, "unknown");
}

#[tokio::test]
async fn check_constraints_reject_unknown_group_authorization_and_chat_values() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_033(&pool).await;
    migrate_to(&pool, 33).await;

    let invalid_mode = sqlx::query(
        "UPDATE channel_plugins SET group_access_mode = 'members' WHERE channel_plugin_id = ?",
    )
    .bind(COMPANION_PLUGIN)
    .execute(&pool)
    .await
    .unwrap_err();
    let invalid_authorization = sqlx::query(
        "UPDATE channel_users SET authorization_kind = 'implicit' WHERE channel_user_id = ?",
    )
    .bind(CHANNEL_USER)
    .execute(&pool)
    .await
    .unwrap_err();
    let invalid_chat = sqlx::query(
        "UPDATE channel_sessions SET chat_kind = 'room' WHERE channel_session_id = ?",
    )
    .bind(CHANNEL_SESSION)
    .execute(&pool)
    .await
    .unwrap_err();

    assert!(invalid_mode.to_string().contains("CHECK"));
    assert!(invalid_authorization.to_string().contains("CHECK"));
    assert!(invalid_chat.to_string().contains("CHECK"));
}

#[tokio::test]
async fn fresh_database_passes_schema_contract_with_group_access_metadata() {
    let db = nomifun_db::init_database_memory().await.unwrap();
    for (table, column, expected) in [
        ("channel_plugins", "group_access_mode", "'allowlist'"),
        ("channel_users", "authorization_kind", "'approved'"),
        ("channel_sessions", "chat_kind", "'unknown'"),
    ] {
        let default_value: String = sqlx::query_scalar(&format!(
            "SELECT dflt_value FROM pragma_table_info('{table}') WHERE name = ?"
        ))
        .bind(column)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(default_value, expected);
    }
}
