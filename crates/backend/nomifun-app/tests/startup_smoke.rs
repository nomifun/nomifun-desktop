//! Smoke test: starting the app twice with the same binary version
//! should be a no-op on the second run (version gate skips rewrite).

use clap::Parser as _;
use nomifun_app::{AppConfig, DesktopServer, bootstrap};
use tempfile::TempDir;

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
    server.shutdown_all().await.unwrap();
    assert!(tmp.path().join("nomifun-backend.db").is_file());
    assert!(tmp.path().join("builtin-skills").is_dir());
}
