//! Migration 020 (`channel_plugins.owner_domain`) backfill + guard-trigger
//! tests over pre-migration data shapes.

use sqlx::migrate::{Migrate, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const CS_AGENT: &str = "0190f5fe-7c00-7a00-8abc-0123456789aa";
const BOT_CS_BOUND: &str = "0190f5fe-7c00-7a00-8abc-0123456789b1";
const BOT_DUAL_BOUND: &str = "0190f5fe-7c00-7a00-8abc-0123456789b2";
const BOT_UNBOUND: &str = "0190f5fe-7c00-7a00-8abc-0123456789b3";
const COMPANION: &str = "0190f5fe-7c00-7a00-8abc-0123456789c1";

/// Apply migrations up to (and including) `max_version`, skipping versions
/// already recorded in `_sqlx_migrations` so repeated calls are incremental.
async fn migrate_to(pool: &sqlx::SqlitePool, max_version: i64) {
    let mut conn = pool.acquire().await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let applied: std::collections::BTreeSet<i64> = conn
        .list_applied_migrations()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.version)
        .collect();
    for m in MIGRATOR.iter() {
        if m.version <= max_version && !applied.contains(&m.version) {
            conn.apply(m).await.unwrap();
        }
    }
}

async fn insert_bot(pool: &sqlx::SqlitePool, bot_id: &str, companion_id: Option<&str>) {
    sqlx::query(
        "INSERT INTO channel_plugins \
            (channel_plugin_id, type, name, enabled, config, companion_id, created_at, updated_at) \
         VALUES (?, 'telegram', 'bot', 0, 'enc', ?, 1, 1)",
    )
    .bind(bot_id)
    .bind(companion_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_pre_020(pool: &sqlx::SqlitePool) {
    migrate_to(pool, 19).await;
    sqlx::query(
        "INSERT INTO cs_agents (cs_agent_id, name, created_at, updated_at) VALUES (?, 'CS', 1, 1)",
    )
    .bind(CS_AGENT)
    .execute(pool)
    .await
    .unwrap();
    // A bot only bound by customer service (must move to the cs domain).
    insert_bot(pool, BOT_CS_BOUND, None).await;
    // A bot invalidly bound on both sides (companion wins, cs binding dropped).
    insert_bot(pool, BOT_DUAL_BOUND, Some(COMPANION)).await;
    // An untouched companion-pool bot.
    insert_bot(pool, BOT_UNBOUND, None).await;
    for bot in [BOT_CS_BOUND, BOT_DUAL_BOUND] {
        sqlx::query(
            "INSERT INTO cs_channel_bindings (cs_agent_id, channel_plugin_id, created_at) \
             VALUES (?, ?, 1)",
        )
        .bind(CS_AGENT)
        .bind(bot)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn owner_domain(pool: &sqlx::SqlitePool, bot_id: &str) -> String {
    sqlx::query_scalar("SELECT owner_domain FROM channel_plugins WHERE channel_plugin_id = ?")
        .bind(bot_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn backfill_moves_cs_bound_bots_and_repairs_dual_bindings() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_020(&pool).await;

    migrate_to(&pool, 20).await;

    assert_eq!(
        owner_domain(&pool, BOT_CS_BOUND).await,
        "customer_service",
        "a cs-bound, companion-free bot moves to the customer-service domain"
    );
    assert_eq!(
        owner_domain(&pool, BOT_DUAL_BOUND).await,
        "companion",
        "a dual-bound bot stays with the companion"
    );
    assert_eq!(
        owner_domain(&pool, BOT_UNBOUND).await,
        "companion",
        "an unbound bot defaults to the companion pool"
    );

    let bindings: Vec<String> =
        sqlx::query_scalar("SELECT channel_plugin_id FROM cs_channel_bindings ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        bindings,
        vec![BOT_CS_BOUND.to_owned()],
        "the dual-bound bot's customer-service binding is dropped"
    );
}

#[tokio::test]
async fn guard_triggers_reject_companion_bindings_on_cs_bots() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    seed_pre_020(&pool).await;
    migrate_to(&pool, 20).await;

    // UPDATE guard: a cs-domain bot cannot gain a companion binding.
    let err = sqlx::query(
        "UPDATE channel_plugins SET companion_id = ? WHERE channel_plugin_id = ?",
    )
    .bind(COMPANION)
    .bind(BOT_CS_BOUND)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("customer-service channel bots cannot carry a companion binding"),
        "update guard message: {err}"
    );

    // UPDATE guard: a companion-bound bot cannot switch to the cs domain.
    let err = sqlx::query(
        "UPDATE channel_plugins SET owner_domain = 'customer_service' WHERE channel_plugin_id = ?",
    )
    .bind(BOT_DUAL_BOUND)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("companion binding"));

    // INSERT guard: a new cs-domain row cannot carry a companion binding.
    let err = sqlx::query(
        "INSERT INTO channel_plugins \
            (channel_plugin_id, type, name, enabled, config, companion_id, owner_domain, \
             created_at, updated_at) \
         VALUES ('0190f5fe-7c00-7a00-8abc-0123456789b4', 'telegram', 'bot', 0, 'enc', ?, \
                 'customer_service', 1, 1)",
    )
    .bind(COMPANION)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("companion binding"));

    // The CHECK constraint rejects unknown domains outright.
    let err = sqlx::query(
        "UPDATE channel_plugins SET owner_domain = 'martian' WHERE channel_plugin_id = ?",
    )
    .bind(BOT_UNBOUND)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(err.to_string().contains("CHECK"));
}

#[tokio::test]
async fn fresh_database_passes_schema_contract_with_owner_domain() {
    // init_database_memory runs ALL migrations + the id schema contract
    // (which now requires the column, its default, and both guard triggers).
    let db = nomifun_db::init_database_memory().await.unwrap();
    let default_domain: String = sqlx::query_scalar(
        "SELECT dflt_value FROM pragma_table_info('channel_plugins') WHERE name = 'owner_domain'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(default_domain, "'companion'");
}
