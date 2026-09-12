use super::*;
use std::sync::Barrier;

#[test]
fn concurrent_materializers_publish_one_complete_host_file() {
    let temp = TempDir::new().unwrap();
    for round in 0..4 {
        let directory = temp.path().join(round.to_string());
        let barrier = Barrier::new(16);
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        materialize_bundled_extension_host(&directory)
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(
            results.iter().all(Result::is_ok),
            "concurrent publication failed: {results:?}"
        );
        let first = results[0].as_ref().unwrap();
        assert!(
            results
                .iter()
                .all(|result| result.as_ref().unwrap() == first)
        );
        assert_eq!(
            std::fs::read(first).unwrap(),
            include_bytes!("../../assets/extension-host.mjs")
        );
        assert_eq!(
            std::fs::read_dir(directory).unwrap().count(),
            1,
            "temporary files leaked"
        );
    }
}

#[test]
fn existing_corrupt_host_file_is_rejected_without_overwriting_it() {
    let temp = TempDir::new().unwrap();
    let file = materialize_bundled_extension_host(temp.path()).unwrap();
    std::fs::write(&file, b"foreign content").unwrap();
    assert!(matches!(
        materialize_bundled_extension_host(temp.path()),
        Err(JavaScriptHostError::InvalidConfiguration(_))
    ));
    assert_eq!(std::fs::read(file).unwrap(), b"foreign content");
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn directory_at_host_digest_path_is_rejected_without_replacement() {
    let temp = TempDir::new().unwrap();
    let file = materialize_bundled_extension_host(temp.path()).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(matches!(
        materialize_bundled_extension_host(temp.path()),
        Err(JavaScriptHostError::InvalidConfiguration(_))
    ));
    assert!(file.is_dir());
}

#[tokio::test]
async fn invalid_plugin_entrypoints_are_rejected_before_starting_a_generation() {
    let host = supervisor(Duration::from_secs(3)).await;
    let temp = TempDir::new().unwrap();
    let context = context(temp.path(), "mount-a", 'a');
    let valid = module(context.target.clone()).await;
    let mut wrong_target = valid.clone();
    wrong_target.target = target("mount-b", 'b');
    let mut corrupt = valid.clone();
    corrupt.module_digest = DigestHex::from("0".repeat(64));
    let mut relative = valid.clone();
    relative.main_mjs = PathBuf::from("main.mjs");
    let mut wrong_name = valid.clone();
    wrong_name.main_mjs = fixture("protocol-host.mjs").canonicalize().unwrap();
    let mut missing = valid.clone();
    missing.main_mjs = temp.path().join("missing/main.mjs");
    for module in [wrong_target, corrupt, relative, wrong_name, missing] {
        assert!(
            host.load_mount(MountLoadDemand {
                context: context.clone(),
                module
            })
            .await
            .is_err()
        );
        assert_eq!(host.state(), JavaScriptHostState::Stopped);
    }
    let generation = host
        .load_mount(MountLoadDemand {
            context,
            module: valid,
        })
        .await
        .unwrap();
    assert_eq!(generation, 1);
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn materialized_host_executes_the_real_handshake_and_mount_lifecycle() {
    let temp = TempDir::new().unwrap();
    let module = materialize_bundled_extension_host(temp.path().join("host")).unwrap();
    let host = supervisor_with_host(Duration::from_secs(3), module).await;
    let generation = load(&host, &temp, "mount-a", 'a').await;
    let value = host
        .invoke(
            contribution(target("mount-a", 'a')),
            ActionId::from("echo"),
            StrictJsonValue(json!({"published": true})),
        )
        .await
        .unwrap();
    host.stop_generation(generation).await.unwrap();
    assert_eq!(value.0["input"]["published"], true);
}
