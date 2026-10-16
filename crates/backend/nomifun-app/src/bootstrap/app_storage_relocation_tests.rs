use super::*;
use std::{collections::{BTreeMap, BTreeSet}, fs};
use nomifun_agent_contracts::{AgentBindingValue, AgentSessionLiveRecord, AgentSessionMetadata,
    PresetRevisionRef, PrincipalRef, ResolvedSnapshotRef, SemanticSessionEventDraft,
    SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
    TypedResourceBinding, resource_definition_id};
use nomifun_agent_session::CreateSessionRequest;
use nomifun_app_hosting::{AppRecord, AppSettings, AppState, ImportKind};
use nomifun_db::sqlx;
use sha2::{Digest, Sha256};

fn text(path: &Path) -> String { path.to_string_lossy().into_owned() }

async fn seed(source: &Path) {
    fs::create_dir_all(source).unwrap();
    let db = nomifun_db::init_database(&source.join("nomifun-backend.db")).await.unwrap();
    checkpoint(db.pool()).await;
    db.close().await;
    let generation = uuid::Uuid::now_v7().to_string();
    fs::write(source.join("storage-generation"), &generation).unwrap();
    nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(source, source, &generation).unwrap();
}

async fn checkpoint(pool: &sqlx::SqlitePool) {
    let status: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)").fetch_one(pool).await.unwrap();
    assert_eq!(status, (0, 0, 0));
}

fn registry_path(source: &Path, id: &str) -> PathBuf {
    source.join("app-hosting").join("registry").join(format!("{id}.json"))
}

fn app(source: &Path, id: &str) -> AppRecord {
    let directory = source.join("app-hosting").join("apps").join(id);
    fs::create_dir_all(&directory).unwrap();
    fs::create_dir_all(source.join("app-hosting").join("registry")).unwrap();
    fs::write(directory.join("index.html"), b"<!doctype html><title>retained app</title>").unwrap();
    AppRecord { id: id.into(), name: "Retained application".into(), description: "Application-owned project".into(),
        tags: vec!["fixture".into()], development_session_id: None, directory,
        import_kind: ImportKind::Source, settings: AppSettings::default(), state: AppState::Stopped,
        operation: None, last_error: None, output: String::new(), pid: None, preview: false, prepared: false }
}

fn save_app(source: &Path, app: &AppRecord) {
    fs::write(registry_path(source, &app.id), serde_json::to_vec_pretty(&serde_json::json!({
        "record":app,"identity":null,
    })).unwrap()).unwrap();
}

async fn session(store: &AgentSessionStore, owner: &PrincipalRef, root: &Path) -> (String, AgentBindingValue) {
    let id = uuid::Uuid::now_v7().to_string();
    let identity = serde_json::to_vec(&(owner.principal_id.as_str(), text(root))).unwrap();
    let mut bindings = Vec::new();
    for (kind, resource_id, operations) in [
        ("workspace", format!("selected-workspace-{:x}", Sha256::digest(&identity)), BTreeSet::from(["read".into(), "write".into()])),
        ("process_session", "managed-process-session".into(), BTreeSet::from(["execute".into(), "observe".into()])),
        ("project_memory", format!("project-workspace-memory-{:x}", Sha256::digest(&identity)), BTreeSet::from(["read".into(), "write".into()])),
    ] {
        let mut binding = TypedResourceBinding { binding_id: "unassigned".into(), resource_kind: kind.into(),
            resource_id: resource_id.into(), owner_id: owner.principal_id.clone(), operations,
            connection_config_ref: None, typed_parameters: BTreeMap::from([("workspace_root".into(), text(root))]) };
        binding.binding_id = resource_definition_id(&binding).unwrap();
        bindings.push(binding);
    }
    let binding = AgentBindingValue {
        preset_revision_ref: PresetRevisionRef { preset_id: "personal-project-editor".into(), revision: 1,
            revision_digest: "a".repeat(64).into() },
        resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: "saved-project-snapshot".into(),
            snapshot_digest: "b".repeat(64).into() }, typed_resource_bindings: bindings, binding_version: 1,
    };
    let created = store.create_session(CreateSessionRequest::new(AgentSessionLiveRecord {
        agent_session_id: id.as_str().into(), owner_ref: owner.clone(), metadata: AgentSessionMetadata {
            purpose: Default::default(), app_project_id: Some(root.file_name().unwrap().to_str().unwrap().into()), title: Some("Existing project history".into()), pinned: false,
            archived: false, reasoning_effort: None, companion: None }, agent_binding: binding.clone(),
        remote_binding_provenance: None, parent_session_id: None, fork_base_payload_id: None, next_seq: 1,
    }, 1_788_000_000_000, format!("fixture-open-{id}"), "session_api".into(),
        format!("fixture-open-{id}").into(), format!("fixture-open-{id}").into())).await.unwrap();
    store.append_event(&SessionEventAppend { agent_session_id: id.as_str().into(),
        event_id: format!("fixture-ready-{id}").into(), producer_id: "runtime-supervisor".into(),
        idempotency_key: format!("fixture-ready-{id}").into(), semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("session/ready".into()), kind_version: 1,
            correlation_id: format!("fixture-ready-{id}").into(), causation_event_id: Some(created.opening_ack.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::json!({}))),
        } }).await.unwrap();
    (id, binding)
}

#[tokio::test]
async fn storage_relocation_boot_moves_registered_app_projects_and_preserves_history() {
    let fixture = tempfile::tempdir().unwrap();
    let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
    let source = base.join("source"); let target = base.join("target"); let control = base.join("control");
    seed(&source).await;
    let first_id = uuid::Uuid::now_v7().to_string(); let second_id = uuid::Uuid::now_v7().to_string();
    let mut first = app(&source, &first_id); let second = app(&source, &second_id);
    let database = nomifun_db::init_database(&source.join("nomifun-backend.db")).await.unwrap();
    let owner_id: String = sqlx::query_scalar("SELECT owner_user_id FROM installation_identity").fetch_one(database.pool()).await.unwrap();
    let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id };
    let store = AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
    let mut sessions = Vec::new();
    for root in [&first.directory, &first.directory, &second.directory] {
        let (id, binding) = session(&store, &principal, root).await;
        let opening = store.read_events(&id.as_str().into(), None, 1).await.unwrap().events.remove(0);
        sessions.push((id, binding, opening));
    }
    first.development_session_id = Some(sessions[1].0.clone());
    save_app(&source, &first); save_app(&source, &second);
    let source_registries: Vec<_> = [&first, &second].into_iter().map(|app|
        (app.id.clone(), fs::read(registry_path(&source, &app.id)).unwrap())).collect();
    checkpoint(database.pool()).await; database.close().await;
    let source_database = fs::read(source.join("nomifun-backend.db")).unwrap();
    let plan = storage::request_storage_relocation(&control, &source, &source, &target).unwrap();
    storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
    assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
    assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), target);
    assert_eq!(fs::read(source.join("nomifun-backend.db")).unwrap(), source_database);
    for (id, bytes) in source_registries {
        assert_eq!(fs::read(registry_path(&source, &id)).unwrap(), bytes);
        assert_eq!(fs::read(target.join("app-hosting").join("apps").join(id).join("index.html")).unwrap(),
            b"<!doctype html><title>retained app</title>");
    }
    let moved = nomifun_db::open_database_for_backup(&target.join("nomifun-backend.db")).await.unwrap();
    let moved_store = AgentSessionStore::from_pool(moved.pool().clone()).await.unwrap();
    for (id, original, opening) in &sessions {
        let current = moved_store.get_live_session(&id.as_str().into()).await.unwrap();
        let app_id = Path::new(&original.typed_resource_bindings[0].typed_parameters["workspace_root"])
            .file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(current.metadata.app_project_id.as_deref(), Some(app_id.as_str()));
        assert_eq!(current.agent_binding.preset_revision_ref, original.preset_revision_ref);
        assert_eq!(current.agent_binding.resolved_snapshot_ref, original.resolved_snapshot_ref);
        assert_eq!(current.agent_binding.binding_version, original.binding_version + 1);
        assert_eq!(moved_store.read_events(&id.as_str().into(), None, 1).await.unwrap().events[0], *opening);
        for previous in &original.typed_resource_bindings {
            let updated = current.agent_binding.typed_resource_bindings.iter().find(|item| item.resource_kind == previous.resource_kind).unwrap();
            assert_eq!(updated.resource_id, previous.resource_id);
            assert_eq!(updated.operations, previous.operations);
            assert_eq!(updated.owner_id, previous.owner_id);
            let app_id = if previous.typed_parameters["workspace_root"] == text(&first.directory) { &first_id } else { &second_id };
            assert_eq!(updated.typed_parameters["workspace_root"], text(&target.join("app-hosting").join("apps").join(app_id)));
        }
    }
    moved.close().await;
    let owner = nomifun_app_hosting::HostingOwner::new(target.join("app-hosting"),
        nomifun_app_hosting::LocalConnection { host_url: "http://127.0.0.1:1".into(), local_trust: "fixture-memory-only".into() }).await.unwrap();
    let loaded_first = owner.get(&first_id).await.unwrap(); let loaded_second = owner.get(&second_id).await.unwrap();
    assert_ne!(loaded_first.directory, loaded_second.directory);
    assert_eq!(loaded_first.directory, target.join("app-hosting").join("apps").join(&first_id));
    assert_eq!(loaded_first.development_session_id, first.development_session_id);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_relocation_boot_refuses_unconfirmed_app_process_without_mutating_source() {
    let fixture = tempfile::tempdir().unwrap();
    let base = nomifun_common::paths::canonicalize_simplified(fixture.path()).unwrap();
    let source = base.join("source"); let target = base.join("target"); let control = base.join("control");
    seed(&source).await;
    let id = uuid::Uuid::now_v7().to_string(); let mut current = app(&source, &id);
    current.pid = Some(1); current.state = AppState::Running; save_app(&source, &current);
    let registry = fs::read(registry_path(&source, &id)).unwrap();
    let database = fs::read(source.join("nomifun-backend.db")).unwrap();
    let code = fs::read(current.directory.join("index.html")).unwrap();
    let plan = storage::request_storage_relocation(&control, &source, &source, &target).unwrap();
    storage::start_storage_relocation(&control, &plan.operation_id).unwrap();
    assert_eq!(prepare_startup_storage_root_async(source.clone(), None).await.unwrap(), source);
    assert!(!target.exists());
    assert!(!plan.staging_data_dir.exists());
    assert_eq!(fs::read(registry_path(&source, &id)).unwrap(), registry);
    assert_eq!(fs::read(source.join("nomifun-backend.db")).unwrap(), database);
    assert_eq!(fs::read(current.directory.join("index.html")).unwrap(), code);
    assert!(storage::resolve_storage_location(&control).unwrap().is_none());
    assert!(storage::require_storage_root_not_sealed(&source).is_ok());
}
