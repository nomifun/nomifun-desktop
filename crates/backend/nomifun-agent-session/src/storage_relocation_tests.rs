use super::*;
use std::path::Path;
use serde_json::Value;
use nomifun_agent_contracts::{managed_workspace_parameters, resource_definition_id};

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
