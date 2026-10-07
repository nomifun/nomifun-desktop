use nomi_config::logging::{ResolvedLogging, create_file_layer};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn invalid_log_filter_does_not_create_files_or_workers() {
    let temp = tempfile::tempdir().unwrap();
    let config = ResolvedLogging {
        enabled: true,
        level: "nomi=invalid-level".into(),
        dir: temp.path().join("not-created"),
    };
    assert!(create_file_layer::<tracing_subscriber::Registry>(&config).is_err());
    assert!(!config.dir.exists());
}

#[test]
fn create_file_layer_writes_json_to_file() {
    let tmp = tempfile::tempdir().unwrap();
    let config = ResolvedLogging {
        enabled: true,
        level: "info".to_string(),
        dir: tmp.path().to_path_buf(),
    };

    let (layer, _guard) = create_file_layer(&config).unwrap();

    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        info!(target: "nomi_test", key = "value", "test message");
    });

    drop(_guard);

    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .collect();
    assert!(!entries.is_empty(), "expected at least one .log file");

    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    assert!(
        content.contains("test message"),
        "log should contain message"
    );
    assert!(content.contains("nomi_test"), "log should contain target");
}

#[test]
fn create_file_layer_creates_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("sub").join("dir");
    let config = ResolvedLogging {
        enabled: true,
        level: "info".to_string(),
        dir: nested.clone(),
    };

    let result = create_file_layer::<tracing_subscriber::Registry>(&config);
    assert!(result.is_ok());
    assert!(nested.exists());
}
