use nomifun_js_authoring::{
    ExactDependencyLock, NeverCancel, NpmResolverIdentity, PluginLanguage,
    PluginProjectId, PluginScaffoldRequest, PluginSourceArchive, PluginSourceFileBytes,
    SourceScope, SourceStore, SourceStoreLimits, UserId,
};
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn fixture() -> (tempfile::TempDir, SourceStore, SourceScope) {
    let temp = tempfile::tempdir().unwrap();
    let store = SourceStore::new(temp.path().join("managed"), SourceStoreLimits::default())
        .unwrap();
    let scope = SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();
    let project = store
        .create_plugin_project(
            scope.clone(),
            &PluginScaffoldRequest {
                package_id: "example.share-source".into(),
                package_version: "1.0.0".into(),
                display_name: "Share Source".into(),
                description: "Source Share fixture.".into(),
                language: PluginLanguage::TypeScript,
            },
            &NeverCancel,
        )
        .unwrap();
    let lock = ExactDependencyLock::empty(
        project.capture().dependency_requests(),
        NpmResolverIdentity::new("test", "1.0.0").unwrap(),
    )
    .unwrap();
    store
        .write_initial_dependency_lock(
            &scope,
            project.capture().snapshot(),
            &lock,
            &NeverCancel,
        )
        .unwrap();
    (temp, store, scope)
}

#[test]
fn exported_source_imports_as_a_new_exact_owner_project() {
    let (_temp, store, source_scope) = fixture();
    let archive = store
        .export_project_source(&source_scope, &NeverCancel)
        .unwrap();
    let target_scope =
        SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();
    let imported = store
        .import_project_source(target_scope.clone(), &archive, &NeverCancel)
        .unwrap();

    assert_eq!(imported.capture().snapshot(), archive.snapshot());
    assert_eq!(
        imported.dependency_lock_digest(),
        &archive.dependency_lock().digest().unwrap()
    );
    assert_eq!(imported.project().scope(), &target_scope);
    assert_eq!(
        store
            .export_project_source(&target_scope, &NeverCancel)
            .unwrap(),
        archive
    );
}

#[test]
fn imported_source_rejects_tampered_bytes_without_publishing_project() {
    let (_temp, store, source_scope) = fixture();
    let archive = store
        .export_project_source(&source_scope, &NeverCancel)
        .unwrap();
    let mut files = archive.files().to_vec();
    let first = files.remove(0);
    let mut bytes = first.bytes().to_vec();
    bytes.push(b' ');
    files.push(PluginSourceFileBytes::new(
        first.normalized_relative_path().clone(),
        bytes,
    ));
    let tampered = PluginSourceArchive::new(
        archive.snapshot().clone(),
        archive.dependency_lock().clone(),
        files,
    );
    let target_scope =
        SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap();

    assert!(
        store
            .import_project_source(target_scope.clone(), &tampered, &NeverCancel)
            .is_err()
    );
    assert!(store.load_project(&target_scope).is_err());
    assert_eq!(
        std::fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}
