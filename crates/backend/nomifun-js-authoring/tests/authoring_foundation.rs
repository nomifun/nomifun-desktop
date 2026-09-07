use std::fs;
use std::path::{Path, PathBuf};

use nomifun_js_authoring::{
    AuthoringError, DependencyRequestSet, ExactDependencyLock, LockedNpmPackage,
    NeverCancel, NormalizedSourcePath, NpmResolverIdentity, OperationCancellation,
    PluginLanguage, PluginProjectId, PluginScaffoldRequest, SourceFileDigest,
    SourceScope, SourceSnapshot, SourceStore, SourceStoreLimits, UserId,
    digest_bytes,
};
use tempfile::TempDir;
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn scope() -> SourceScope {
    SourceScope::new(
        UserId::from(id()),
        PluginProjectId::from(id()),
    )
    .unwrap()
}

fn scaffold(language: PluginLanguage) -> PluginScaffoldRequest {
    PluginScaffoldRequest {
        package_id: "example.csv-tools".into(),
        package_version: "0.1.0".into(),
        display_name: "CSV Tools".into(),
        description: "Read and transform CSV data.".into(),
        language,
    }
}

fn fixture() -> (TempDir, SourceStore) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        SourceStore::new(temp.path().join("managed"), SourceStoreLimits::default()).unwrap();
    (temp, store)
}

#[test]
fn scaffold_is_owner_project_scoped_and_reproducible() {
    let (_temp, store) = fixture();
    let first_scope = scope();
    let second_scope = scope();
    let first = store
        .create_plugin_project(
            first_scope.clone(),
            &scaffold(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    let second = store
        .create_plugin_project(
            second_scope.clone(),
            &scaffold(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();

    assert!(first.project().managed_relative_path().contains(&format!(
        "sources/{}/projects/{}/source",
        first_scope.owner_id().as_ref(),
        first_scope.project_id().as_ref()
    )));
    assert_ne!(
        first.project().source_root(),
        second.project().source_root()
    );
    assert_eq!(
        first.capture().snapshot().digest(),
        second.capture().snapshot().digest()
    );
    assert!(first.project().source_root().join("src/main.js").is_file());
    assert_eq!(
        first.capture().dependency_requests(),
        &DependencyRequestSet::empty()
    );

    let package_json =
        fs::read_to_string(first.project().source_root().join("package.json")).unwrap();
    assert!(!package_json.contains("scripts"));
    assert!(!package_json.contains("node_modules"));
}

#[test]
fn javascript_and_typescript_scaffolds_are_distinct_fixed_inputs() {
    let (_temp, store) = fixture();
    let javascript = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();
    let typescript = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::TypeScript), &NeverCancel)
        .unwrap();
    assert_ne!(
        javascript.capture().snapshot().digest(),
        typescript.capture().snapshot().digest()
    );
    assert!(
        typescript
            .project()
            .source_root()
            .join("src/main.ts")
            .is_file()
    );
    assert!(
        fs::read_to_string(typescript.project().source_root().join("src/main.ts"))
            .unwrap()
            .contains("ActivationContext")
    );
}

#[test]
fn canonical_snapshot_ignores_creation_order_but_not_bytes() {
    let (_temp, store) = fixture();
    let first = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();
    let second = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();

    fs::create_dir_all(first.project().source_root().join("assets")).unwrap();
    fs::write(first.project().source_root().join("assets/a.txt"), b"alpha").unwrap();
    fs::write(first.project().source_root().join("assets/b.txt"), b"beta").unwrap();

    fs::create_dir_all(second.project().source_root().join("assets")).unwrap();
    fs::write(second.project().source_root().join("assets/b.txt"), b"beta").unwrap();
    fs::write(
        second.project().source_root().join("assets/a.txt"),
        b"alpha",
    )
    .unwrap();

    let first_snapshot = store
        .snapshot(first.project().scope(), &NeverCancel)
        .unwrap();
    let second_snapshot = store
        .snapshot(second.project().scope(), &NeverCancel)
        .unwrap();
    assert_eq!(
        first_snapshot.snapshot().digest(),
        second_snapshot.snapshot().digest()
    );

    fs::write(
        second.project().source_root().join("assets/a.txt"),
        b"changed",
    )
    .unwrap();
    let changed = store
        .snapshot(second.project().scope(), &NeverCancel)
        .unwrap();
    assert_ne!(
        first_snapshot.snapshot().digest(),
        changed.snapshot().digest()
    );
}

#[test]
fn snapshot_contract_rejects_case_collisions() {
    let digest = digest_bytes(b"x");
    let result = SourceSnapshot::from_files(vec![
        SourceFileDigest::new(
            NormalizedSourcePath::parse("Src/Main.ts").unwrap(),
            digest.clone(),
            1,
        ),
        SourceFileDigest::new(
            NormalizedSourcePath::parse("src/main.ts").unwrap(),
            digest,
            1,
        ),
    ]);
    assert!(matches!(result, Err(AuthoringError::PathCollision { .. })));
}

#[test]
fn source_capture_rejects_runtime_dependencies_native_addons_and_user_locks() {
    for relative in [
        "node_modules/dependency/index.js",
        "native/addon.node",
        "package-lock.json",
        ".npmrc",
    ] {
        let (_temp, store) = fixture();
        let project = store
            .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
            .unwrap();
        let path = project.project().source_root().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
        assert!(
            matches!(
                store.snapshot(project.project().scope(), &NeverCancel),
                Err(AuthoringError::ForbiddenSourceEntry { .. })
            ),
            "{relative}"
        );
    }
}

#[test]
fn fixed_package_json_rejects_scripts_protocols_and_duplicate_dependencies() {
    for package_json in [
        r#"{"private":true,"type":"module","scripts":{"build":"evil"},"dependencies":{}}"#,
        r#"{"private":true,"type":"module","dependencies":{"left-pad":"git+https://example.invalid/repo"}}"#,
        r#"{"private":true,"type":"module","dependencies":{"left-pad":"1.3.0","left-pad":"1.3.1"}}"#,
    ] {
        assert!(DependencyRequestSet::from_package_json(package_json.as_bytes()).is_err());
    }

    let requests = DependencyRequestSet::from_package_json(
        br#"{"private":true,"type":"module","dependencies":{"@scope/core":"^2.0.0","left-pad":"1.3.0"}}"#,
    )
    .unwrap();
    assert_eq!(requests.dependencies().len(), 2);
}

#[test]
fn exact_dependency_lock_binds_request_and_canonical_graph() {
    let requests = DependencyRequestSet::new([
        ("alpha".to_owned(), "^1.0.0".to_owned()),
        ("beta".to_owned(), "2.0.0".to_owned()),
    ])
    .unwrap();
    let alpha = locked_package(
        "alpha",
        "1.2.0",
        [("shared".to_owned(), "shared@3.0.0".to_owned())],
    );
    let beta = locked_package("beta", "2.0.0", []);
    let shared = locked_package("shared", "3.0.0", []);
    let resolver = NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap();

    let lock = ExactDependencyLock::new(
        requests.digest().unwrap(),
        resolver.clone(),
        [
            ("beta".to_owned(), "beta@2.0.0".to_owned()),
            ("alpha".to_owned(), "alpha@1.2.0".to_owned()),
        ],
        [
            (shared.key(), shared.clone()),
            (alpha.key(), alpha.clone()),
            (beta.key(), beta.clone()),
        ],
    )
    .unwrap();
    lock.validate_against(&requests).unwrap();

    let reordered = ExactDependencyLock::new(
        requests.digest().unwrap(),
        resolver,
        [
            ("alpha".to_owned(), "alpha@1.2.0".to_owned()),
            ("beta".to_owned(), "beta@2.0.0".to_owned()),
        ],
        [
            (beta.key(), beta),
            (alpha.key(), alpha),
            (shared.key(), shared),
        ],
    )
    .unwrap();
    assert_eq!(lock.digest().unwrap(), reordered.digest().unwrap());

    let other_requests =
        DependencyRequestSet::new([("alpha".to_owned(), "^2.0.0".to_owned())]).unwrap();
    assert!(lock.validate_against(&other_requests).is_err());
}

#[test]
fn exact_lock_rejects_missing_and_unreachable_packages() {
    let requests = DependencyRequestSet::new([("alpha".to_owned(), "^1.0.0".to_owned())]).unwrap();
    let alpha = locked_package("alpha", "1.0.0", []);
    let unused = locked_package("unused", "1.0.0", []);
    let resolver = NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap();
    let missing = ExactDependencyLock::new(
        requests.digest().unwrap(),
        resolver.clone(),
        [("alpha".to_owned(), "missing@1.0.0".to_owned())],
        [(alpha.key(), alpha.clone())],
    );
    assert!(missing.is_err());

    let unreachable = ExactDependencyLock::new(
        requests.digest().unwrap(),
        resolver,
        [("alpha".to_owned(), alpha.key())],
        [(alpha.key(), alpha), (unused.key(), unused)],
    );
    assert!(unreachable.is_err());
}

#[test]
fn exact_lock_rejects_resolution_outside_the_direct_request() {
    let requests = DependencyRequestSet::new([("alpha".to_owned(), "^1.0.0".to_owned())]).unwrap();
    let alpha = locked_package("alpha", "2.0.0", []);
    let lock = ExactDependencyLock::new(
        requests.digest().unwrap(),
        NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap(),
        [("alpha".to_owned(), alpha.key())],
        [(alpha.key(), alpha)],
    )
    .unwrap();
    assert!(lock.validate_against(&requests).is_err());

    let exact_requests =
        DependencyRequestSet::new([("alpha".to_owned(), "1.0.0".to_owned())]).unwrap();
    let newer_alpha = locked_package("alpha", "1.0.1", []);
    let exact_lock = ExactDependencyLock::new(
        exact_requests.digest().unwrap(),
        NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap(),
        [("alpha".to_owned(), newer_alpha.key())],
        [(newer_alpha.key(), newer_alpha)],
    )
    .unwrap();
    assert!(exact_lock.validate_against(&exact_requests).is_err());
}

#[test]
fn dependency_contract_constructors_reject_duplicate_entries() {
    assert!(
        DependencyRequestSet::new([
            ("alpha".to_owned(), "^1.0.0".to_owned()),
            ("alpha".to_owned(), "^2.0.0".to_owned()),
        ])
        .is_err()
    );
    assert!(
        LockedNpmPackage::new(
            "alpha",
            "1.0.0",
            "sha512-YWJjZA==",
            digest_bytes(b"archive"),
            digest_bytes(b"package-json"),
            [
                ("shared".to_owned(), "shared@1.0.0".to_owned()),
                ("shared".to_owned(), "shared@2.0.0".to_owned()),
            ],
        )
        .is_err()
    );

    let requests = DependencyRequestSet::new([("alpha".to_owned(), "^1.0.0".to_owned())]).unwrap();
    let alpha = locked_package("alpha", "1.0.0", []);
    assert!(
        ExactDependencyLock::new(
            requests.digest().unwrap(),
            NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap(),
            [
                ("alpha".to_owned(), alpha.key()),
                ("alpha".to_owned(), alpha.key()),
            ],
            [(alpha.key(), alpha)],
        )
        .is_err()
    );
}

#[test]
fn build_staging_is_exact_and_drop_cleans_it() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::TypeScript), &NeverCancel)
        .unwrap();
    let expected = project.capture().snapshot().clone();
    let operation_root;
    {
        let staged = store
            .stage_snapshot(project.project().scope(), &expected, &NeverCancel)
            .unwrap();
        operation_root = staged.operation_root().to_path_buf();
        assert!(staged.source_root().join("src/main.ts").is_file());
        assert!(staged.output_root().is_dir());
        assert_eq!(staged.capture().snapshot(), &expected);
        assert!(!staged.operation_root().join("node_modules").exists());
    }
    assert!(!operation_root.exists());
    assert_eq!(staging_count(store.managed_root()), 0);
}

struct CancelWhenStagingExists {
    staging_root: PathBuf,
}

impl OperationCancellation for CancelWhenStagingExists {
    fn is_cancelled(&self) -> bool {
        fs::read_dir(&self.staging_root)
            .map(|entries| entries.count() > 0)
            .unwrap_or(false)
    }
}

#[test]
fn dependency_lock_is_host_owned_and_project_delete_is_explicit() {
    let (_temp, store) = fixture();
    let scope = scope();
    let project = store
        .create_plugin_project(
            scope.clone(),
            &scaffold(PluginLanguage::JavaScript),
            &NeverCancel,
        )
        .unwrap();
    let requests = project.capture().dependency_requests();
    let lock = ExactDependencyLock::empty(
        requests,
        NpmResolverIdentity::new("nomifun-npm", "1.0.0").unwrap(),
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
    assert_eq!(lock_digest, lock.digest().unwrap());
    assert_eq!(
        store
            .load_dependency_lock(&scope, &NeverCancel)
            .unwrap()
            .digest()
            .unwrap(),
        lock_digest
    );
    store.delete_project(&scope).unwrap();
    assert!(matches!(
        store.load_project(&scope),
        Err(AuthoringError::ProjectNotFound)
    ));
}

#[test]
fn cancellation_cleans_create_and_build_staging() {
    let (_temp, store) = fixture();
    let cancellation = CancelWhenStagingExists {
        staging_root: store.managed_root().join(".staging"),
    };
    assert!(matches!(
        store.create_plugin_project(
            scope(),
            &scaffold(PluginLanguage::JavaScript),
            &cancellation,
        ),
        Err(AuthoringError::Canceled)
    ));
    assert_eq!(staging_count(store.managed_root()), 0);

    let project = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();
    assert!(matches!(
        store.stage_snapshot(
            project.project().scope(),
            project.capture().snapshot(),
            &cancellation,
        ),
        Err(AuthoringError::Canceled)
    ));
    assert_eq!(staging_count(store.managed_root()), 0);
}

#[test]
fn scope_record_tamper_is_rejected() {
    let (_temp, store) = fixture();
    let project = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();
    fs::write(
        project.project().project_root().join("scope.json"),
        br#"{"format_version":"1.0.0","scope":{"owner_id":"bad","project_id":"bad"}}"#,
    )
    .unwrap();
    assert!(store.load_project(project.project().scope()).is_err());
}

#[test]
fn source_symlinks_are_rejected_when_supported_by_the_host() {
    let temp = tempfile::tempdir().unwrap();
    let (_managed, store) = {
        let managed = temp.path().join("managed");
        let store = SourceStore::new(&managed, SourceStoreLimits::default()).unwrap();
        (managed, store)
    };
    let project = store
        .create_plugin_project(scope(), &scaffold(PluginLanguage::JavaScript), &NeverCancel)
        .unwrap();
    let outside = temp.path().join("outside.js");
    fs::write(&outside, b"outside").unwrap();
    let link = project.project().source_root().join("src/link.js");
    if !create_file_symlink(&outside, &link) {
        return;
    }
    assert!(matches!(
        store.snapshot(project.project().scope(), &NeverCancel),
        Err(AuthoringError::UnsafeSourcePath { .. })
    ));
}

fn locked_package<const N: usize>(
    name: &str,
    version: &str,
    dependencies: [(String, String); N],
) -> LockedNpmPackage {
    LockedNpmPackage::new(
        name,
        version,
        "sha512-YWJjZA==",
        digest_bytes(format!("{name}-{version}-archive").as_bytes()),
        digest_bytes(format!("{name}-{version}-package-json").as_bytes()),
        dependencies,
    )
    .unwrap()
}

fn staging_count(managed_root: &Path) -> usize {
    fs::read_dir(managed_root.join(".staging")).unwrap().count()
}

#[cfg(unix)]
fn create_file_symlink(source: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(source, target).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(source: &Path, target: &Path) -> bool {
    std::os::windows::fs::symlink_file(source, target).is_ok()
}
