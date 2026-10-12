use super::*;
use nomifun_agent_contracts::{ObservedFileOperation, ObservedWorkspaceFile, TurnWorkspaceFilesObservedV1, TURN_WORKSPACE_FILES_OBSERVED};

fn observation(f: &Fixture, lease: &NativeExecutionLease, key: &str) -> SessionEventAppend {
    let root = std::env::temp_dir().to_string_lossy().into_owned();
    append(&f.session.agent_session_id, key, "runtime_supervisor", key, TURN_WORKSPACE_FILES_OBSERVED,
        "lease-turn", Some(f.started.clone()), serde_json::to_value(TurnWorkspaceFilesObservedV1 {
            operation_id: "lease-turn".into(), execution_generation: lease.generation(),
            workspace_root_sha256: digest_bytes(root.as_bytes()).as_ref().into(), workspace_root: root,
            complete: true, unchanged_paths: vec![], files: vec![ObservedWorkspaceFile {
                path: "骆驼祥子读后感.docx".into(), operation: ObservedFileOperation::Create,
                size_bytes: Some(2048), sha256: Some("a".repeat(64)), source_sha256: None,
            }],
        }).unwrap())
}

#[tokio::test]
async fn workspace_file_observation_is_persistent_but_not_model_history_or_checkpoint_budget() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "file-observation").await;
    let lease = store.claim_native_execution(claim(&f, "file-owner", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &lease).await;
    let mut event = observation(&f, &lease, "workspace-observed");
    let SessionEventPayloadRef::InlineJson(value) = event.semantic_event.payload.clone() else { unreachable!() };
    let bytes = canonical_json_bytes(&value.0).unwrap();
    let stored = SessionPayloadRecord {
        agent_session_id: f.session.agent_session_id.clone(), payload_id: ArtifactId::from("file-observation-payload"),
        media_type: "application/json".into(), byte_len: bytes.len() as u64, digest: digest_bytes(&bytes),
        body: SessionPayloadBody::Json(value),
    };
    event.semantic_event.payload = SessionEventPayloadRef::Stored(stored.payload_id.clone());
    let budget = store.native_payload_bytes(&lease).await.unwrap();
    store.append_native_observation(&lease, &event, Some(&stored)).await.unwrap();
    assert_eq!(store.native_payload_bytes(&lease).await.unwrap(), budget);
    let facts = store.turn_output_facts(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap();
    assert!(facts.event_payloads.contains_key(event.event_id.as_ref()));
    for facts in [store.chat_causality_facts(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap(),
        store.native_recovery_facts(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap()] {
        assert!(!facts.events.iter().any(|row| row.kind.0 == TURN_WORKSPACE_FILES_OBSERVED));
        assert!(!facts.event_payloads.contains_key(event.event_id.as_ref()));
    }
    assert_eq!(store.load_native_checkpoint(&owner(), &f.session.agent_session_id, &"lease-turn".into()).await.unwrap(), Some(cp));
    store.rebuild_projections(&f.session.agent_session_id).await.unwrap();
    assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status, "running");
    assert!(store.turn_file_facts_by_root(&f.session.agent_session_id, &f.root).await.unwrap().is_none());
}

#[tokio::test]
async fn workspace_observation_requires_exact_native_turn_generation_and_cause() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "file-observation-fence").await;
    let lease = store.claim_native_execution(claim(&f, "file-owner", 0, None)).await.unwrap();
    let valid = observation(&f, &lease, "workspace-valid");
    assert!(store.append_event(&valid).await.is_err(), "unleased entry cannot publish observations");
    let mut wrong = observation(&f, &lease, "workspace-wrong-cause");
    wrong.semantic_event.causation_event_id = Some(f.root.clone());
    assert!(store.append_native_observation(&lease, &wrong, None).await.is_err());
    let mut wrong = observation(&f, &lease, "workspace-wrong-generation");
    let SessionEventPayloadRef::InlineJson(value) = &mut wrong.semantic_event.payload else { unreachable!() };
    value.0["execution_generation"] = json!(lease.generation() + 1);
    assert!(store.append_native_observation(&lease, &wrong, None).await.is_err());
    let mut wrong = observation(&f, &lease, "workspace-traversal");
    let SessionEventPayloadRef::InlineJson(value) = &mut wrong.semantic_event.payload else { unreachable!() };
    value.0["files"][0]["path"] = json!("../foreign.docx");
    assert!(store.append_native_observation(&lease, &wrong, None).await.is_err());
    store.append_native_observation(&lease, &valid, None).await.unwrap();
    let (foreign, _) = create_ready(&store, "file-foreign-session").await;
    assert!(matches!(store.turn_file_facts_by_root(&foreign.agent_session_id, &f.root).await,
        Err(SessionStoreError::NotFound(_))));
}
