use std::collections::BTreeSet;
use std::fs;

use nomifun_js_authoring::{
    DependencyRequestSet, DigestHex, ExactDependencyLock, LockedNpmPackage,
    NeverCancel, NpmResolverIdentity, PluginLanguage, PluginProjectId,
    PluginScaffoldRequest, SourceScope, SourceStore, SourceStoreLimits, UserId,
};
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn fixture() -> (
    tempfile::TempDir,
    SourceStore,
    SourceScope,
    nomifun_js_authoring::CapturedSource,
    DigestHex,
) {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::new(
        temp.path().join("managed"),
        SourceStoreLimits::default(),
    )
    .unwrap();
    let scope = SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();
    let project = store
        .create_plugin_project(
            scope.clone(),
            &PluginScaffoldRequest {
                package_id: "example.dependencies".into(),
                package_version: "1.0.0".into(),
                display_name: "Dependencies".into(),
                description: "Dependency mutation fixture.".into(),
                language: PluginLanguage::JavaScript,
            },
            &NeverCancel,
        )
        .unwrap();
    let lock = ExactDependencyLock::empty(
        project.capture().dependency_requests(),
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
    )
    .unwrap();
    let lock_digest = store
        .write_initial_dependency_lock(
            &scope,
            project.capture().snapshot(),
            &lock,
            &NeverCancel,
        )
        .unwrap();
    (
        temp,
        store,
        scope,
        project.capture().clone(),
        lock_digest,
    )
}

fn next_lock() -> (DependencyRequestSet, ExactDependencyLock) {
    let requests =
        DependencyRequestSet::new([("alpha".into(), "^1.0.0".into())]).unwrap();
    let package = LockedNpmPackage::new(
        "alpha",
        "1.2.0",
        "sha512-YWJjZA==",
        DigestHex::from("a".repeat(64)),
        DigestHex::from("b".repeat(64)),
        [],
    )
    .unwrap();
    let lock = ExactDependencyLock::new(
        requests.digest().unwrap(),
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
        [("alpha".into(), "alpha@1.2.0".into())],
        [("alpha@1.2.0".into(), package)],
    )
    .unwrap();
    (requests, lock)
}

#[test]
fn unpersisted_dependency_prepare_is_cleaned_without_touching_live_source() {
    let (_temp, store, scope, current, current_lock_digest) = fixture();
    let (requests, lock) = next_lock();
    let prepared = store
        .prepare_dependency_mutation(
            &scope,
            &id(),
            7,
            3,
            current.snapshot().digest(),
            &current_lock_digest,
            &requests,
            &lock,
            &NeverCancel,
        )
        .unwrap();
    assert!(!prepared.facts().is_noop());
    drop(prepared);

    assert_eq!(
        store
            .snapshot(&scope, &NeverCancel)
            .unwrap()
            .snapshot(),
        current.snapshot()
    );
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn unchanged_dependency_map_preserves_package_json_bytes_as_a_noop() {
    let (_temp, store, scope, _current, current_lock_digest) = fixture();
    let package_json = store
        .managed_root()
        .join("sources")
        .join(scope.owner_id().as_ref())
        .join("projects")
        .join(scope.project_id().as_ref())
        .join("source")
        .join("package.json");
    let parsed: serde_json::Value =
        serde_json::from_slice(&fs::read(&package_json).unwrap()).unwrap();
    let noncanonical = serde_json::to_vec_pretty(&parsed).unwrap();
    fs::write(&package_json, &noncanonical).unwrap();
    let current = store.snapshot(&scope, &NeverCancel).unwrap();
    let lock = store
        .load_dependency_lock(&scope, &NeverCancel)
        .unwrap();

    let prepared = store
        .prepare_dependency_mutation(
            &scope,
            &id(),
            7,
            3,
            current.snapshot().digest(),
            &current_lock_digest,
            current.dependency_requests(),
            &lock,
            &NeverCancel,
        )
        .unwrap();
    assert!(prepared.facts().is_noop());
    drop(prepared);
    assert_eq!(fs::read(package_json).unwrap(), noncanonical);
}

#[test]
fn durable_dependency_commit_can_rollback_to_the_exact_previous_pair() {
    let (_temp, store, scope, current, current_lock_digest) = fixture();
    let (requests, lock) = next_lock();
    let durable = store
        .prepare_dependency_mutation(
            &scope,
            &id(),
            7,
            3,
            current.snapshot().digest(),
            &current_lock_digest,
            &requests,
            &lock,
            &NeverCancel,
        )
        .unwrap()
        .persist();
    let facts = durable.facts().clone();

    store.commit_dependency_mutation(&durable).unwrap();
    assert_eq!(store.list_dependency_mutation_journals().unwrap(), vec![facts.clone()]);
    assert!(store.load_project(&scope).is_err());
    store.rollback_dependency_mutation(&facts).unwrap();
    assert!(store.list_dependency_mutation_journals().unwrap().is_empty());

    assert_eq!(
        store
            .snapshot(&scope, &NeverCancel)
            .unwrap()
            .snapshot(),
        current.snapshot()
    );
    assert_eq!(
        store
            .load_dependency_lock(&scope, &NeverCancel)
            .unwrap()
            .digest()
            .unwrap(),
        current_lock_digest
    );
}

#[test]
fn finalized_dependency_commit_exposes_only_the_new_coherent_pair() {
    let (_temp, store, scope, current, current_lock_digest) = fixture();
    let (requests, lock) = next_lock();
    let durable = store
        .prepare_dependency_mutation(
            &scope,
            &id(),
            7,
            3,
            current.snapshot().digest(),
            &current_lock_digest,
            &requests,
            &lock,
            &NeverCancel,
        )
        .unwrap()
        .persist();
    let facts = durable.facts().clone();

    store.commit_dependency_mutation(&durable).unwrap();
    store.finish_dependency_mutation(&facts).unwrap();
    assert!(store.list_dependency_mutation_journals().unwrap().is_empty());

    let capture = store.snapshot(&scope, &NeverCancel).unwrap();
    assert_eq!(capture.snapshot().digest(), facts.next_source_digest());
    assert_eq!(capture.dependency_requests(), &requests);
    assert_eq!(
        store
            .load_dependency_lock(&scope, &NeverCancel)
            .unwrap()
            .digest()
            .unwrap(),
        *facts.next_lock_digest()
    );
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn startup_cleanup_removes_only_unretained_dependency_staging() {
    let (_temp, store, scope, current, current_lock_digest) = fixture();
    let (requests, lock) = next_lock();
    let mutation_id = id();
    let durable = store
        .prepare_dependency_mutation(
            &scope,
            &mutation_id,
            7,
            3,
            current.snapshot().digest(),
            &current_lock_digest,
            &requests,
            &lock,
            &NeverCancel,
        )
        .unwrap()
        .persist();
    drop(durable);

    store
        .cleanup_orphan_dependency_staging(&BTreeSet::from([mutation_id.clone()]))
        .unwrap();
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        1
    );
    store
        .cleanup_orphan_dependency_staging(&BTreeSet::new())
        .unwrap();
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn restart_rollback_recovers_partial_dependency_file_swaps() {
    for completed_moves in [1, 3] {
        let (_temp, store, scope, current, current_lock_digest) = fixture();
        let (requests, lock) = next_lock();
        let mutation_id = id();
        let durable = store
            .prepare_dependency_mutation(
                &scope,
                &mutation_id,
                7,
                3,
                current.snapshot().digest(),
                &current_lock_digest,
                &requests,
                &lock,
                &NeverCancel,
            )
            .unwrap()
            .persist();
        let facts = durable.facts().clone();
        let managed_root = store.managed_root().to_path_buf();
        let operation_root = managed_root
            .join(".staging")
            .join(format!("dependency-{mutation_id}"));
        let project_root = managed_root
            .join("sources")
            .join(scope.owner_id().as_ref())
            .join("projects")
            .join(scope.project_id().as_ref());
        let source_root = project_root.join("source");

        fs::copy(
            operation_root.join("mutation.json"),
            project_root.join(".dependency-mutation.json"),
        )
        .unwrap();
        fs::rename(
            source_root.join("package.json"),
            operation_root.join("previous-package.json"),
        )
        .unwrap();
        if completed_moves == 3 {
            fs::rename(
                project_root.join("dependency-lock.json"),
                operation_root.join("previous-dependency-lock.json"),
            )
            .unwrap();
            fs::rename(
                operation_root.join("source").join("package.json"),
                source_root.join("package.json"),
            )
            .unwrap();
        }
        drop(durable);
        drop(store);

        let restarted = SourceStore::new(&managed_root, SourceStoreLimits::default()).unwrap();
        assert_eq!(
            restarted.dependency_mutation_journal(&scope).unwrap(),
            Some(facts.clone())
        );
        restarted.rollback_dependency_mutation(&facts).unwrap();
        assert_eq!(
            restarted
                .snapshot(&scope, &NeverCancel)
                .unwrap()
                .snapshot(),
            current.snapshot()
        );
        assert_eq!(
            restarted
                .load_dependency_lock(&scope, &NeverCancel)
                .unwrap()
                .digest()
                .unwrap(),
            current_lock_digest
        );
        assert!(restarted.list_dependency_mutation_journals().unwrap().is_empty());
    }
}
