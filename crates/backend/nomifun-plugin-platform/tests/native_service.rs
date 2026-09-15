#[path = "support/native.rs"]
mod support;
use nomifun_agent_contracts::*;
use nomifun_plugin_platform::runtime::*;
use serde_json::json;
use std::time::Duration;
use support::*;

#[tokio::test]
async fn native_is_default_denied_and_target_checked_without_node() {
    let root = tempfile::tempdir().unwrap();
    let input = spec_input(materialization(b"not executed".to_vec()).descriptor);
    assert!(
        binding(root.path(), false)
            .resolve_spec(input.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("opt-in")
    );
    let runtime = binding(root.path(), true);
    let spec = runtime.resolve_spec(input.clone()).await.unwrap();
    assert_eq!(
        runtime
            .current_fingerprint_for(&spec.runtime)
            .await
            .unwrap(),
        Some(spec.runtime)
    );
    let mut other = input;
    let target = if NativePluginTarget::current() == Some(NativePluginTarget::WindowsX64) {
        NativePluginTarget::LinuxX64
    } else {
        NativePluginTarget::WindowsX64
    };
    other.descriptor.execution = PluginServiceExecution::Native { target };
    other.descriptor.entrypoint = target.entrypoint().into();
    assert!(
        runtime
            .resolve_spec(other)
            .await
            .unwrap_err()
            .to_string()
            .contains("matching target")
    );
}

#[tokio::test]
#[ignore = "requires compiled Rust SDK echo executable; see native plugin developer guide"]
async fn native_process_echo_stream_storage_cancel_crash_and_integrity() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let root = tempfile::tempdir().unwrap();
        let materialized = materialization(executable());
        let path = root
            .path()
            .join(&materialized.file.normalized_relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &materialized.file.bytes).unwrap();
        let runtime = binding(root.path(), true);
        let mut input = spec_input(materialized.descriptor);
        input.storage = runtime
            .resolve_storage("owner", &input.plugin_product_id, false, false)
            .await
            .unwrap()
            .descriptor;
        let spec = runtime.resolve_spec(input).await.unwrap();
        runtime
            .register_module(
                spec.plugin_product_id.clone(),
                spec.release.release_digest.clone(),
                path.clone(),
            )
            .await
            .unwrap();
        runtime.start(spec.clone()).await.unwrap();
        let result = runtime
            .invoke(
                &spec,
                "echo-1".into(),
                "echo".into(),
                StrictJsonValue(json!({"hello":"rust"})),
                Default::default(),
                1,
            )
            .await
            .unwrap();
        assert_eq!(result.0["payload"], json!({"hello":"rust"}));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let stream_runtime = runtime.clone();
        let stream_spec = spec.clone();
        let task = tokio::spawn(async move {
            stream_runtime
                .invoke_with_events(
                    &stream_spec,
                    "stream-1".into(),
                    "stream".into(),
                    StrictJsonValue(json!({})),
                    Default::default(),
                    2,
                    Some(sender),
                )
                .await
        });
        for sequence in 1..=3 {
            assert_eq!(
                receiver.recv().await.unwrap().0,
                json!({"sequence":sequence})
            );
        }
        assert_eq!(task.await.unwrap().unwrap().0, json!(null));
        let stored = runtime
            .invoke(
                &spec,
                "kv-1".into(),
                "kv".into(),
                StrictJsonValue(json!({"operation":"set","key":"rust","value":42})),
                Default::default(),
                3,
            )
            .await
            .unwrap();
        assert!(!stored.0.is_null());
        let fetched = runtime
            .invoke(
                &spec,
                "kv-2".into(),
                "kv".into(),
                StrictJsonValue(json!({"operation":"get","key":"rust"})),
                Default::default(),
                4,
            )
            .await
            .unwrap();
        assert_eq!(fetched.0["value"], json!(42));
        let cancellation = PluginRuntimeCallCancellation::default();
        let child_cancel = cancellation.clone();
        let child_runtime = runtime.clone();
        let child_spec = spec.clone();
        let hanging = tokio::spawn(async move {
            child_runtime
                .invoke(
                    &child_spec,
                    "hang-1".into(),
                    "hang".into(),
                    StrictJsonValue(json!({})),
                    child_cancel,
                    5,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation.cancel();
        assert!(hanging.await.unwrap().is_err());
        assert!(
            runtime
                .invoke(
                    &spec,
                    "after-cancel".into(),
                    "echo".into(),
                    StrictJsonValue(json!({})),
                    Default::default(),
                    6
                )
                .await
                .is_ok()
        );
        assert!(
            runtime
                .invoke(
                    &spec,
                    "crash-1".into(),
                    "crash".into(),
                    StrictJsonValue(json!({})),
                    Default::default(),
                    7
                )
                .await
                .is_err()
        );
        runtime.stop(&spec.plugin_product_id).await.unwrap();
        // Hash is rechecked at each launch, not just at import/registration.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(&path, b"tampered executable").unwrap();
        assert!(runtime.start(spec).await.is_err());
    })
    .await
    .expect("native lifecycle test timed out");
}
