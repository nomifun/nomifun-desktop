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

async fn seed_runtime_rows(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
         VALUES \
            ('system_default_user', 'system_default_user', '', 1, 1), \
            ('secondary-user', 'secondary-user', 'hash', 1, 1)",
    )
    .execute(pool)
    .await
    .unwrap();

    for (id, user_id, name) in [
        (101_i64, "system_default_user", "owner conversation"),
        (102_i64, "secondary-user", "secondary conversation"),
    ] {
        sqlx::query(
            "INSERT INTO conversations \
             (id, user_id, name, type, extra, status, created_at, updated_at) \
             VALUES (?, ?, ?, 'nomi', '{}', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(user_id)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    for (id, user_id, name) in [
        (201_i64, "system_default_user", "owner terminal"),
        (202_i64, "secondary-user", "secondary terminal"),
    ] {
        sqlx::query(
            "INSERT INTO terminal_sessions \
             (id, name, cwd, command, args, created_at, updated_at, user_id) \
             VALUES (?, ?, '/tmp', '$SHELL', '[]', 1, 1, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn migration_040_cleans_cross_owner_state_and_enforces_both_directions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(39).run(&pool).await.unwrap();
    seed_runtime_rows(&pool).await;

    sqlx::query(
        "INSERT INTO knowledge_bases \
         (id, name, description, root_path, managed, extra, created_at, updated_at) \
         VALUES ('kb-owner-boundary', 'KB', '', '/tmp/kb', 1, '{}', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    for (kind, column, target) in [
        ("conversation", "target_conv_id", 101_i64),
        ("conversation", "target_conv_id", 102_i64),
        ("terminal", "target_term_id", 201_i64),
        ("terminal", "target_term_id", 202_i64),
    ] {
        let sql = format!(
            "INSERT INTO knowledge_bindings \
             (target_kind, {column}, enabled, writeback, writeback_mode, \
              writeback_eagerness, channel_write_enabled, updated_at) \
             VALUES (?, ?, 1, 0, 'staged', 'conservative', 0, 1)"
        );
        sqlx::query(&sql)
            .bind(kind)
            .bind(target)
            .execute(&pool)
            .await
            .unwrap();
    }

    for (id, kind, target) in [
        (1_i64, "conversation", 101_i64),
        (2_i64, "conversation", 102_i64),
        (3_i64, "terminal", 201_i64),
        (4_i64, "terminal", 202_i64),
        (5_i64, "conversation", 999_i64),
    ] {
        sqlx::query(
            "INSERT INTO requirements \
             (id, title, tag, status, owner_session_id, owner_kind, \
              active_turn_started_at, lease_expires_at, created_at, updated_at) \
             VALUES (?, 'claim', 'boundary', 'in_progress', ?, ?, 10, 20, 1, 1)",
        )
        .bind(id)
        .bind(target)
        .bind(kind)
        .execute(&pool)
        .await
        .unwrap();
    }

    migrator_through(40).run(&pool).await.unwrap();

    let bindings: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT target_kind, target_conv_id, target_term_id \
         FROM knowledge_bindings ORDER BY binding_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        bindings,
        vec![
            ("conversation".into(), Some(101), None),
            ("terminal".into(), None, Some(201)),
        ],
        "legacy bindings to secondary-user targets must be deleted"
    );

    let claims: Vec<(i64, String, Option<i64>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT id, status, owner_session_id, owner_kind, lease_expires_at \
         FROM requirements ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(claims[0].1, "in_progress");
    assert_eq!(claims[0].2, Some(101));
    assert_eq!(claims[2].1, "in_progress");
    assert_eq!(claims[2].2, Some(201));
    for row in [&claims[1], &claims[3], &claims[4]] {
        assert_eq!(row.1, "pending");
        assert!(row.2.is_none() && row.3.is_none() && row.4.is_none());
    }

    let forged_binding = sqlx::query(
        "INSERT INTO knowledge_bindings \
         (target_kind, target_conv_id, enabled, writeback, writeback_mode, \
          writeback_eagerness, channel_write_enabled, updated_at) \
         VALUES ('conversation', 102, 1, 0, 'staged', 'conservative', 0, 2)",
    )
    .execute(&pool)
    .await;
    assert!(forged_binding.is_err(), "raw SQL cannot bind Knowledge to a secondary user");

    let forged_claim = sqlx::query(
        "UPDATE requirements \
         SET status='in_progress', owner_session_id=202, owner_kind='terminal' \
         WHERE id=5",
    )
    .execute(&pool)
    .await;
    assert!(forged_claim.is_err(), "raw SQL cannot claim a Requirement for a secondary user");

    let rewrite_bound_conversation =
        sqlx::query("UPDATE conversations SET user_id='secondary-user' WHERE id=101")
            .execute(&pool)
            .await;
    assert!(
        rewrite_bound_conversation.is_err(),
        "ownership cannot be changed behind a Knowledge binding or Requirement claim"
    );

    sqlx::query("DELETE FROM conversations WHERE id=101")
        .execute(&pool)
        .await
        .unwrap();
    let released: (String, Option<i64>, Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT status, owner_session_id, owner_kind, lease_expires_at \
         FROM requirements WHERE id=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(released, ("pending".into(), None, None, None));

    // The owner path remains valid after the hard cut.
    sqlx::query(
        "UPDATE requirements \
         SET status='in_progress', owner_session_id=201, owner_kind='terminal' \
         WHERE id=5",
    )
    .execute(&pool)
    .await
    .unwrap();
}
