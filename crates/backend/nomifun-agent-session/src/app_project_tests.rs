use super::*;

fn app_session_request(app: &str, explicit: bool, suffix: &str) -> CreateSessionRequest {
    let mut session = live_session(session_id());
    session.metadata.app_project_id = explicit.then(|| app.to_owned());
    session.agent_binding.typed_resource_bindings.push(TypedResourceBinding {
        binding_id: ResourceBindingId::from(format!("workspace-{suffix}")),
        resource_kind: ResourceKind::from("workspace"),
        resource_id: ResourceId::from(format!("selected-workspace-{}", "a".repeat(64))),
        owner_id: owner().principal_id,
        operations: BTreeSet::new(), connection_config_ref: None,
        typed_parameters: BTreeMap::from([("workspace_root".into(), format!("C:/isolated/data/app-hosting/apps/{app}"))]),
    });
    let key = format!("user:{}:open:app-development:{app}:{suffix}", owner().principal_id);
    let mut request = create_request(session.clone(), &key);
    request.operation_id = format!("open:{key}").into();
    request.producer_id = "session_api".into();
    request.correlation_id = format!("open:{key}").into();
    request.opening_event_id = Some(format!("opening:{key}").into());
    request
}

async fn app_session(store: &AgentSessionStore, app: &str, explicit: bool, suffix: &str)
    -> AgentSessionLiveRecord {
    let request = app_session_request(app, explicit, suffix);
    let id = request.session.agent_session_id.clone();
    store.create_session(request).await.unwrap();
    store.get_live_session(&id).await.unwrap()
}

#[tokio::test]
async fn app_project_scope_filters_before_pagination_and_protects_deleting_members() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let app = Uuid::now_v7().to_string();
    let other_app = Uuid::now_v7().to_string();
    let first = app_session(&store, &app, true, "initial").await;
    let second = app_session(&store, &app, true, &format!("new:{}", Uuid::now_v7())).await;
    let foreign_app = app_session(&store, &other_app, true, "initial").await;
    let mut ordinary = live_session(session_id());
    ordinary.agent_binding = first.agent_binding.clone();
    store.create_session(create_request(ordinary.clone(), "same-root-ordinary")).await.unwrap();
    let ordinary_page = store.list_live_ordinary_conversation_sessions(&owner(), None, 1).await.unwrap();
    assert_eq!(ordinary_page.total, 1);
    assert!(!ordinary_page.has_more);
    assert_eq!(ordinary_page.items[0].session.agent_session_id, ordinary.agent_session_id);
    let page = store.list_live_app_project_sessions(&owner(), &app, None, 1).await.unwrap();
    assert_eq!(page.total, 2);
    assert!(page.has_more);
    let next = store.list_live_app_project_sessions(&owner(), &app, page.next_cursor.as_deref(), 1).await.unwrap();
    assert_eq!(next.total, 2);
    assert!(!next.has_more);
    assert_ne!(page.items[0].session.agent_session_id, next.items[0].session.agent_session_id);
    store.update_session_metadata(&owner(), &first.agent_session_id, crate::UpdateAgentSessionMetadata {
        title: None, archived: Some(true), pinned: None,
    }).await.unwrap();
    assert_eq!(store.app_project_id(&owner(), &first.agent_session_id).await.unwrap().as_deref(), Some(app.as_str()));
    let command = DeleteAgentSessionCommand { operation_id: "app-member-delete".into(),
        agent_session_id: second.agent_session_id.clone(), owner_ref: owner(), requested_at: 2_000_000_000_000 };
    store.fence_delete(&command).await.unwrap();
    assert_eq!(store.list_live_app_project_sessions(&owner(), &app, None, 10).await.unwrap().total, 1);
    let members = store.list_app_project_session_members(&owner(), &app).await.unwrap();
    assert!(members.contains(&first.agent_session_id) && members.contains(&second.agent_session_id));
    assert!(!members.contains(&ordinary.agent_session_id) && !members.contains(&foreign_app.agent_session_id));
    assert_eq!(store.app_project_id(&owner(), &second.agent_session_id).await.unwrap().as_deref(), Some(app.as_str()));
    store.complete_delete(&command, command.requested_at + 1).await.unwrap();
    assert_eq!(store.list_app_project_session_members(&owner(), &app).await.unwrap(), vec![first.agent_session_id]);
    assert!(store.app_project_id(&owner(), &second.agent_session_id).await.unwrap().is_none());
}

#[tokio::test]
async fn app_project_scope_recognizes_only_exact_current_generation_opening_receipts_without_writes() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let app = Uuid::now_v7().to_string();
    for suffix in ["initial".to_owned(), Uuid::now_v7().to_string(), format!("new:{}", Uuid::now_v7())] {
        let created = app_session(&store, &app, false, &suffix).await;
        assert_eq!(created.metadata.app_project_id.as_deref(), Some(app.as_str()));
        let before = store.read_events(&created.agent_session_id, None, 100).await.unwrap().events;
        store.list_live_app_project_sessions(&owner(), &app, None, 100).await.unwrap();
        assert_eq!(store.read_events(&created.agent_session_id, None, 100).await.unwrap().events, before);
        let mut new_binding = created.agent_binding.clone();
        new_binding.typed_resource_bindings.clear();
        new_binding.binding_version += 1;
        store.replace_session_resource_bindings(&owner(), &created.agent_session_id,
            &created.agent_binding, new_binding, "workspace").await.unwrap();
        assert_eq!(store.app_project_id(&owner(), &created.agent_session_id).await.unwrap().as_deref(), Some(app.as_str()),
            "scope reads the original opening rather than the current workspace");
    }
    for defect in ["producer", "key", "event", "correlation", "operation", "parent", "root", "workspace_identity", "workspace_owner"] {
        let suffix = format!("new:{}", Uuid::now_v7());
        let created = if defect == "event" {
            let mut request = app_session_request(&app, false, &suffix);
            request.opening_event_id = Some(format!("wrong:{}", Uuid::now_v7()).into());
            let id = request.session.agent_session_id.clone();
            store.create_session(request).await.unwrap();
            store.get_live_session(&id).await.unwrap()
        } else { app_session(&store, &app, false, &suffix).await };
        let id = &created.agent_session_id;
        let (field, replacement) = match defect {
            "producer" => ("producer_id", "manual_client".to_owned()),
            "key" => ("idempotency_key", format!("user:{}:open:app-development:{app}:new:not-a-uuid", owner().principal_id)),
            "event" => ("already_changed", String::new()),
            "correlation" => ("correlation_id", "wrong-open".to_owned()),
            _ => ("", String::new()),
        };
        if field == "already_changed" {
            // The wrong identity was committed by the fixture creator, keeping
            // native head foreign keys intact while testing the receipt shape.
        } else if !field.is_empty() {
            sqlx::query(&format!("UPDATE agent_events SET {field}=? WHERE session_id=? AND seq=1"))
                .bind(replacement).bind(id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else {
            let (path, value) = match defect {
                "operation" => ("$.operation_id", "wrong-open".to_owned()),
                "parent" => ("$.parent_session_id", Uuid::now_v7().to_string()),
                "root" => ("$.agent_binding.typed_resource_bindings[0].typed_parameters.workspace_root", format!("C:/ordinary/{app}")),
                "workspace_identity" => ("$.agent_binding.typed_resource_bindings[0].resource_id", "default-workspace".to_owned()),
                _ => ("$.agent_binding.typed_resource_bindings[0].owner_id", "another-owner".to_owned()),
            };
            sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,?,?) WHERE session_id=? AND seq=1")
                .bind(path).bind(value).bind(id.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        assert!(store.app_project_id(&owner(), id).await.unwrap().is_none(), "{defect} is not app ownership");
        assert!(store.list_live_ordinary_conversation_sessions(&owner(), None, 100).await.unwrap()
            .items.iter().any(|item| item.session.agent_session_id == *id), "SQL and typed reader disagree for {defect}");
    }
}

#[tokio::test]
async fn app_project_scope_null_field_retains_exact_current_generation_receipt_semantics() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let app = Uuid::now_v7().to_string();
    let created = app_session(&store, &app, false, "initial").await;
    // A legal optional null has the same meaning as field absence. Only the
    // complete earlier canonical app creation receipt supplies affiliation.
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.metadata.app_project_id',null) WHERE session_id=? AND seq=1")
        .bind(created.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    let before = store.read_events(&created.agent_session_id, None, 100).await.unwrap().events;
    assert_eq!(store.app_project_id(&owner(), &created.agent_session_id).await.unwrap().as_deref(), Some(app.as_str()));
    assert_eq!(store.list_live_ordinary_conversation_sessions(&owner(), None, 10).await.unwrap().total, 0);
    assert_eq!(store.list_live_app_project_sessions(&owner(), &app, None, 10).await.unwrap().total, 1);
    assert_eq!(store.list_app_project_session_members(&owner(), &app).await.unwrap(), vec![created.agent_session_id.clone()]);
    assert_eq!(store.read_events(&created.agent_session_id, None, 100).await.unwrap().events, before);
}

#[tokio::test]
async fn app_project_scope_rejects_noncanonical_ids_and_other_owners() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    for invalid in ["app", "00000000-0000-4000-8000-000000000000", "019ABCAB-CDEF-7000-8000-000000000000"] {
        let mut session = live_session(session_id());
        session.metadata.app_project_id = Some(invalid.into());
        assert!(store.create_session(create_request(session, invalid)).await.is_err());
        assert!(store.list_live_app_project_sessions(&owner(), invalid, None, 10).await.is_err());
    }
    let app = Uuid::now_v7().to_string();
    let session = app_session(&store, &app, true, "initial").await;
    let foreign = PrincipalRef { principal_kind: "user".into(), principal_id: "foreign".into() };
    assert!(store.app_project_id(&foreign, &session.agent_session_id).await.is_err());
    assert!(store.list_app_project_session_members(&foreign, &app).await.unwrap().is_empty());
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.metadata.app_project_id','invalid') WHERE session_id=? AND seq=1")
        .bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.app_project_id(&owner(), &session.agent_session_id).await.is_err());
    assert!(store.list_live_ordinary_conversation_sessions(&owner(), None, 10).await.is_err(),
        "malformed scope cannot hide unreadable canonical facts behind an unknown application ID");
}
