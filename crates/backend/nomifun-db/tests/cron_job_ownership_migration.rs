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

async fn legacy_job(pool: &sqlx::SqlitePool, id: &str, conversation_id: Option<i64>) {
    sqlx::query(
        "INSERT INTO cron_jobs (\
             id, name, enabled, schedule_kind, schedule_value, payload_message, \
             execution_mode, conversation_id, agent_type, created_by, \
             created_at, updated_at, run_count, retry_count, max_retries\
         ) VALUES (?, ?, 1, 'every', '60000', 'ping', 'existing', ?, \
                   'nomi', 'user', 1, 1, 0, 0, 3)",
    )
    .bind(id)
    .bind(id)
    .bind(conversation_id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_038_materializes_owner_once_and_installs_cross_owner_guards() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(37).run(&pool).await.unwrap();

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
    for (id, owner) in [(101_i64, "owner-a"), (102, "owner-b"), (103, "owner-a")] {
        sqlx::query(
            "INSERT INTO conversations \
             (id, user_id, name, type, extra, status, created_at, updated_at) \
             VALUES (?, ?, 'owned', 'nomi', '{}', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }

    legacy_job(&pool, "cron_direct", Some(101)).await;
    legacy_job(&pool, "cron_inverse", None).await;
    legacy_job(&pool, "cron_unbound", None).await;
    sqlx::query("UPDATE conversations SET cron_job_id = 'cron_inverse' WHERE id = 103")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO cron_job_runs \
         (id, job_id, executed_at_ms, status, created_at_ms) \
         VALUES ('run_preserved', 'cron_direct', 10, 'ok', 10)",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator_through(38).run(&pool).await.unwrap();
    sqlx::query("PRAGMA foreign_keys = ON; PRAGMA legacy_alter_table = OFF")
        .execute(&pool)
        .await
        .unwrap();

    let owners: Vec<(String, String)> =
        sqlx::query_as("SELECT id, user_id FROM cron_jobs ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        owners,
        vec![
            ("cron_direct".into(), "owner-a".into()),
            ("cron_inverse".into(), "owner-a".into()),
            ("cron_unbound".into(), "system_default_user".into()),
        ]
    );

    let owner_not_null: i64 = sqlx::query_scalar(
        "SELECT [notnull] FROM pragma_table_info('cron_jobs') WHERE name = 'user_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(owner_not_null, 1);
    let preserved_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_job_runs WHERE id = 'run_preserved'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(preserved_runs, 1);

    assert!(
        sqlx::query("UPDATE cron_jobs SET user_id = 'owner-b' WHERE id = 'cron_direct'")
            .execute(&pool)
            .await
            .is_err(),
        "job ownership must be immutable"
    );
    assert!(
        sqlx::query("UPDATE cron_jobs SET conversation_id = 102 WHERE id = 'cron_direct'")
            .execute(&pool)
            .await
            .is_err(),
        "a job cannot bind another owner's Conversation"
    );
    assert!(
        sqlx::query("UPDATE conversations SET cron_job_id = 'cron_direct' WHERE id = 102")
            .execute(&pool)
            .await
            .is_err(),
        "a Conversation cannot bind another owner's job"
    );
    assert!(
        sqlx::query(
            "INSERT INTO conversation_artifacts \
             (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
             VALUES (102, 'cron_direct', 'skill_suggest', 'active', '{}', 1, 1)",
        )
        .execute(&pool)
        .await
        .is_err(),
        "an artifact cannot bind another owner's job"
    );
    sqlx::query(
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (101, 'cron_direct', 'skill_suggest', 'active', '{}', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE conversations SET user_id = 'owner-b' WHERE id = 101")
            .execute(&pool)
            .await
            .is_err(),
        "a Conversation with cron artifacts cannot move to another owner"
    );

    sqlx::query("DELETE FROM cron_jobs WHERE id = 'cron_direct'")
        .execute(&pool)
        .await
        .unwrap();
    let artifact_job: Option<String> = sqlx::query_scalar(
        "SELECT cron_job_id FROM conversation_artifacts WHERE conversation_id = 101",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(artifact_job, None, "artifact FK must still target rebuilt cron_jobs");
    let cascaded_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_job_runs WHERE id = 'run_preserved'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cascaded_runs, 0, "run FK must still cascade from rebuilt cron_jobs");

    let fk_violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fk_violations, 0);
}

#[tokio::test]
async fn migration_038_rejects_ambiguous_legacy_owner_instead_of_guessing() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(37).run(&pool).await.unwrap();

    for owner in ["owner-a", "owner-b"] {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES (?, ?, 'hash', 1, 1)",
        )
        .bind(owner)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }
    for (id, owner) in [(201_i64, "owner-a"), (202, "owner-b")] {
        sqlx::query(
            "INSERT INTO conversations \
             (id, user_id, name, type, extra, status, created_at, updated_at) \
             VALUES (?, ?, 'owned', 'nomi', '{}', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }
    legacy_job(&pool, "cron_ambiguous", Some(201)).await;
    sqlx::query("UPDATE conversations SET cron_job_id = 'cron_ambiguous' WHERE id = 202")
        .execute(&pool)
        .await
        .unwrap();

    let result = migrator_through(38).run(&pool).await;
    assert!(result.is_err(), "ambiguous owners must roll back migration 038");
    let has_user_id: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('cron_jobs') WHERE name = 'user_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(has_user_id, 0, "failed migration must leave legacy schema intact");
}

#[tokio::test]
async fn migration_038_rejects_cross_owner_legacy_cron_artifacts() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON")
        .execute(&pool)
        .await
        .unwrap();
    migrator_through(37).run(&pool).await.unwrap();

    for owner in ["owner-a", "owner-b"] {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, updated_at) \
             VALUES (?, ?, 'hash', 1, 1)",
        )
        .bind(owner)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }
    for (id, owner) in [(301_i64, "owner-a"), (302, "owner-b")] {
        sqlx::query(
            "INSERT INTO conversations \
             (id, user_id, name, type, extra, status, created_at, updated_at) \
             VALUES (?, ?, 'owned', 'nomi', '{}', 'pending', 1, 1)",
        )
        .bind(id)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }
    legacy_job(&pool, "cron_artifact_owner_a", Some(301)).await;
    sqlx::query(
        "INSERT INTO conversation_artifacts \
         (conversation_id, cron_job_id, kind, status, payload, created_at, updated_at) \
         VALUES (302, 'cron_artifact_owner_a', 'skill_suggest', 'active', '{}', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let result = migrator_through(38).run(&pool).await;
    assert!(
        result.is_err(),
        "a cross-owner legacy artifact must not be silently reassigned"
    );
    let has_user_id: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('cron_jobs') WHERE name = 'user_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(has_user_id, 0, "failed migration must roll back the rebuild");
}
