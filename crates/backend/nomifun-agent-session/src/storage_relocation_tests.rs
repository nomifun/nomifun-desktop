use super::*;
use std::path::Path;
use serde_json::Value;
use nomifun_agent_contracts::{digest_payload, managed_workspace_parameters, resource_definition_id};

fn resource(kind: &str, id: &str, parameter: &str, root: &str) -> TypedResourceBinding {
    let mut value = TypedResourceBinding {
        binding_id: "unassigned".into(), resource_kind: kind.into(), resource_id: id.into(),
        owner_id: owner().principal_id, operations: BTreeSet::from(["read".into()]),
        connection_config_ref: None, typed_parameters: BTreeMap::from([(parameter.into(), root.into())]),
    };
    value.binding_id = resource_definition_id(&value).unwrap();
    value
}

async fn ready_with_resources(store: &AgentSessionStore, resources: Vec<TypedResourceBinding>, key: &str)
    -> AgentSessionLiveRecord {
    let mut value = live_session(session_id());
    value.agent_binding.typed_resource_bindings = resources;
    let created = store.create_session(create_request(value, key)).await.unwrap();
    store.append_event(&append(&created.session.agent_session_id, &format!("ready-{key}"), "runtime-supervisor",
        &format!("ready-{key}"), "session/ready", key, Some(created.opening_ack.event_id), json!({}))).await.unwrap();
    created.session
}

async fn app_relocation_store() -> (AgentSessionStore, PrincipalRef) {
    use nomifun_db::IUserRepository;
    let db = nomifun_db::init_database_memory().await.unwrap();
    let user = nomifun_db::SqliteUserRepository::new(db.pool().clone())
        .get_system_user().await.unwrap().unwrap();
    (AgentSessionStore::from_pool(db.pool().clone()).await.unwrap(),
        PrincipalRef { principal_kind: "user".into(), principal_id: user.user_id.to_string() })
}

async fn ready_app_with_resources(store: &AgentSessionStore, owner: &PrincipalRef,
    mut resources: Vec<TypedResourceBinding>, key: &str) -> AgentSessionLiveRecord {
    let mut value = live_session(session_id());
    value.owner_ref = owner.clone();
    for resource in &mut resources {
        resource.owner_id = owner.principal_id.clone();
        resource.binding_id = resource_definition_id(resource).unwrap();
    }
    value.agent_binding.typed_resource_bindings = resources;
    let created = store.create_session(create_request(value, key)).await.unwrap();
    store.append_event(&append(&created.session.agent_session_id, &format!("ready-{key}"), "runtime-supervisor",
        &format!("ready-{key}"), "session/ready", key, Some(created.opening_ack.event_id), json!({}))).await.unwrap();
    created.session
}

fn app_project_resources(root: &str, suffix: &str) -> Vec<TypedResourceBinding> {
    vec![resource("workspace", &format!("selected-workspace-{suffix}"), "workspace_root", root),
        resource("process_session", "managed-process-session", "workspace_root", root),
        resource("project_memory", &format!("project-workspace-memory-{suffix}"), "workspace_root", root)]
}

#[tokio::test]
async fn registered_app_projects_relocate_all_sessions_and_history_with_stable_resource_identity() {
    let (store, current_owner) = app_relocation_store().await;
    let old = tempfile::tempdir().unwrap(); let middle = tempfile::tempdir().unwrap(); let final_root = tempfile::tempdir().unwrap();
    let source = old.path().to_path_buf();
    let app_one = Uuid::now_v7().to_string(); let app_two = Uuid::now_v7().to_string();
    let app_ids = BTreeSet::from([app_one.clone(), app_two.clone()]);
    let root_one = source.join("app-hosting").join("apps").join(&app_one).to_string_lossy().into_owned();
    let root_two = source.join("app-hosting").join("apps").join(&app_two).to_string_lossy().into_owned();
    let first = ready_app_with_resources(&store, &current_owner, app_project_resources(&root_one, "one"), "app-one-first").await;
    let second = ready_app_with_resources(&store, &current_owner, app_project_resources(&root_one, "one"), "app-one-second").await;
    let other_app = ready_app_with_resources(&store, &current_owner, app_project_resources(&root_two, "two"), "app-two-first").await;
    let external_root = format!("{root_one}-external");
    let external = ready_app_with_resources(&store, &current_owner, app_project_resources(&external_root, "external"), "external-project").await;
    let unknown_id = Uuid::now_v7().to_string();
    let unregistered_root = source.join("app-hosting").join("apps").join(unknown_id).to_string_lossy().into_owned();
    let unregistered = ready_app_with_resources(&store, &current_owner, app_project_resources(&unregistered_root, "unregistered"), "unregistered-app-directory").await;
    let old_file = Path::new(&root_one).join("report.txt").to_string_lossy().into_owned();
    let file = store.append_event(&append(&first.agent_session_id, &Uuid::now_v7().to_string(), "session-api",
        "app-file", "message/user-accepted", "app-file", None,
        json!({"content":format!("[[NOMI_FILES]]\n{old_file}"),"files":[old_file]}))).await.unwrap().record.unwrap();
    let receipt = historical_file_receipt(&store, &first, &root_one, "app-history-receipt").await;
    let opening: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/opening'")
        .bind(first.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let operation = OperationId::from(Uuid::now_v7().to_string());
    store.validate_workspace_relocation_with_app_workspaces(&source, &source, &app_ids).await.unwrap();
    let moved = store.relocate_managed_workspaces_with_app_workspaces(&source, &source, middle.path(), &operation, &app_ids).await.unwrap();
    assert_eq!(moved.changed_sessions, 3);
    assert!(moved.managed_namespaces.is_empty());
    assert_eq!(moved.app_workspaces.len(), 3);
    assert_eq!(moved.app_workspaces[&first.agent_session_id], moved.app_workspaces[&second.agent_session_id]);
    assert_ne!(moved.app_workspaces[&first.agent_session_id], moved.app_workspaces[&other_app.agent_session_id]);
    let replay = store.relocate_managed_workspaces_with_app_workspaces(&source, &source, middle.path(), &operation, &app_ids).await.unwrap();
    assert_eq!(replay.changed_sessions, 0);
    assert_eq!(replay.app_workspaces, moved.app_workspaces);
    assert!(store.relocate_managed_workspaces_with_app_workspaces(&source, &source, middle.path(), &operation, &BTreeSet::new()).await.is_err());
    store.validate_workspace_relocation_with_app_workspaces(middle.path(), middle.path(), &app_ids).await.unwrap();
    store.relocate_managed_workspaces_with_app_workspaces(middle.path(), middle.path(), final_root.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &app_ids).await.unwrap();
    let expected_root = final_root.path().join("app-hosting").join("apps").join(&app_one);
    for session in [&first, &second, &other_app] {
        let updated = store.get_live_session(&session.agent_session_id).await.unwrap();
        let expected = final_root.path().join("app-hosting").join("apps").join(
            if session.agent_session_id == other_app.agent_session_id { &app_two } else { &app_one });
        assert_eq!(updated.agent_binding.preset_revision_ref, session.agent_binding.preset_revision_ref);
        assert_eq!(updated.agent_binding.resolved_snapshot_ref, session.agent_binding.resolved_snapshot_ref);
        let (inline, body): (Option<String>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT event.inline_json,payload.body FROM agent_events event LEFT JOIN agent_payloads payload ON payload.payload_id=event.payload_id AND payload.session_id=event.session_id WHERE event.session_id=? AND event.event_id=?")
            .bind(session.agent_session_id.as_ref()).bind(format!("storage-relocation:{}:{}", operation.as_ref(), session.agent_session_id.as_ref()))
            .fetch_one(store.test_pool()).await.unwrap();
        let proof_value = if let Some(inline) = inline { serde_json::from_str(&inline).unwrap() } else {
            match serde_json::from_slice::<SessionPayloadBody>(&body.unwrap()).unwrap() {
                SessionPayloadBody::Json(value) => value.0,
                _ => panic!("location boundary must retain an exact typed canonical JSON proof"),
            }
        };
        let transition: nomifun_agent_contracts::AgentBindingChangedPayloadV1 = serde_json::from_value(proof_value).unwrap();
        transition.validate_storage_relocation().unwrap();
        assert_eq!(transition.workspace_relocation.unwrap().previous_resources, session.agent_binding.typed_resource_bindings);
        for (before, after) in session.agent_binding.typed_resource_bindings.iter().zip(&updated.agent_binding.typed_resource_bindings) {
            assert_eq!(before.resource_id, after.resource_id);
            assert_eq!(before.operations, after.operations);
            assert_eq!(before.connection_config_ref, after.connection_config_ref);
            assert_ne!(before.binding_id, after.binding_id);
            assert_eq!(after.typed_parameters["workspace_root"], expected.to_string_lossy());
            if session.agent_session_id == first.agent_session_id && before.resource_kind.as_ref() == "workspace" {
                let retained: String = sqlx::query_scalar("SELECT typed_parameters_json FROM agent_session_resources WHERE session_id=? AND binding_id=?")
                    .bind(session.agent_session_id.as_ref()).bind(before.binding_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
                assert_eq!(serde_json::from_str::<BTreeMap<String,String>>(&retained).unwrap(), before.typed_parameters,
                    "the historical file Effect still owns its exact original definition");
            }
        }
    }
    assert_eq!(store.get_live_session(&external.agent_session_id).await.unwrap().agent_binding, external.agent_binding);
    assert_eq!(store.get_live_session(&unregistered.agent_session_id).await.unwrap().agent_binding, unregistered.agent_binding);
    let explicit_old = store.append_event(&append(&first.agent_session_id, &Uuid::now_v7().to_string(), "session-api",
        "app-explicit-old", "message/user-accepted", "app-explicit-old", None,
        json!({"content":old_file,"files":[old_file]}))).await.unwrap().record.unwrap();
    std::fs::create_dir_all(&expected_root).unwrap();
    std::fs::write(expected_root.join("report.txt"), b"migrated project file").unwrap();
    old.close().unwrap(); middle.close().unwrap();
    let wrong_app_file = Path::new(&root_two).join("report.txt").to_string_lossy().into_owned();
    let adjacent = format!("{root_one}-other/report.txt");
    let candidates = BTreeMap::from([(file.seq, BTreeSet::from([old_file.clone(), wrong_app_file.clone(), adjacent.clone()])),
        (explicit_old.seq, BTreeSet::from([old_file.clone()]))]);
    let managed = final_root.path().join("conversations");
    let paths = store.managed_history_file_paths(&first.agent_session_id, &managed, &candidates).await.unwrap();
    assert_eq!(std::fs::read(&paths[&file.seq][&old_file]).unwrap(), b"migrated project file");
    assert!(!paths[&file.seq].contains_key(&wrong_app_file));
    assert!(!paths[&file.seq].contains_key(&adjacent));
    assert!(paths[&explicit_old.seq].is_empty());
    let hash = digest_bytes(root_one.as_bytes()).as_ref().to_owned();
    let roots = store.history_workspace_roots(&first.agent_session_id, &managed,
        &BTreeMap::from([(receipt.seq, BTreeSet::from([hash.clone()]))])).await.unwrap();
    assert_eq!(roots[&receipt.seq][&hash], expected_root.to_string_lossy());
    let retained: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/opening'")
        .bind(first.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(retained, opening);
    let original = store.read_events(&first.agent_session_id, None, 100).await.unwrap().events.into_iter()
        .find(|event| event.event_id == file.event_id).unwrap();
    assert_eq!(original.payload, file.payload);
    assert!(store.managed_history_file_paths(&first.agent_session_id, &source.join("conversations"), &candidates)
        .await.unwrap().is_empty(), "an App outside the requested active dataset supplies no managed aliases");
}

#[tokio::test]
async fn deleted_app_history_survives_a_later_dataset_move_without_adopting_a_new_project_root() {
    let (store, current_owner) = app_relocation_store().await;
    let source = tempfile::tempdir().unwrap(); let middle = tempfile::tempdir().unwrap(); let target = tempfile::tempdir().unwrap();
    let app = Uuid::now_v7().to_string(); let ids = BTreeSet::from([app.clone()]);
    let source_root = source.path().join("app-hosting").join("apps").join(&app).to_string_lossy().into_owned();
    let middle_root = middle.path().join("app-hosting").join("apps").join(&app).to_string_lossy().into_owned();
    let session = ready_app_with_resources(&store, &current_owner, app_project_resources(&source_root, "deleted"), "deleted-app-history").await;
    let file = Path::new(&source_root).join("report.docx").to_string_lossy().into_owned();
    let message = store.append_event(&append(&session.agent_session_id, &Uuid::now_v7().to_string(), "session-api",
        "deleted-app-file", "message/user-accepted", "deleted-app-file", None,
        json!({"content":format!("Saved project file: {file}"),"files":[file]}))).await.unwrap().record.unwrap();
    let original_receipt = historical_file_receipt(&store, &session, &source_root, "deleted-app-original").await;
    store.relocate_managed_workspaces_with_app_workspaces(source.path(), source.path(), middle.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &ids).await.unwrap();
    let relocated = store.get_live_session(&session.agent_session_id).await.unwrap();
    let middle_receipt = historical_file_receipt(&store, &relocated, &middle_root, "deleted-app-relocated").await;
    let retained_events = store.read_events(&session.agent_session_id, None, 100).await.unwrap().events;
    let retained_resource: String = sqlx::query_scalar("SELECT typed_parameters_json FROM agent_session_resources WHERE session_id=? AND binding_id=?")
        .bind(session.agent_session_id.as_ref()).bind(session.agent_binding.typed_resource_bindings[0].binding_id.as_ref())
        .fetch_one(store.test_pool()).await.unwrap();
    std::fs::create_dir_all(&middle_root).unwrap();
    std::fs::write(Path::new(&middle_root).join("report.docx"), b"application work").unwrap();
    std::fs::remove_dir_all(&middle_root).unwrap();
    store.validate_workspace_relocation(middle.path(), middle.path()).await.unwrap();
    let outcome = store.relocate_managed_workspaces(middle.path(), middle.path(), target.path(),
        &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    assert_eq!(outcome.changed_sessions, 0, "a deleted App has no registry authority on the second dataset move");
    assert!(outcome.app_workspaces.is_empty());
    assert_eq!(store.get_live_session(&session.agent_session_id).await.unwrap().agent_binding, relocated.agent_binding);
    let managed = target.path().join("conversations");
    let paths = store.managed_history_file_paths(&session.agent_session_id, &managed,
        &BTreeMap::from([(message.seq, BTreeSet::from([file.clone()]))])).await.unwrap();
    assert!(paths.is_empty(), "old message paths are retained without any target App alias");
    let original_hash = digest_bytes(source_root.as_bytes()).as_ref().to_owned();
    let middle_hash = digest_bytes(middle_root.as_bytes()).as_ref().to_owned();
    let roots = store.history_workspace_roots(&session.agent_session_id, &managed, &BTreeMap::from([
        (original_receipt.seq, BTreeSet::from([original_hash.clone()])),
        (middle_receipt.seq, BTreeSet::from([middle_hash.clone()])),
    ])).await.unwrap();
    assert_eq!(roots[&original_receipt.seq][&original_hash], source_root,
        "the original Effect remains tied to its immutable external definition");
    assert_eq!(roots[&middle_receipt.seq][&middle_hash], middle_root);
    assert_eq!(store.read_events(&session.agent_session_id, None, 100).await.unwrap().events, retained_events);
    let current_retained: String = sqlx::query_scalar("SELECT typed_parameters_json FROM agent_session_resources WHERE session_id=? AND binding_id=?")
        .bind(session.agent_session_id.as_ref()).bind(session.agent_binding.typed_resource_bindings[0].binding_id.as_ref())
        .fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(current_retained, retained_resource);
    assert!(!target.path().join("app-hosting").join("apps").join(app).exists());
    assert!(!Path::new(&middle_root).exists(), "history reads cannot recreate the deleted application directory");
}

#[tokio::test]
async fn external_classification_does_not_hide_tampered_application_proofs_or_current_bindings() {
    let (store, current_owner) = app_relocation_store().await;
    let source = tempfile::tempdir().unwrap(); let middle = tempfile::tempdir().unwrap(); let target = tempfile::tempdir().unwrap();
    let app = Uuid::now_v7().to_string(); let ids = BTreeSet::from([app.clone()]);
    let root = source.path().join("app-hosting").join("apps").join(&app).to_string_lossy().into_owned();
    let session = ready_app_with_resources(&store, &current_owner, app_project_resources(&root, "tampered"), "tampered-app-history").await;
    let file = Path::new(&root).join("report.docx").to_string_lossy().into_owned();
    let message = store.append_event(&append(&session.agent_session_id, &Uuid::now_v7().to_string(), "session-api",
        "tampered-app-file", "message/user-accepted", "tampered-app-file", None,
        json!({"content":file,"files":[file]}))).await.unwrap().record.unwrap();
    let receipt = historical_file_receipt(&store, &session, &root, "tampered-app-file").await;
    store.relocate_managed_workspaces_with_app_workspaces(source.path(), source.path(), middle.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &ids).await.unwrap();
    store.relocate_managed_workspaces(middle.path(), middle.path(), target.path(),
        &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let current = store.get_live_session(&session.agent_session_id).await.unwrap();
    let mut tampered = current.agent_binding.clone();
    tampered.typed_resource_bindings[1].typed_parameters.insert("workspace_root".into(), root.clone());
    tampered.typed_resource_bindings[1].binding_id = resource_definition_id(&tampered.typed_resource_bindings[1]).unwrap();
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&tampered).unwrap()).bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    let managed = target.path().join("conversations");
    let paths = BTreeMap::from([(message.seq, BTreeSet::from([file]))]);
    let roots = BTreeMap::from([(receipt.seq, BTreeSet::from([digest_bytes(root.as_bytes()).as_ref().to_owned()]))]);
    assert!(store.managed_history_file_paths(&session.agent_session_id, &managed, &paths).await.is_err());
    assert!(store.history_workspace_roots(&session.agent_session_id, &managed, &roots).await.is_err());
    // Raising a binding version cannot turn a definition absent from the
    // canonical current registry into a legitimate later selection either.
    tampered.binding_version += 1;
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&tampered).unwrap()).bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.managed_history_file_paths(&session.agent_session_id, &managed, &paths).await.is_err());
    assert!(store.history_workspace_roots(&session.agent_session_id, &managed, &roots).await.is_err());
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&current.agent_binding).unwrap()).bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    let original: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' ORDER BY seq DESC LIMIT 1")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let mut transition: nomifun_agent_contracts::AgentBindingChangedPayloadV1 = serde_json::from_str(&original).unwrap();
    let proof = transition.workspace_relocation.as_mut().unwrap();
    proof.next_resources[1] = proof.previous_resources[1].clone();
    transition.request_digest = digest_payload(proof).unwrap();
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(serde_json::to_string(&transition).unwrap()).bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.managed_history_file_paths(&session.agent_session_id, &managed, &paths).await.is_err(),
        "a partial location proof cannot be swallowed by external classification");
    assert!(store.history_workspace_roots(&session.agent_session_id, &managed, &roots).await.is_err());
}

#[tokio::test]
async fn app_relocation_preserves_existing_installation_project_memory_binding() {
    let (store, current_owner) = app_relocation_store().await;
    let source = tempfile::tempdir().unwrap(); let target = tempfile::tempdir().unwrap();
    let app = Uuid::now_v7().to_string(); let ids = BTreeSet::from([app.clone()]);
    let root = source.path().join("app-hosting").join("apps").join(&app).to_string_lossy().into_owned();
    let mut resources = app_project_resources(&root, "existing");
    resources.pop();
    let mut global_memory = resource("project_memory", "default-project-memory", "workspace_root", &root);
    global_memory.typed_parameters.clear();
    global_memory.owner_id = current_owner.principal_id.clone();
    global_memory.binding_id = resource_definition_id(&global_memory).unwrap();
    resources.push(global_memory.clone());
    let session = ready_app_with_resources(&store, &current_owner, resources, "existing-default-memory").await;
    store.relocate_managed_workspaces_with_app_workspaces(source.path(), source.path(), target.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &ids).await.unwrap();
    let current = store.get_live_session(&session.agent_session_id).await.unwrap();
    assert_eq!(current.agent_binding.typed_resource_bindings[2], global_memory);
    assert!(current.agent_binding.typed_resource_bindings[..2].iter().all(|resource|
        resource.typed_parameters["workspace_root"] == target.path().join("app-hosting").join("apps").join(&app).to_string_lossy()));
    store.validate_workspace_relocation_with_app_workspaces(target.path(), target.path(), &ids).await.unwrap();
}

#[tokio::test]
async fn registered_app_projects_reject_other_installation_owners_and_mixed_current_roots() {
    use nomifun_db::IUserRepository;
    let (store, _) = app_relocation_store().await;
    let source = tempfile::tempdir().unwrap(); let target = tempfile::tempdir().unwrap();
    let app = Uuid::now_v7().to_string(); let ids = BTreeSet::from([app.clone()]);
    let root = source.path().join("app-hosting").join("apps").join(app).to_string_lossy().into_owned();
    let other = nomifun_db::SqliteUserRepository::new(store.test_pool().clone()).create_user("other-app-owner", "unused-test-hash").await.unwrap();
    let other_owner = PrincipalRef { principal_kind: "user".into(), principal_id: other.user_id.to_string() };
    let session = ready_app_with_resources(&store, &other_owner, app_project_resources(&root, "owned"), "app-owner-proof").await;
    assert!(store.relocate_managed_workspaces_with_app_workspaces(source.path(), source.path(), target.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &ids).await.is_err());
    assert_eq!(store.get_live_session(&session.agent_session_id).await.unwrap().agent_binding, session.agent_binding);
    let (store, current_owner) = app_relocation_store().await;
    let session = ready_app_with_resources(&store, &current_owner, app_project_resources(&root, "owned"), "app-mixed-before").await;
    let mut inconsistent = app_project_resources(&root, "mixed");
    inconsistent[1] = resource("process_session", "managed-process-session", "workspace_root", target.path().to_str().unwrap());
    ready_app_with_resources(&store, &current_owner, inconsistent, "app-mixed-roots").await;
    assert!(store.validate_workspace_relocation_with_app_workspaces(source.path(), source.path(), &ids).await.is_err());
    assert!(store.relocate_managed_workspaces_with_app_workspaces(source.path(), source.path(), target.path(),
        &OperationId::from(Uuid::now_v7().to_string()), &ids).await.is_err());
    assert_eq!(store.get_live_session(&session.agent_session_id).await.unwrap().agent_binding, session.agent_binding);
}

#[tokio::test]
async fn storage_relocation_preserves_history_and_external_projects_and_is_idempotent() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let source = old.path().to_str().unwrap();
    let resources = vec![resource("workspace", "default-workspace", "workspace_root", source),
        resource("process_session", "managed-process-session", "workspace_root", source)];
    let session = ready_with_resources(&store, resources.clone(), "managed-relocation").await;
    let external = ready_with_resources(&store,
        vec![resource("workspace", "selected-workspace:external", "workspace_root", source),
            resource("process_session", "managed-process-session", "workspace_root", source)], "external-relocation").await;
    let opening: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/opening'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let operation = OperationId::from(Uuid::now_v7().to_string());
    store.validate_workspace_relocation(old.path(), old.path()).await.unwrap();
    let outcome = store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &operation).await.unwrap();
    assert_eq!(outcome.changed_sessions, 1);
    assert_eq!(outcome.managed_namespaces.len(), 1);
    assert_eq!(outcome.managed_namespaces[0].agent_session_id, session.agent_session_id);
    assert_eq!(outcome.managed_namespaces[0].opening_workspace, resources[0]);
    let updated = store.get_live_session(&session.agent_session_id).await.unwrap();
    assert_eq!(updated.agent_binding.binding_version, session.agent_binding.binding_version + 1);
    assert_eq!(updated.agent_binding.preset_revision_ref, session.agent_binding.preset_revision_ref);
    assert_eq!(updated.agent_binding.resolved_snapshot_ref, session.agent_binding.resolved_snapshot_ref);
    assert!(updated.agent_binding.typed_resource_bindings.iter().all(|r| r.typed_parameters == managed_workspace_parameters()));
    assert_eq!(store.get_live_session(&external.agent_session_id).await.unwrap().agent_binding, external.agent_binding);
    let retained: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/opening'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(opening, retained);
    let raw: String = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let transition: nomifun_agent_contracts::AgentBindingChangedPayloadV1 = serde_json::from_str(&raw).unwrap();
    transition.validate_storage_relocation().unwrap();
    assert_eq!(transition.workspace_relocation.unwrap().previous_resources, resources);
    let replay = store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &operation).await.unwrap();
    assert_eq!(replay.changed_sessions, 0);
    assert_eq!(replay.managed_namespaces, outcome.managed_namespaces);
    assert!(store.relocate_managed_workspaces(old.path(), old.path(), Path::new(source), &operation).await.is_err());
    store.validate_workspace_relocation(new.path(), new.path()).await.unwrap();
}

#[tokio::test]
async fn storage_relocation_history_resolves_two_moves_after_old_locations_are_offline() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let intermediate = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let source = old.path().to_path_buf(); let middle = intermediate.path().to_path_buf();
    let session = ready_with_resources(&store,
        vec![resource("workspace", "default-workspace", "workspace_root", source.to_str().unwrap())], "twice-relocated-history").await;
    let id = &session.agent_session_id;
    let original = source.join("conversations").join(id.as_ref()).join("uploads").join("report_nomifun_1234567890123.png").to_string_lossy().into_owned();
    let display = source.join("conversations").join(id.as_ref()).join("uploads").join("report.png").to_string_lossy().into_owned();
    let first = append(&id, &Uuid::now_v7().to_string(), "session-api", "first-file-history", "message/user-accepted", "first-file-history", None,
        json!({"content":format!("附件\n[[NOMI_FILES]]\n{display}"),"files":[original]}));
    let first = store.append_event(&first).await.unwrap().record.unwrap();
    let first_payload = first.payload.clone();
    store.relocate_managed_workspaces(&source, &source, &middle, &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let middle_file = middle.join("conversations").join(id.as_ref()).join("result.txt").to_string_lossy().into_owned();
    let second = append(&id, &Uuid::now_v7().to_string(), "session-api", "second-file-history", "message/user-accepted", "second-file-history", None,
        json!({"content":format!("[[NOMI_FILES]]\n{middle_file}"),"files":[middle_file]}));
    let second = store.append_event(&second).await.unwrap().record.unwrap();
    let operation = OperationId::from(Uuid::now_v7().to_string());
    let outcome = store.relocate_managed_workspaces(&middle, &middle, new.path(), &operation).await.unwrap();
    assert_eq!(outcome.changed_sessions, 1,
        "unchanged logical resource definitions still require a canonical location proof");
    assert_eq!(outcome.managed_namespaces[0].agent_session_id, *id);
    assert_eq!(outcome.managed_namespaces[0].opening_workspace, session.agent_binding.typed_resource_bindings[0]);
    let replay = store.relocate_managed_workspaces(&middle, &middle, new.path(), &operation).await.unwrap();
    assert_eq!(replay.changed_sessions, 0);
    assert_eq!(replay.managed_namespaces, outcome.managed_namespaces);
    let explicit_old = append(&id, &Uuid::now_v7().to_string(), "session-api", "explicit-old-file", "message/user-accepted", "explicit-old-file", None,
        json!({"content":format!("[[NOMI_FILES]]\n{original}"),"files":[original]}));
    let explicit_old = store.append_event(&explicit_old).await.unwrap().record.unwrap();
    let another_session = source.join("conversations").join(Uuid::now_v7().to_string()).join("report.png").to_string_lossy().into_owned();
    let adjacent = format!("{}-adjacent/file.png", source.join("conversations").join(id.as_ref()).display());
    let traversal = format!("{}/../escape.png", source.join("conversations").join(id.as_ref()).display());
    let candidates = BTreeMap::from([
        (first.seq, BTreeSet::from([display.clone(), original.clone(), another_session.clone(), adjacent.clone(), traversal.clone()])),
        (second.seq, BTreeSet::from([middle_file.clone()])),
        (explicit_old.seq, BTreeSet::from([original.clone()])),
    ]);
    let current_root = new.path().join("conversations");
    let actual_new = current_root.join(id.as_ref()).join("uploads").join("report_nomifun_1234567890123.png");
    std::fs::create_dir_all(actual_new.parent().unwrap()).unwrap();
    std::fs::write(&actual_new, b"copied attachment").unwrap();
    old.close().unwrap(); intermediate.close().unwrap();
    assert!(!source.exists() && !middle.exists());
    let paths = store.managed_history_file_paths(id, &current_root, &candidates).await.unwrap();
    assert_eq!(paths[&first.seq][&display], actual_new.to_string_lossy());
    assert_eq!(paths[&first.seq][&original], actual_new.to_string_lossy());
    assert_eq!(std::fs::read(&paths[&first.seq][&display]).unwrap(), b"copied attachment");
    assert!(!paths[&first.seq].contains_key(&another_session));
    assert!(!paths[&first.seq].contains_key(&adjacent));
    assert!(!paths[&first.seq].contains_key(&traversal));
    assert_eq!(paths[&second.seq][&middle_file], current_root.join(id.as_ref()).join("result.txt").to_string_lossy());
    assert!(paths[&explicit_old.seq].is_empty(), "new explicit references to an old directory are not relocation aliases");
    let retained = store.read_events(id, None, 100).await.unwrap().events.into_iter().find(|event| event.event_id == first.event_id).unwrap();
    assert_eq!(retained.payload, first_payload);
    let changed = store.get_live_session(id).await.unwrap();
    assert_eq!(changed.agent_binding.binding_version, session.agent_binding.binding_version + 2);
    assert_eq!(changed.agent_binding.resolved_snapshot_ref, session.agent_binding.resolved_snapshot_ref);
    assert!(store.managed_history_file_paths(id, &middle.join("conversations"), &candidates).await.is_err(), "active root must match the last canonical target");
}

#[tokio::test]
async fn storage_relocation_retains_external_workspace_and_resolves_earlier_managed_namespace() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let middle = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let source = old.path().to_path_buf();
    let opening = resource("workspace", "default-workspace", "workspace_root", source.to_str().unwrap());
    let session = ready_with_resources(&store, vec![opening.clone()], "managed-then-external").await;
    let id = &session.agent_session_id;
    let old_file = source.join("conversations").join(id.as_ref()).join("old.txt").to_string_lossy().into_owned();
    let first = store.append_event(&append(id, &Uuid::now_v7().to_string(), "session-api", "before-external", "message/user-accepted", "before-external", None,
        json!({"content":format!("[[NOMI_FILES]]\n{old_file}"),"files":[old_file]}))).await.unwrap().record.unwrap();
    let external_root = source.join("conversations").join(format!("{}-external", id.as_ref()));
    let external = resource("workspace", "selected-workspace:external", "workspace_root", external_root.to_str().unwrap());
    let mut replacement = agent_replacement(&session.agent_binding, "external-target");
    replacement.typed_resource_bindings = vec![external.clone()];
    store.replace_session_agent_binding(&owner(), id, agent_switch(session.agent_binding.clone(), replacement.clone(), "external-switch")).await.unwrap();
    let external_file = external_root.join("new.txt").to_string_lossy().into_owned();
    let after = store.append_event(&append(id, &Uuid::now_v7().to_string(), "session-api", "after-external", "message/user-accepted", "after-external", None,
        json!({"content":format!("[[NOMI_FILES]]\n{external_file}\n{old_file}"),"files":[external_file,old_file]}))).await.unwrap().record.unwrap();
    let first_move = store.relocate_managed_workspaces(&source, &source, middle.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let second_move = store.relocate_managed_workspaces(middle.path(), middle.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    assert_eq!(first_move.changed_sessions, 1);
    assert_eq!(second_move.changed_sessions, 1);
    assert_eq!(first_move.managed_namespaces, second_move.managed_namespaces);
    assert_eq!(second_move.managed_namespaces[0].opening_workspace, opening);
    let current = store.get_live_session(id).await.unwrap();
    assert_eq!(current.agent_binding.typed_resource_bindings, vec![external]);
    assert_eq!(current.agent_binding.resolved_snapshot_ref, replacement.resolved_snapshot_ref);
    let current_root = new.path().join("conversations");
    let migrated = current_root.join(id.as_ref()).join("old.txt");
    std::fs::create_dir_all(migrated.parent().unwrap()).unwrap(); std::fs::write(&migrated, b"managed history").unwrap();
    old.close().unwrap();
    let paths = store.managed_history_file_paths(id, &current_root, &BTreeMap::from([
        (first.seq, BTreeSet::from([old_file.clone()])), (after.seq, BTreeSet::from([old_file.clone(), external_file.clone()])),
    ])).await.unwrap();
    assert_eq!(std::fs::read(&paths[&first.seq][&old_file]).unwrap(), b"managed history");
    assert!(paths[&after.seq].is_empty(), "external-stage paths are not adopted by historical prefix similarity");
    let events = store.read_events(id, None, 100).await.unwrap().events;
    let relocations = events.into_iter().filter(|event| event.kind.0 == "session/agent-binding-changed")
        .filter_map(|event| match event.payload { nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(value) =>
            serde_json::from_value::<nomifun_agent_contracts::AgentBindingChangedPayloadV1>(value.0).ok(), _ => None })
        .filter_map(|payload| payload.workspace_relocation).collect::<Vec<_>>();
    assert_eq!(relocations.len(), 2);
    assert!(relocations.iter().all(|proof| proof.managed_session_namespace.as_ref().unwrap().opening_workspace == opening));
}

#[tokio::test]
async fn storage_relocation_updates_only_selected_paths_with_exact_opening_session_ownership() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap(); let last = tempfile::tempdir().unwrap();
    let session = ready_with_resources(&store,
        vec![resource("workspace", "default-workspace", "workspace_root", old.path().to_str().unwrap())], "selected-owned-folder").await;
    let id = &session.agent_session_id;
    let exact_root = old.path().join("conversations").join(id.as_ref()).to_string_lossy().into_owned();
    let selected = resource("workspace", "selected-workspace:owned", "workspace_root", &exact_root);
    let process = resource("process_session", "managed-process-session", "workspace_root", &exact_root);
    let mut replacement = agent_replacement(&session.agent_binding, "owned-selected-target");
    replacement.typed_resource_bindings = vec![selected.clone(), process.clone()];
    store.replace_session_agent_binding(&owner(), id, agent_switch(session.agent_binding.clone(), replacement.clone(), "owned-folder-switch")).await.unwrap();
    let outcome = store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    assert_eq!(outcome.managed_namespaces[0].agent_session_id, *id);
    assert_eq!(outcome.managed_namespaces[0].opening_workspace, session.agent_binding.typed_resource_bindings[0]);
    store.validate_workspace_relocation(new.path(), new.path()).await.unwrap();
    store.relocate_managed_workspaces(new.path(), new.path(), last.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let current = store.get_live_session(id).await.unwrap();
    for (before, after) in [selected, process].iter().zip(&current.agent_binding.typed_resource_bindings) {
        assert_eq!(before.resource_id, after.resource_id); assert_eq!(before.operations, after.operations);
        assert_eq!(before.owner_id, after.owner_id); assert_eq!(before.connection_config_ref, after.connection_config_ref);
        assert_ne!(before.binding_id, after.binding_id);
        assert_eq!(after.typed_parameters["workspace_root"], last.path().join("conversations").join(id.as_ref()).to_string_lossy());
    }
    assert_eq!(current.agent_binding.resolved_snapshot_ref, replacement.resolved_snapshot_ref);
}

#[tokio::test]
async fn storage_relocation_moves_only_registered_managed_knowledge() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let managed = Uuid::now_v7().to_string(); let external = Uuid::now_v7().to_string();
    let managed_root = old.path().join("knowledge").join(&managed).to_string_lossy().into_owned();
    let external_root = old.path().join("knowledge").join(&external).to_string_lossy().into_owned();
    for (id, root, flag) in [(&managed, &managed_root, 1_i64), (&external, &external_root, 0_i64)] {
        sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,managed,created_at,updated_at) VALUES(?,'base',?,?,1,1)")
            .bind(id).bind(root).bind(flag).execute(store.test_pool()).await.unwrap();
    }
    let outside = resource("knowledge_base", &external, "knowledge_root", &external_root);
    let session = ready_with_resources(&store, vec![resource("knowledge_base", &managed, "knowledge_root", &managed_root), outside.clone()], "knowledge-relocation").await;
    store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let updated = store.get_live_session(&session.agent_session_id).await.unwrap();
    assert_eq!(updated.agent_binding.typed_resource_bindings[1], outside);
    assert_eq!(updated.agent_binding.typed_resource_bindings[0].typed_parameters["knowledge_root"], new.path().join("knowledge").join(&managed).to_string_lossy());
}

#[tokio::test]
async fn storage_relocation_keeps_large_proofs_in_the_same_canonical_payload_chain() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let mut resources = Vec::new();
    for _ in 0..2 {
        let id = Uuid::now_v7().to_string();
        let root = old.path().join("knowledge").join(&id).to_string_lossy().into_owned();
        sqlx::query("INSERT INTO knowledge_bases(knowledge_base_id,name,root_path,managed,created_at,updated_at) VALUES(?,'base',?,1,1,1)")
            .bind(&id).bind(&root).execute(store.test_pool()).await.unwrap();
        let mut frozen = resource("knowledge_base", &id, "knowledge_root", &root);
        frozen.typed_parameters.insert("retained_policy".into(), "x".repeat(18_000));
        frozen.binding_id = resource_definition_id(&frozen).unwrap();
        resources.push(frozen);
    }
    let session = ready_with_resources(&store, resources, "large-storage-proof").await;
    let operation = OperationId::from(Uuid::now_v7().to_string());
    let outcome = store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &operation).await.unwrap();
    assert_eq!(outcome.changed_sessions, 1);
    assert!(outcome.managed_namespaces.is_empty());
    let stored: bool = sqlx::query_scalar("SELECT payload_id IS NOT NULL FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert!(stored);
    let replay = store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &operation).await.unwrap();
    assert_eq!(replay.changed_sessions, 0);
    assert!(replay.managed_namespaces.is_empty());
}

#[tokio::test]
async fn storage_relocation_rejects_unproven_roots_and_rolls_back_the_whole_dataset() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let good = ready_with_resources(&store, vec![resource("workspace", "default-workspace", "workspace_root", old.path().to_str().unwrap())], "good-relocation").await;
    ready_with_resources(&store, vec![resource("workspace", "default-workspace", "workspace_root", new.path().to_str().unwrap())], "bad-relocation").await;
    assert!(store.validate_workspace_relocation(old.path(), old.path()).await.is_err());
    assert!(store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.is_err());
    assert_eq!(store.get_live_session(&good.agent_session_id).await.unwrap().agent_binding, good.agent_binding);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind='session/agent-binding-changed'").fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn storage_relocation_rejects_active_turns_and_pending_patch_recovery() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap();
    create_turn(&store, "relocation-blocker", "active-relocation-turn").await;
    assert!(store.validate_workspace_relocation(old.path(), old.path()).await.is_err());
    let second = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, started) = create_turn(&second, "relocation-patch", "patch-operation").await;
    second.append_event(&append(&session.agent_session_id, "patch-relocation-event", "runtime-supervisor", "patch-relocation-key",
        "runtime/progress-recorded", "patch-operation", Some(started), json!({"event":{"event":"patch_recovery_updated", "state":{
            "version":2,"targets":["a.txt"],"unresolved_targets":[],"unresolved_before_input":null,"target_budget_exceeded":false
        }}}))).await.unwrap();
    second.append_event(&append(&session.agent_session_id, "patch-relocation-terminal", "runtime-supervisor", "patch-relocation-terminal-key",
        "turn/completed", "patch-operation", Some(EventId::from("patch-relocation-event")), json!({}))).await.unwrap();
    assert!(second.validate_workspace_relocation(old.path(), old.path()).await.is_err());
}

#[tokio::test]
async fn storage_relocation_preserves_failed_terminal_checkpoints_as_history_without_resume_authority() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let (session, started) = create_turn(&store, "failed-relocation", "failed-relocation-turn").await;
    let state = StrictJsonValue(json!({"version":1,"model_steps":7,"plan":{"revision":7}}));
    let digest = digest_bytes(&canonical_json_bytes(&state.0).unwrap());
    let event = append(&session.agent_session_id, "failed-relocation-checkpoint", "runtime_supervisor",
        "failed-relocation-checkpoint", "runtime/progress-recorded", "failed-relocation-turn", Some(started.clone()),
        json!({"producer_seq":1,"event":{"event":"execution_checkpoint_saved","step":7,"revision":1,"digest":digest}}));
    let saved = store.save_native_checkpoint(&event, crate::NativeCheckpointWrite {
        owner: owner(), operation_id: "failed-relocation-turn".into(),
        snapshot: session.agent_binding.resolved_snapshot_ref.clone(), active_set_generation: 0,
        expected_revision: 0, execution_fence: 0, lease: None, state,
    }).await.unwrap();
    store.append_event(&append(&session.agent_session_id, "failed-relocation-terminal", "runtime-supervisor",
        "failed-relocation-terminal", "turn/failed", "failed-relocation-turn", Some(started), json!({"reason":"model failure"}))).await.unwrap();
    store.validate_workspace_relocation(old.path(), old.path()).await.unwrap();
    store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let retained = store.load_native_checkpoint(&owner(), &session.agent_session_id, &saved.operation_id).await.unwrap().unwrap();
    assert_eq!(retained.turn_state, "failed");
    assert_eq!(retained.state, saved.state);
    assert_eq!(retained.digest, saved.digest);
}

async fn validate_terminal_native_storage(events: Vec<Value>) -> Result<(), SessionStoreError> {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let (session, started) = create_turn(&store, "native-relocation", "native-relocation-turn").await;
    for (index, event) in events.into_iter().enumerate() {
        let identity = format!("native-relocation-progress-{index}");
        store.append_event(&append(&session.agent_session_id, &identity, "runtime-supervisor", &identity,
            "runtime/progress-recorded", "native-relocation-turn", Some(started.clone()), json!({"event":event}))).await.unwrap();
    }
    store.append_event(&append(&session.agent_session_id, "native-relocation-completed", "runtime-supervisor",
        "native-relocation-completed", "turn/completed", "native-relocation-turn", Some(started), json!({}))).await.unwrap();
    store.validate_workspace_relocation(root.path(), root.path()).await
}

#[tokio::test]
async fn storage_relocation_requires_exact_native_process_exit_receipts_even_after_terminal() {
    let dispatch = json!({"event":"host_process_dispatch","operation_id":"process-one","ordinal":1});
    assert!(validate_terminal_native_storage(vec![dispatch.clone()]).await.is_err());
    assert!(validate_terminal_native_storage(vec![dispatch.clone(),
        json!({"event":"host_process_quiescent","operation_id":"process-one","ordinal":1,"process_count":1})]).await.is_ok());
    assert!(validate_terminal_native_storage(vec![dispatch.clone(),
        json!({"event":"host_process_quiescent","operation_id":"different-operation","ordinal":1,"process_count":1})]).await.is_err());
    assert!(validate_terminal_native_storage(vec![dispatch,
        json!({"event":"host_process_dispatch","operation_id":"process-two","ordinal":2}),
        json!({"event":"host_process_quiescent","operation_id":"process-two","ordinal":2,"process_count":2})]).await.is_ok());
}

#[tokio::test]
async fn storage_relocation_requires_owner_settlement_or_cleanup_for_native_invocations() {
    let tool = json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"tool-one"}});
    let resource = json!({"event":"host_resource_dispatch","operation_id":"resource-one"});
    let cleanup = json!({"event":"host_cleanup_proven"});
    assert!(validate_terminal_native_storage(vec![tool.clone()]).await.is_err());
    assert!(validate_terminal_native_storage(vec![tool.clone(),
        json!({"event":"host_tool_settled","operation_id":"tool-one"})]).await.is_ok());
    assert!(validate_terminal_native_storage(vec![tool.clone(), resource, cleanup.clone()]).await.is_ok());
    assert!(validate_terminal_native_storage(vec![cleanup, tool]).await.is_err());
}

async fn historical_file_receipt(store: &AgentSessionStore, session: &AgentSessionLiveRecord,
    workspace: &str, key: &str) -> SessionEventRecord {
    let turn = OperationId::from(format!("file-history:{key}"));
    let (_, started) = store.start_turn(&session.agent_session_id, "session_api".into(),
        key.into(), turn.clone(), StrictJsonValue(json!({"content":"write a report"}))).await.unwrap();
    let started = started.record.unwrap();
    let tool = store.append_event(&append(&session.agent_session_id, &format!("file-tool:{key}"),
        "runtime-supervisor", &format!("file-tool:{key}"), "tool/call-started", key,
        Some(started.event_id.clone()), json!({"operation_id":key,"call_id":key,
            "capability_id":"workspace.files","action_id":"workspace.files/write"}))).await.unwrap().ack.unwrap();
    let request = EffectEventRequest {
        agent_session_id:session.agent_session_id.clone(), effect_id:format!("file-effect:{key}"),
        turn_id:turn.clone(), operation_id:key.into(), owner_domain:"workspace".into(),
        capability_module:"workspace.files".into(), action_id:"workspace.files/write".into(),
        resource_binding_id:Some(session.agent_binding.typed_resource_bindings[0].binding_id.clone()),
        resource_key:Some(format!("workspace:{key}")), input_digest:digest('7'), recorded_at:1000,
        event_id:format!("file-effect-start:{key}").into(), producer_id:"capability_host".into(),
        idempotency_key:format!("file-effect:{key}").into(), correlation_id:format!("file-effect:{key}").into(),
        strategy:EffectStrategy::ManagedEffect, causation_event_id:Some(tool.event_id),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    let effect = store.record_effect_started(request.clone()).await.unwrap().ack.unwrap();
    let receipt = store.record_effect_terminal(EffectEventRequest {
        event_id:format!("file-effect-return:{key}").into(), producer_id:"owning_plugin".into(),
        causation_event_id:Some(effect.event_id), recorded_at:1001,
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"result":{
            "path":"report.docx","written":true,"created":true,"bytes":42,"sha256":"d".repeat(64),
            "workspace_path":{"root_sha256":digest_bytes(workspace.as_bytes()),"path":"report.docx","case_resolved":false},
        }}))), ..request
    }, EffectTerminalState::Succeeded).await.unwrap().record.unwrap();
    store.append_event(&append(&session.agent_session_id, &format!("file-turn-end:{key}"),
        "runtime-supervisor", &format!("file-turn-end:{key}"), "turn/completed", turn.as_ref(),
        Some(started.event_id), json!({}))).await.unwrap();
    receipt
}

#[tokio::test]
async fn storage_relocation_resolves_pre_observation_file_receipts_without_old_disk_or_history_rewrite() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let old_root = old.path().to_path_buf();
    let session = ready_with_resources(&store, vec![resource("workspace", "default-workspace",
        "workspace_root", old_root.to_str().unwrap())], "old-file-receipt").await;
    let source = old_root.join("conversations").join(session.agent_session_id.as_ref()).to_string_lossy().into_owned();
    let receipt = historical_file_receipt(&store, &session, &source, "before-relocation").await;
    let retained = receipt.payload.clone();
    store.relocate_managed_workspaces(&old_root, &old_root, new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    old.close().unwrap();
    assert!(!old_root.exists());
    let hash = digest_bytes(source.as_bytes()).as_ref().to_owned();
    let candidates = BTreeMap::from([(receipt.seq, BTreeSet::from([hash.clone(), "f".repeat(64)]))]);
    let current = new.path().join("conversations");
    let roots = store.history_workspace_roots(&session.agent_session_id, &current, &candidates).await.unwrap();
    assert_eq!(roots[&receipt.seq][&hash], current.join(session.agent_session_id.as_ref()).to_string_lossy());
    assert!(!roots[&receipt.seq].contains_key(&"f".repeat(64)));
    let original = store.read_events(&session.agent_session_id, None, 100).await.unwrap().events.into_iter()
        .find(|event| event.event_id == receipt.event_id).unwrap();
    assert_eq!(original.payload, retained);
    assert!(store.history_workspace_roots(&session.agent_session_id, &old_root.join("conversations"), &candidates).await.is_err());
}

#[tokio::test]
async fn history_file_root_uses_original_effect_resource_after_external_workspace_selection_changes() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let original = tempfile::tempdir().unwrap(); let later = tempfile::tempdir().unwrap();
    let original_root = std::fs::canonicalize(original.path()).unwrap().to_string_lossy().into_owned();
    let session = ready_with_resources(&store, vec![resource("workspace", "selected-workspace:original",
        "workspace_root", &original_root)], "external-file-receipt").await;
    let receipt = historical_file_receipt(&store, &session, &original_root, "external-before-switch").await;
    let mut replacement = session.agent_binding.clone();
    replacement.binding_version += 1;
    replacement.typed_resource_bindings = vec![resource("workspace", "selected-workspace:later",
        "workspace_root", later.path().to_str().unwrap())];
    store.replace_session_resource_bindings(&owner(), &session.agent_session_id, &session.agent_binding,
        replacement, "workspace").await.unwrap();
    original.close().unwrap();
    let hash = digest_bytes(original_root.as_bytes()).as_ref().to_owned();
    let candidates = BTreeMap::from([(receipt.seq, BTreeSet::from([hash.clone()]))]);
    let roots = store.history_workspace_roots(&session.agent_session_id, &later.path().join("conversations"), &candidates).await.unwrap();
    assert_eq!(roots[&receipt.seq][&hash], original_root);
}

#[tokio::test]
async fn storage_relocation_resolves_native_observation_roots_with_the_same_proof_chain() {
    use nomifun_agent_contracts::{ObservedFileOperation, ObservedWorkspaceFile, TurnWorkspaceFilesObservedV1, TURN_WORKSPACE_FILES_OBSERVED};
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let old = tempfile::tempdir().unwrap(); let new = tempfile::tempdir().unwrap();
    let session = ready_with_resources(&store, vec![resource("workspace", "default-workspace",
        "workspace_root", old.path().to_str().unwrap())], "new-observation-relocation").await;
    let root = old.path().join("conversations").join(session.agent_session_id.as_ref()).to_string_lossy().into_owned();
    let operation = OperationId::from("observed-before-relocation");
    let (_, started) = store.start_turn(&session.agent_session_id, "session_api".into(), operation.as_ref().into(),
        operation.clone(), StrictJsonValue(json!({"content":"create a document","admission":{
            "route_identity":ChatRouteIdentity::new("coding.codex@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "chat-route".into(),4),
            "resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref,
        }}))).await.unwrap();
    let started = started.record.unwrap();
    let lease = store.claim_native_execution(crate::NativeExecutionClaim {
        owner:owner(), agent_session_id:session.agent_session_id.clone(), operation_id:operation.clone(),
        snapshot:session.agent_binding.resolved_snapshot_ref.clone(), active_set_generation:0,
        holder:"file-observer".into(), expected_fence:0, checkpoint:None,
    }).await.unwrap();
    let observation = append(&session.agent_session_id, "observed-workspace", "runtime_supervisor", "observed-workspace",
        TURN_WORKSPACE_FILES_OBSERVED, operation.as_ref(), Some(started.event_id.clone()),
        serde_json::to_value(TurnWorkspaceFilesObservedV1 {
            operation_id:operation.clone(), execution_generation:lease.generation(),
            workspace_root:root.clone(), workspace_root_sha256:digest_bytes(root.as_bytes()).as_ref().into(),
            complete:true, unchanged_paths:vec![], files:vec![ObservedWorkspaceFile {
                path:"report.docx".into(), operation:ObservedFileOperation::Create, size_bytes:Some(42), sha256:Some("d".repeat(64)), source_sha256:None,
            }],
        }).unwrap());
    let observed = store.append_native_observation(&lease, &observation, None).await.unwrap().record.unwrap();
    store.append_native_event(&lease, &append(&session.agent_session_id, "observed-cleanup", "runtime_supervisor", "observed-cleanup",
        "runtime/progress-recorded", operation.as_ref(), Some(started.event_id.clone()),
        json!({"producer_seq":1,"event":{"event":"host_cleanup_proven"}})), None).await.unwrap();
    let completion = append(&session.agent_session_id, "observed-completion", "runtime_supervisor", "observed-completion",
        "message/completed", "observed-assistant", Some(started.event_id.clone()), json!({"part_count":0,"content_digest":digest_bytes(b"")}));
    let terminal = append(&session.agent_session_id, "observed-terminal", "runtime_supervisor", "observed-terminal",
        "turn/completed", operation.as_ref(), Some(started.event_id), json!({}));
    store.append_native_chat_completion(&lease, &completion, &terminal).await.unwrap();
    store.relocate_managed_workspaces(old.path(), old.path(), new.path(), &OperationId::from(Uuid::now_v7().to_string())).await.unwrap();
    let candidates = BTreeMap::from([(observed.seq, BTreeSet::from([digest_bytes(root.as_bytes()).as_ref().to_owned()]))]);
    let current = new.path().join("conversations");
    let resolved = store.history_workspace_roots(&session.agent_session_id, &current, &candidates).await.unwrap();
    assert_eq!(resolved[&observed.seq].values().next().unwrap(), &current.join(session.agent_session_id.as_ref()).to_string_lossy());
}
