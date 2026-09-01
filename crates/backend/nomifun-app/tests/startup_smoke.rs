//! Smoke test: starting the app twice with the same binary version
//! should be a no-op on the second run (version gate skips rewrite).

use clap::Parser as _;
use nomifun_app::{AppConfig, DesktopServer, bootstrap};
use nomifun_v4_root::{FRESH_V4_DATABASE_FILE, FreshV4Coordinator};
use tempfile::TempDir;

#[tokio::test]
async fn second_start_with_same_version_is_noop() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(first, "first call should materialize");

    let second =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(!second, "second call with same version should skip");
}

#[tokio::test]
async fn version_bump_triggers_rewrite() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(first);

    let second =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-2.0.0")
            .await
            .unwrap();
    assert!(second, "version change should trigger a fresh materialize");

    let version = std::fs::read_to_string(data_dir.join("builtin-skills").join(".version")).unwrap();
    assert_eq!(version, "test-2.0.0");
}

#[tokio::test]
async fn ready_v4_startup_fails_closed_without_legacy_database_or_shell_assets() {
    let tmp = TempDir::new().unwrap();
    let v4_identity = format!("nomifun-app@{}", env!("CARGO_PKG_VERSION"));
    FreshV4Coordinator::default()
        .bootstrap(tmp.path(), &v4_identity, &[])
        .await
        .unwrap();

    let config = AppConfig {
        data_dir: tmp.path().to_path_buf(),
        work_dir: tmp.path().to_path_buf(),
        ..AppConfig::default()
    };
    let error = bootstrap::init_data_layer(&config)
        .await
        .expect_err("ready-v4 startup must not fall through to the v3 shell");

    assert!(error
        .to_string()
        .contains("legacy v3 data-layer initialization is fenced"));
    assert!(!config.database_path().exists());
    assert!(!tmp.path().join("builtin-skills").exists());
    assert!(tmp.path().join(FRESH_V4_DATABASE_FILE).is_file());
}

#[tokio::test]
async fn embedded_server_selects_fresh_v4_host_before_any_legacy_data_layer() {
    let tmp = TempDir::new().unwrap();
    let cli = nomifun_app::cli::Cli::parse_from([
        "nomicore-startup-test",
        "--data-dir",
        tmp.path().to_str().unwrap(),
    ]);

    let env = bootstrap::init_environment(&cli, "").unwrap();
    let host = env.canonical_host().unwrap();
    let application = host.compose(&env.config).await.unwrap();
    assert_eq!(
        application
            .platform()
            .materialized_registry()
            .unwrap()
            .capabilities
            .len(),
        137
    );
    application.close().await.unwrap();
    assert!(tmp.path().join(FRESH_V4_DATABASE_FILE).is_file());
    assert!(!tmp.path().join("nomifun-backend.db").exists());
    assert!(!tmp.path().join("builtin-skills").exists());
}

#[tokio::test]
async fn desktop_fresh_v4_startup_selects_host_before_legacy_data_layer() {
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
    )
    .await
    .expect("Fresh-v4 desktop startup");
    assert!(server.loopback_port() > 0);
    server.shutdown_all().await.unwrap();
    assert!(tmp.path().join(FRESH_V4_DATABASE_FILE).is_file());
    assert!(!tmp.path().join("nomifun-backend.db").exists());
    assert!(!tmp.path().join("builtin-skills").exists());
}
