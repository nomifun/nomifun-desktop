//! Safe retirement of one empty field in current application configuration.
//! This reader never reads, converts or deletes Agent Session data.
use std::fs;
use nomifun_app_hosting::{AppMetadata, HostingOwner, LocalConnection};

fn connection() -> LocalConnection {
    LocalConnection { host_url: "http://127.0.0.1:5177".into(), local_trust: "registry-retirement-test".into() }
}

#[tokio::test]
async fn empty_retired_reference_is_read_once_and_never_written_back() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("hosting");
    let owner = HostingOwner::new(root.clone(), connection()).await.unwrap();
    let app = owner.create("用户作品".into(), AppMetadata { description: "保留说明".into(), tags: vec!["工具".into()] }).await.unwrap();
    fs::write(app.directory.join("work.txt"), "USER_WORK").unwrap();
    owner.shutdown().await.unwrap();
    drop(owner);
    let path = root.join("registry").join(format!("{}.json", app.id));
    let mut stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["record"]["settings"]["exclusive_session_ids"] = serde_json::json!([]);
    fs::write(&path, serde_json::to_vec_pretty(&stored).unwrap()).unwrap();
    let owner = HostingOwner::new(root, connection()).await.unwrap();
    let loaded = owner.get(&app.id).await.unwrap();
    assert_eq!(loaded.description, app.description);
    assert_eq!(loaded.tags, app.tags);
    assert_eq!(fs::read_to_string(app.directory.join("work.txt")).unwrap(), "USER_WORK");
    let rewritten: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(rewritten["record"]["settings"].get("exclusive_session_ids").is_none());
    assert!(serde_json::from_value::<nomifun_app_hosting::AppSettings>(serde_json::json!({"exclusive_session_ids":[]})).is_err(),
        "the public settings contract must reject even empty retired ownership claims");
}

#[tokio::test]
async fn nonempty_or_invalid_retired_references_fail_closed_without_touching_files() {
    for retired in [serde_json::json!([uuid::Uuid::now_v7().to_string()]), serde_json::Value::Null, serde_json::json!("unknown")] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("hosting");
        let owner = HostingOwner::new(root.clone(), connection()).await.unwrap();
        let app = owner.create("必须保留".into(), AppMetadata::default()).await.unwrap();
        fs::write(app.directory.join("work.txt"), "USER_WORK").unwrap();
        owner.shutdown().await.unwrap();
        drop(owner);
        let path = root.join("registry").join(format!("{}.json", app.id));
        let mut stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["record"]["settings"]["exclusive_session_ids"] = retired;
        let before = serde_json::to_vec_pretty(&stored).unwrap();
        fs::write(&path, &before).unwrap();
        let error = match HostingOwner::new(root, connection()).await { Ok(_) => panic!("unknown ownership must be rejected"), Err(error) => error.to_string() };
        assert!(error.contains("exclusive_session_ids") && error.contains("canonical Session history are preserved"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_to_string(app.directory.join("work.txt")).unwrap(), "USER_WORK");
    }
}
