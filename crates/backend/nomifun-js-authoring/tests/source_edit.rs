use std::fs;

use nomifun_js_authoring::{
    AuthoringError, ExactDependencyLock, NeverCancel, NpmResolverIdentity,
    PluginLanguage, PluginScaffoldRequest, SourceFileEdit, SourceScope, SourceStore,
    SourceStoreLimits, UserId, PluginProjectId,
};
use tempfile::TempDir;
use uuid::Uuid;

fn id() -> String {
    Uuid::now_v7().to_string()
}

fn scope() -> SourceScope {
    SourceScope::new(UserId::from(id()), PluginProjectId::from(id())).unwrap()
}

fn fixture() -> (TempDir, SourceStore) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        SourceStore::new(temp.path().join("managed"), SourceStoreLimits::default())
            .unwrap();
    (temp, store)
}

fn scaffold() -> nomifun_js_authoring::PluginScaffoldRequest {
    PluginScaffoldRequest {
        package_id: "example.chat-dev".into(),
        package_version: "1.0.0".into(),
        display_name: "Chat Dev".into(),
        description: "Chat Dev source edit fixture.".into(),
        language: PluginLanguage::JavaScript,
    }
}

fn create_project(
    store: &SourceStore,
) -> (SourceScope, nomifun_js_authoring::ScaffoldedPluginProject) {
    let scope = scope();
    let project = store
        .create_plugin_project(scope.clone(), &scaffold(), &NeverCancel)
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
    (scope, project)
}

#[test]
fn chat_dev_source_edit_is_typed_cas_and_keeps_exact_lock_valid() {
    let (_temp, store) = fixture();
    let (scope, project) = create_project(&store);
    let old_snapshot = project.capture().snapshot().clone();
    let old_lock = store.load_dependency_lock(&scope, &NeverCancel).unwrap();

    let result = store
        .apply_source_edit(
            &scope,
            &old_snapshot,
            SourceFileEdit::replace(
                "src/main.js",
                b"export async function activate() { return { edited: true }; }\n"
                    .to_vec(),
            )
            .unwrap(),
            &NeverCancel,
        )
        .unwrap();

    assert!(result.changed());
    assert_ne!(result.capture().snapshot(), &old_snapshot);
    assert_eq!(
        store
            .load_dependency_lock(&scope, &NeverCancel)
            .unwrap()
            .digest()
            .unwrap(),
        old_lock.digest().unwrap()
    );
    assert_eq!(
        fs::read_to_string(
            store
                .load_project(&scope)
                .unwrap()
                .source_root()
                .join("src/main.js")
        )
        .unwrap(),
        "export async function activate() { return { edited: true }; }\n"
    );
}

#[test]
fn chat_dev_source_edit_rejects_stale_snapshot_without_mutation() {
    let (_temp, store) = fixture();
    let (scope, project) = create_project(&store);
    let stale = project.capture().snapshot().clone();

    let first = store
        .apply_source_edit(
            &scope,
            &stale,
            SourceFileEdit::replace("src/main.js", b"export async function activate() {}\n".to_vec())
                .unwrap(),
            &NeverCancel,
        )
        .unwrap();
    let before_second = fs::read(
        store
            .load_project(&scope)
            .unwrap()
            .source_root()
            .join("src/main.js"),
    )
    .unwrap();

    let error = store
        .apply_source_edit(
            &scope,
            &stale,
            SourceFileEdit::replace("package.json", br#"{"private":true,"type":"module"}"#.to_vec())
                .unwrap(),
            &NeverCancel,
        )
        .unwrap_err();

    assert!(matches!(error, AuthoringError::SourceChanged { .. }));
    assert_eq!(
        fs::read(
            store
                .load_project(&scope)
                .unwrap()
                .source_root()
                .join("src/main.js"),
        )
        .unwrap(),
        before_second
    );
    assert_ne!(first.capture().snapshot(), &stale);
}

#[test]
fn chat_dev_source_edit_rejects_dependency_change_until_lock_is_mutated() {
    let (_temp, store) = fixture();
    let (scope, project) = create_project(&store);
    let expected = project.capture().snapshot().clone();
    let original = fs::read(
        project
            .project()
            .source_root()
            .join("package.json"),
    )
    .unwrap();

    let error = store
        .apply_source_edit(
            &scope,
            &expected,
            SourceFileEdit::replace(
                "package.json",
                br#"{"private":true,"type":"module","dependencies":{"left-pad":"1.3.0"}}"#
                    .to_vec(),
            )
            .unwrap(),
            &NeverCancel,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        AuthoringError::DependencyLockOutOfDate { .. }
    ));
    assert_eq!(
        fs::read(
            store
                .load_project(&scope)
                .unwrap()
                .source_root()
                .join("package.json"),
        )
        .unwrap(),
        original
    );
}

#[test]
fn chat_dev_source_edit_can_delete_a_file_and_cleans_edit_staging() {
    let (_temp, store) = fixture();
    let (scope, project) = create_project(&store);
    let source_root = project.project().source_root().to_path_buf();
    fs::write(source_root.join("src/extra.js"), b"export const extra = true;\n").unwrap();
    let expected = store.snapshot(&scope, &NeverCancel).unwrap();

    let result = store
        .apply_source_edit(
            &scope,
            expected.snapshot(),
            SourceFileEdit::delete("src/extra.js").unwrap(),
            &NeverCancel,
        )
        .unwrap();

    assert!(result.changed());
    assert!(!source_root.join("src/extra.js").exists());
    assert_eq!(
        fs::read_dir(store.managed_root().join(".staging"))
            .unwrap()
            .count(),
        0
    );
}
