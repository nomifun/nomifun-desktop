use super::*;
use nomifun_agent_contracts::{CompanionSessionOrigin, CompanionSessionRole};

fn origin(role: CompanionSessionRole) -> CompanionSessionOrigin {
    CompanionSessionOrigin {
        companion_id: Uuid::now_v7().to_string(),
        role,
        robot_id: (role == CompanionSessionRole::Robot).then(|| "robot-board-1".into()),
    }
}

#[tokio::test]
async fn companion_scope_is_read_from_the_opening_and_survives_mutable_metadata() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let mut session = live_session(session_id());
    let scope = origin(CompanionSessionRole::Robot);
    session.metadata.companion = Some(scope.clone());
    let request = create_request(session.clone(), "robot-scope");
    store.create_session(request.clone()).await.unwrap();
    let before = store.read_events(&session.agent_session_id, None, 100).await.unwrap().events;
    let updated = store.update_session_metadata(&owner(), &session.agent_session_id, crate::UpdateAgentSessionMetadata {
        title: Some("Renamed robot chat".into()), archived: Some(true), pinned: Some(true),
    }).await.unwrap();
    assert_eq!(updated.metadata.companion.as_ref(), Some(&scope));
    store.update_session_reasoning_effort(&owner(), &session.agent_session_id, Some(ReasoningEffort::High)).await.unwrap();
    assert_eq!(store.companion_origin(&owner(), &session.agent_session_id).await.unwrap().as_ref(), Some(&scope));
    assert_eq!(store.read_events(&session.agent_session_id, None, 100).await.unwrap().events, before);
    let mut another_device = request;
    another_device.session.metadata.companion.as_mut().unwrap().robot_id = Some("robot-board-2".into());
    assert!(matches!(store.create_session(another_device).await, Err(SessionStoreError::IdempotencyConflict(_))));
    let foreign = PrincipalRef { principal_kind: "user".into(), principal_id: "foreign".into() };
    assert!(store.companion_origin(&foreign, &session.agent_session_id).await.is_err());
    let command = DeleteAgentSessionCommand { operation_id: "robot-scope-delete".into(),
        agent_session_id: session.agent_session_id.clone(), owner_ref: owner(), requested_at: 2_000_000_000_000 };
    store.fence_delete(&command).await.unwrap();
    assert_eq!(store.companion_origin(&owner(), &session.agent_session_id).await.unwrap().as_ref(), Some(&scope));
    store.complete_delete(&command, command.requested_at + 1).await.unwrap();
    assert!(store.companion_origin(&owner(), &session.agent_session_id).await.unwrap().is_none());
}

#[tokio::test]
async fn companion_scope_rejects_invalid_roles_ids_and_conflicting_product_ownership() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    for defect in ["companion-id", "v4", "uppercase", "companion-device", "robot-missing", "robot-empty", "robot-padding", "app", "purpose", "remote"] {
        let mut session = live_session(session_id());
        let mut scope = origin(CompanionSessionRole::Robot);
        match defect {
            "companion-id" => scope.companion_id = "companion".into(),
            "v4" => scope.companion_id = "00000000-0000-4000-8000-000000000000".into(),
            "uppercase" => scope.companion_id = "019ABCAB-CDEF-7000-8000-000000000000".into(),
            "companion-device" => scope.role = CompanionSessionRole::Companion,
            "robot-missing" => scope.robot_id = None,
            "robot-empty" => scope.robot_id = Some(" ".into()),
            "robot-padding" => scope.robot_id = Some(" robot-board-1".into()),
            "app" => session.metadata.app_project_id = Some(Uuid::now_v7().to_string()),
            "purpose" => session.metadata.purpose = nomifun_agent_contracts::SessionPurpose::PluginAuthoring,
            "remote" => session.remote_binding_provenance = Some(RemoteBindingProvenance {
                remote_binding_id: "remote-1".into(), binding_version: 1,
            }),
            _ => unreachable!(),
        }
        session.metadata.companion = Some(scope);
        assert!(store.create_session(create_request(session, defect)).await.is_err(), "accepted {defect}");
    }
    let mut session = live_session(session_id());
    session.metadata.companion = Some(origin(CompanionSessionRole::Companion));
    store.create_session(create_request(session.clone(), "desktop-scope")).await.unwrap();
    assert_eq!(store.get_live_session(&session.agent_session_id).await.unwrap().metadata.companion, session.metadata.companion);
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.metadata.companion.companion_id','invalid') WHERE session_id=? AND seq=1")
        .bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.companion_origin(&owner(), &session.agent_session_id).await.is_err());
}

#[tokio::test]
async fn companion_scope_never_infers_product_origin_from_resources_or_opening_identity() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let mut session = live_session(session_id());
    let companion = Uuid::now_v7().to_string();
    session.agent_binding.typed_resource_bindings.push(TypedResourceBinding {
        binding_id: "companion-resource".into(), resource_kind: "companion".into(),
        resource_id: companion.clone().into(), owner_id: owner().principal_id,
        operations: BTreeSet::new(), connection_config_ref: None, typed_parameters: BTreeMap::new(),
    });
    store.create_session(create_request(session.clone(), &format!("companion-session:{companion}"))).await.unwrap();
    assert!(store.companion_origin(&owner(), &session.agent_session_id).await.unwrap().is_none());
}
