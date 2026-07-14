use std::borrow::Cow;

use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;

static ALL_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

fn migrator_through(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            ALL_MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: false,
        no_tx: false,
    }
}

#[tokio::test]
async fn migration_039_backfills_resolvable_owners_and_drops_orphan_audit_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    // Reproduce the actual contiguous upgrade boundary: unified Agent
    // Execution (037) and cron ownership (038) are already installed before
    // IDMM ownership (039) rebuilds the audit table.
    migrator_through(38).run(&pool).await.unwrap();

    for (id, username) in [("owner-a", "owner-a"), ("owner-b", "owner-b")] {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES (?, ?, 'hash', 1, 1)",
        )
        .bind(id)
        .bind(username)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO conversations \
         (id, user_id, name, type, extra, status, created_at, updated_at) \
         VALUES (101, 'owner-a', 'owned conversation', 'nomi', '{}', 'pending', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO terminal_sessions \
         (id, name, cwd, command, args, created_at, updated_at, user_id) \
         VALUES (201, 'owned terminal', '/tmp', '$SHELL', '[]', 1, 1, 'owner-b')",
    )
    .execute(&pool)
    .await
    .unwrap();

    for (id, kind, target) in [
        ("idmmrec_conversation", "conversation", "101"),
        ("idmmrec_terminal", "terminal", "201"),
        ("idmmrec_deleted_conversation", "conversation", "999"),
        ("idmmrec_unknown_kind", "unknown", "101"),
    ] {
        sqlx::query(
            "INSERT INTO idmm_interventions \
             (id, target_kind, target_id, watch, at, signal, tier_used, action, outcome) \
             VALUES (?, ?, ?, 'decision', 1, 'decision', 'rule', 'wait', 'applied')",
        )
        .bind(id)
        .bind(kind)
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();
    }

    migrator_through(39).run(&pool).await.unwrap();

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, user_id FROM idmm_interventions ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("idmmrec_conversation".into(), "owner-a".into()),
            ("idmmrec_terminal".into(), "owner-b".into()),
        ]
    );

    let user_id_not_null: i64 = sqlx::query_scalar(
        "SELECT [notnull] FROM pragma_table_info('idmm_interventions') WHERE name = 'user_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(user_id_not_null, 1);

    let invalid_kind = sqlx::query(
        "INSERT INTO idmm_interventions \
         (id, user_id, target_kind, target_id, watch, at, signal, tier_used, action, outcome) \
         VALUES ('idmmrec_invalid_kind', 'owner-b', 'unknown', '201', \
                 'decision', 2, 'decision', 'rule', 'wait', 'applied')",
    )
    .execute(&pool)
    .await;
    assert!(
        invalid_kind.is_err(),
        "the rebuilt table must reject audit rows outside the supported target domains"
    );

    let cross_owner = sqlx::query(
        "INSERT INTO idmm_interventions \
         (id, user_id, target_kind, target_id, watch, at, signal, tier_used, action, outcome) \
         VALUES ('idmmrec_cross_owner', 'owner-b', 'conversation', '101', \
                 'decision', 2, 'decision', 'rule', 'wait', 'applied')",
    )
    .execute(&pool)
    .await;
    assert!(
        cross_owner.is_err(),
        "raw SQL must not forge an audit row for another user's target"
    );

    let unknown_conversation = sqlx::query(
        "INSERT INTO idmm_interventions \
         (id, user_id, target_kind, target_id, watch, at, signal, tier_used, action, outcome) \
         VALUES ('idmmrec_unknown_conversation', 'owner-a', 'conversation', '999', \
                 'decision', 2, 'decision', 'rule', 'wait', 'applied')",
    )
    .execute(&pool)
    .await;
    assert!(
        unknown_conversation.is_err(),
        "raw SQL must not retain an audit row for a missing conversation"
    );

    let unknown_terminal = sqlx::query(
        "INSERT INTO idmm_interventions \
         (id, user_id, target_kind, target_id, watch, at, signal, tier_used, action, outcome) \
         VALUES ('idmmrec_unknown_terminal', 'owner-b', 'terminal', '999', \
                 'decision', 2, 'decision', 'rule', 'wait', 'applied')",
    )
    .execute(&pool)
    .await;
    assert!(
        unknown_terminal.is_err(),
        "raw SQL must not retain an audit row for a missing terminal"
    );

    let rewrite_audit = sqlx::query(
        "UPDATE idmm_interventions SET outcome = 'halted' \
         WHERE id = 'idmmrec_terminal'",
    )
    .execute(&pool)
    .await;
    assert!(
        rewrite_audit.is_err(),
        "published IDMM audit facts must be immutable even through raw SQL"
    );

    let rewrite_owner = sqlx::query(
        "UPDATE idmm_interventions SET user_id = 'owner-a' \
         WHERE id = 'idmmrec_terminal'",
    )
    .execute(&pool)
    .await;
    assert!(rewrite_owner.is_err(), "audit ownership must be immutable");

    sqlx::query("DELETE FROM users WHERE id = 'owner-a'")
        .execute(&pool)
        .await
        .unwrap();
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT id FROM idmm_interventions ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec!["idmmrec_terminal"]);
}
