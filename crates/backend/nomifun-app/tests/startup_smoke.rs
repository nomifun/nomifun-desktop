//! Smoke test: starting the app twice with the same binary version
//! should be a no-op on the second run (version gate skips rewrite).

use clap::Parser as _;
use nomifun_app::{AppConfig, DesktopServer, bootstrap};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::TempDir;

// Bootstrap publishes process-wide data and generation environment variables.
// Every test in this binary that starts a data layer uses this same lock.
static STARTUP: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

async fn startup_lock() -> tokio::sync::MutexGuard<'static, ()> {
    STARTUP.get_or_init(|| tokio::sync::Mutex::new(())).lock().await
}

async fn assert_desktop_health(server: &DesktopServer) {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{}/health", server.loopback_port()))
        .header("x-nomi-local-trust", server.local_trust_secret())
        .send()
        .await
        .expect("desktop backend health request");
    assert!(response.status().is_success(), "{}", response.status());
    let health: serde_json::Value = response.json().await.unwrap();
    assert_eq!(health["status"], "ok");
}

async fn observe_database(path: &Path) -> sqlx::SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false)
                .read_only(true)
                .busy_timeout(Duration::from_secs(5)),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn second_start_with_same_version_is_noop() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_skill_library::materialize_if_needed(
            data_dir,
            nomifun_skill_library::builtin_skills_corpus(),
            "test-1.0.0",
        )
        .await
        .unwrap();
    assert!(first, "first call should materialize");

    let second =
        nomifun_skill_library::materialize_if_needed(
            data_dir,
            nomifun_skill_library::builtin_skills_corpus(),
            "test-1.0.0",
        )
        .await
        .unwrap();
    assert!(!second, "second call with same version should skip");
}

#[tokio::test]
async fn version_bump_triggers_rewrite() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_skill_library::materialize_if_needed(
            data_dir,
            nomifun_skill_library::builtin_skills_corpus(),
            "test-1.0.0",
        )
        .await
        .unwrap();
    assert!(first);

    let second =
        nomifun_skill_library::materialize_if_needed(
            data_dir,
            nomifun_skill_library::builtin_skills_corpus(),
            "test-2.0.0",
        )
        .await
        .unwrap();
    assert!(second, "version change should trigger a fresh materialize");

    let version = std::fs::read_to_string(data_dir.join("builtin-skills").join(".version")).unwrap();
    assert_eq!(version, "test-2.0.0");
}

#[tokio::test]
async fn retired_parallel_root_files_are_not_opened_or_modified() {
    let _startup = startup_lock().await;
    let tmp = TempDir::new().unwrap();
    let retired_database = tmp.path().join("nomifun-v4.db");
    let retired_marker = tmp.path().join(".nomifun-v4-ready.json");
    std::fs::write(&retired_database, b"opaque retired database").unwrap();
    std::fs::write(&retired_marker, b"opaque retired marker").unwrap();
    let database_before = std::fs::read(&retired_database).unwrap();
    let marker_before = std::fs::read(&retired_marker).unwrap();

    let config = AppConfig {
        data_dir: tmp.path().to_path_buf(),
        work_dir: tmp.path().to_path_buf(),
        ..AppConfig::default()
    };
    let database = bootstrap::init_data_layer(&config).await.unwrap();
    database.close().await;

    assert!(config.database_path().is_file());
    assert!(tmp.path().join("builtin-skills").is_dir());
    assert_eq!(std::fs::read(retired_database).unwrap(), database_before);
    assert_eq!(std::fs::read(retired_marker).unwrap(), marker_before);
}


#[tokio::test]
async fn desktop_startup_uses_the_canonical_data_root() {
    let _startup = startup_lock().await;
    let tmp = TempDir::new().unwrap();
    let work_parent = TempDir::new().unwrap();
    let work = work_parent.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let cli = nomifun_app::cli::Cli::parse_from([
        "nomifun-desktop-startup-test",
        "--data-dir",
        tmp.path().to_str().unwrap(),
        "--work-dir",
        work.to_str().unwrap(),
    ]);

    let (server, _keep_alive) = DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        nomifun_app::DesktopHostServices::default(),
    )
    .await
    .expect("Nomi-core desktop startup");
    assert!(server.loopback_port() > 0);
    assert_desktop_health(&server).await;
    server.shutdown_all().await.unwrap();
    assert!(tmp.path().join("nomifun-backend.db").is_file());
    assert!(tmp.path().join("builtin-skills").is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incompatible_87_row_upgrade_rebuilds_once_and_recovers_interrupted_bootstrap() {
    let _startup = startup_lock().await;
    for (receipt_state, interrupt_after_rebuild, matching_baseline) in [
        ("current", false, false),
        ("missing", false, false),
        ("invalid", false, false),
        ("current", true, false),
        // 0.8.1 has the same baseline checksum and more migration receipts.
        // More rows must not be mistaken for a future application version.
        ("current", false, true),
    ] {
        let root = TempDir::new().unwrap();
        let data = root.path().join("data");
        let work = root.path().join("work");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let config = AppConfig {
            data_dir: data.clone(),
            work_dir: work.clone(),
            work_dir_is_cli_override: true,
            ..AppConfig::default()
        };

        // A previous installation may already have consumed its pre-v3
        // retirement. Produce the real durable marker, rather than inventing
        // its private control format, before simulating the later old lineage.
        std::fs::write(config.database_path(), b"retired legacy dataset").unwrap();
        nomifun_common::factory_reset::retire_non_v3_dataset_after_probe(&data, &work)
            .unwrap();
        let old_database = bootstrap::init_data_layer(&config).await.unwrap();
        bootstrap::finalize_data_layer(&config).unwrap();
        let legacy_marker = data.join(
            "retired-datasets/automatic-legacy-retirement.completed.json",
        );
        let legacy_marker_before = std::fs::read(&legacy_marker).unwrap();
        let old_generation = std::fs::read(data.join("storage-generation")).unwrap();
        let old_owner = nomifun_db::installation_owner_id(old_database.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO client_preferences(key,value,updated_at) \
             VALUES ('upgrade-old-preference','retired user setting',1)",
        )
        .execute(old_database.pool())
        .await
        .unwrap();

        // Exercise the 87-row lineage and the 0.8.1-style matching baseline
        // with retired additive receipts, both retaining v3 identity tables.
        // No historical reader or embedded retired schema is needed here.
        let baseline_checksum: Vec<u8> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version = 1",
        ).fetch_one(old_database.pool()).await.unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations")
            .execute(old_database.pool())
            .await
            .unwrap();
        for version in 1_i64..=if matching_baseline { 3 } else { 87 } {
            sqlx::query(
                "INSERT INTO _sqlx_migrations(version,description,success,checksum,execution_time) \
                 VALUES (?,'incompatible upgrade fixture',1,?,0)",
            )
            .bind(version)
            .bind(if matching_baseline && version == 1 {
                baseline_checksum.clone()
            } else {
                vec![version as u8; 48]
            })
            .execute(old_database.pool())
            .await
            .unwrap();
        }
        old_database.close().await;
        let receipt_path = data.join(nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE);
        match receipt_state {
            "current" => {}
            "missing" => std::fs::remove_file(&receipt_path).unwrap(),
            "invalid" => std::fs::write(&receipt_path, b"invalid old dataset receipt").unwrap(),
            _ => unreachable!(),
        }
        std::fs::create_dir_all(data.join("attachments")).unwrap();
        std::fs::write(data.join("attachments/retired.txt"), b"retired attachment").unwrap();
        std::fs::create_dir_all(data.join("browser-v4/agent-sessions/retired-session")).unwrap();
        std::fs::write(
            data.join("browser-v4/agent-sessions/retired-session/profile.txt"),
            b"retired browser profile",
        )
        .unwrap();
        std::fs::create_dir_all(work.join("conversations/retired-session")).unwrap();
        std::fs::write(
            work.join("conversations/retired-session/history.txt"),
            b"retired Agent history",
        )
        .unwrap();
        std::fs::write(work.join("unrelated-user-file.txt"), b"keep external work").unwrap();

        let rebuilt_generation = if interrupt_after_rebuild {
            let rebuilt = bootstrap::init_data_layer(&config).await.unwrap();
            rebuilt.close().await;
            assert!(nomifun_common::factory_reset::read_pending_v3_reset(&data, &work)
                .unwrap()
                .is_some(), "data-layer creation alone must leave finalization pending");
            Some(std::fs::read(data.join("storage-generation")).unwrap())
        } else {
            None
        };
        let cli = nomifun_app::cli::Cli::parse_from([
            "nomifun-upgrade-startup-test",
            "--data-dir",
            data.to_str().unwrap(),
            "--work-dir",
            work.to_str().unwrap(),
        ]);
        let (server, keep_alive) = tokio::time::timeout(
            Duration::from_secs(30),
            DesktopServer::start_with_outcome(
                &cli, "", None, None, None, nomifun_app::DesktopHostServices::default(),
            ),
        )
        .await
        .expect("upgrade desktop startup must finish")
        .expect("incompatible lineage must bootstrap a usable desktop backend");
        assert_desktop_health(&server).await;
        assert!(!data.join("attachments/retired.txt").exists());
        assert!(!data.join("browser-v4/agent-sessions/retired-session").exists());
        assert!(!work.join("conversations/retired-session").exists());
        assert_eq!(std::fs::read(work.join("unrelated-user-file.txt")).unwrap(), b"keep external work");
        assert_eq!(std::fs::read(&legacy_marker).unwrap(), legacy_marker_before);
        assert!(nomifun_common::factory_reset::read_pending_v3_reset(&data, &work)
            .unwrap()
            .is_none(), "the complete desktop bootstrap must finalize the reset");
        let generation = std::fs::read(data.join("storage-generation")).unwrap();
        assert_ne!(generation, old_generation);
        if let Some(rebuilt_generation) = rebuilt_generation {
            assert_eq!(generation, rebuilt_generation, "restart must resume the already rebuilt generation");
        }
        let observer = observe_database(&config.database_path()).await;
        nomifun_db::validate_current_migration_lineage(&observer).await.unwrap();
        let owner = nomifun_db::installation_owner_id(&observer).await.unwrap();
        assert_ne!(owner, old_owner);
        let old_preferences: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM client_preferences WHERE key='upgrade-old-preference'",
        )
        .fetch_one(&observer)
        .await
        .unwrap();
        assert_eq!(old_preferences, 0, "destructive upgrade must not import old configuration");
        observer.close().await;
        server.shutdown_all().await.unwrap();
        drop(server);
        drop(keep_alive);

        let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
        sqlx::query(
            "INSERT INTO client_preferences(key,value,updated_at) \
             VALUES ('upgrade-current-preference','new user setting',1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        database.close().await;
        let current_attachment = data.join("attachments/0190f5fe-7c00-7a00-8000-000000000121/current.txt");
        std::fs::create_dir_all(current_attachment.parent().unwrap()).unwrap();
        std::fs::write(&current_attachment, b"current attachment").unwrap();

        let (server, keep_alive) = tokio::time::timeout(
            Duration::from_secs(30),
            DesktopServer::start_with_outcome(
                &cli, "", None, None, None, nomifun_app::DesktopHostServices::default(),
            ),
        )
        .await
        .expect("second desktop startup must finish")
        .expect("current dataset must reopen without rebuilding");
        assert_desktop_health(&server).await;
        assert_eq!(std::fs::read(data.join("storage-generation")).unwrap(), generation);
        assert_eq!(std::fs::read(&current_attachment).unwrap(), b"current attachment");
        let observer = observe_database(&config.database_path()).await;
        assert_eq!(nomifun_db::installation_owner_id(&observer).await.unwrap(), owner);
        let preference: String = sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key='upgrade-current-preference'",
        )
        .fetch_one(&observer)
        .await
        .unwrap();
        assert_eq!(preference, "new user setting");
        observer.close().await;
        server.shutdown_all().await.unwrap();
        drop(server);
        drop(keep_alive);
    }
}
