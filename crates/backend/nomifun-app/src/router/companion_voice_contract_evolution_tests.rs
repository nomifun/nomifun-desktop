//! Recording onset prepares the ordinary current-generation bundled contract
//! before freezing its input fence. The following real backend dispatch must
//! retain that exact binding and deliver its canonical public answer.
use super::*;

async fn empty_official_physical_entry(flow:&PhysicalEntryFixture)->AgentSessionId {
    let projection=flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    let model=serde_json::to_value(projection.agent_snapshot.as_ref().unwrap().resolved_model.as_ref().unwrap()).unwrap();
    let editor=api(&flow.fixture._router,"/api/agent-presets/from-template/chat.minimal",json!({
        "reuse_existing":false,"display_name":"Ordinary bundled onset predecessor","model":model,
    })).await;
    let preset=editor["preset"]["preset_id"].as_str().unwrap();
    let mut draft=editor["draft"].clone();
    draft["document"]["enabled_capabilities"]=json!([]);
    draft["document"]["skill_bindings"]=json!([]);
    let _:nomifun_api_types::AgentPresetDraftDto=serde_json::from_value(draft.clone())
        .expect("the exact-empty predecessor must use the formal authoring DTO");
    api(&flow.fixture._router,&format!("/api/agent-presets/{preset}/revisions"),json!({
        "expected_current_revision":draft["current_revision"],"draft":draft,
        "reason":"bootstrap a readable current-generation exact-empty bundled contract",
    })).await;
    // Read the author's immutable exact-empty binding before ordinary Session
    // creation resolves today's Skill library and discovery infrastructure.
    // The latter is a different contract and cannot represent this predecessor.
    let owner=nomifun_agent_contracts::UserId::from(flow.owner.clone());
    let authored=flow.fixture.voice_control.resolve_companion_session_binding_with_model(&owner,preset,None).await.unwrap();
    let (authored_binding,authored_revision,authored_snapshot)=flow.fixture.voice_control.saved_binding_artifacts(&owner,&authored).await.unwrap();
    assert!(authored_revision.payload.enabled_capabilities.is_empty());
    assert!(authored_revision.payload.skill_bindings.is_empty());
    assert!(authored_snapshot.content.enabled_capabilities.is_empty(),
        "the known retired official predecessor must actually be exact-empty, not a current Session with injected discovery");
    let store=flow.fixture.owner.canonical().store();
    let modern=store.get_live_session(&flow.robot_id).await.unwrap();
    let mut predecessor=modern.clone();
    predecessor.agent_binding=authored_binding;
    // Resource presence retains the existing authenticated device identity;
    // the immutable empty Snapshot still authorizes no device or work Action.
    predecessor.agent_binding.typed_resource_bindings.extend(modern.agent_binding.typed_resource_bindings.iter()
        .filter(|resource|matches!(resource.resource_kind.0.as_str(),"companion"|"companion_memory"|"robot")).cloned());
    predecessor.agent_binding.typed_resource_bindings.sort_by(|left,right|left.binding_id.cmp(&right.binding_id));
    let binding=predecessor.agent_binding.clone();
    // Only host-owned source/display metadata identifies this readable old
    // official variant. Frozen Revision and Snapshot contents are never edited.
    assert_eq!(sqlx::query("UPDATE agent_presets SET source_json=?,display_json=? WHERE preset_id=?")
        .bind(json!("official").to_string())
        .bind(json!({"display_name":"chat.minimal","session_only":true}).to_string())
        .bind(binding.preset_revision_ref.preset_id.as_ref()).execute(flow.fixture.pool()).await.unwrap().rows_affected(),1);
    let wire_binding=serde_json::from_value(serde_json::to_value(&binding).unwrap()).unwrap();
    assert!(!flow.fixture.voice_control.is_retired_companion_device_binding(
        &nomifun_agent_contracts::UserId::from(flow.owner.clone()),&wire_binding,
    ).await.unwrap(),"ordinary bundled evolution must not use retired companion.robot adoption");

    let session:AgentSessionId=nomifun_common::generate_id().into();
    predecessor.agent_session_id=session.clone();
    predecessor.metadata.companion=modern.metadata.companion;
    predecessor.metadata.title=Some("Ordinary bundled voice onset evolution".into());
    predecessor.next_seq=1;
    let creation_key=format!("unified-companion-entry:ordinary-onset-{}",uuid::Uuid::now_v7());
    let opening_key=format!("user:{}:open:{creation_key}",flow.owner);
    let mut opening=nomifun_agent_session::CreateSessionRequest::new(predecessor,nomifun_common::now_ms(),
        format!("open:{opening_key}"),"session_api".into(),opening_key.clone().into(),format!("open:{opening_key}").into());
    opening.opening_event_id=Some(format!("opening:{opening_key}").into());
    opening.activation_event_id=Some(format!("active-set:{opening_key}").into());
    opening.initial_active_capability_ids=Vec::new();
    let created=store.create_session(opening).await
        .expect("bootstrap must validate the exact immutable empty binding and existing device resources");
    store.append_event(&nomifun_agent_contracts::SessionEventAppend {
        agent_session_id:session.clone(),event_id:format!("ready:{opening_key}").into(),producer_id:"runtime_supervisor".into(),
        idempotency_key:format!("{opening_key}:ready").into(),semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
            kind:nomifun_agent_contracts::SessionEventKind("session/ready".into()),kind_version:1,
            correlation_id:format!("session:{}",session.as_ref()).into(),causation_event_id:Some(created.opening_ack.event_id),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "resolved_snapshot_ref":binding.resolved_snapshot_ref,
            }))),
        },
    }).await.unwrap();
    session
}

#[tokio::test]
async fn ordinary_bundled_evolution_precedes_onset_capture_and_the_same_backend_dispatch_keeps_its_frozen_binding() {
    let server=CompanionFlowModelServer::new().await;
    let flow=PhysicalEntryFixture::new(&server,"ordinary-bundled-voice-onset").await;
    let session=empty_official_physical_entry(&flow).await;
    let store=flow.fixture.owner.canonical().store();
    let before=store.get_live_session(&session).await.unwrap();
    let old_binding=before.agent_binding.clone();
    let old_snapshot:(String,String)=sqlx::query_as("SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(old_binding.resolved_snapshot_ref.snapshot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    let old_revision:(String,String)=sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(old_binding.preset_revision_ref.preset_id.as_ref()).bind(old_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    let snapshot:Value=serde_json::from_str(&old_snapshot.0).unwrap();
    assert!(snapshot["enabled_capabilities"].as_array().unwrap().is_empty());
    assert!(store.agent_binding_transitions(&session).await.unwrap().is_empty());
    assert!(old_binding.typed_resource_bindings.iter().any(|resource|resource.resource_kind.0.as_str()=="robot"));
    let opening_before:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND seq=1")
        .bind(session.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    let desktop_head=store.head(&flow.desktop_id).await.unwrap().last_seq;
    let device_head=store.head(&flow.robot_id).await.unwrap().last_seq;
    let sessions_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    assert!(server.state.requests.lock().unwrap().is_empty());

    let backend=flow.fixture.services.robot.as_ref().unwrap().conversation_backend_for_test();
    let mut request=flow.request("ordinary-onset-first-input",FOLLOWUP_INPUT);
    request.conversation_id=session.as_ref().into();
    let captured=backend.capture_input_context(&flow.companion_id,&flow.device_id,session.as_ref(),&flow.connection,None)
        .await.unwrap().expect("the production backend must freeze its recording-time canonical context");
    let evolved=store.get_live_session(&session).await.unwrap();
    assert_eq!(evolved.agent_binding.binding_version,old_binding.binding_version+1);
    assert_eq!(captured.binding_version,evolved.agent_binding.binding_version,
        "onset must capture the evolved binding, never the predecessor which dispatch will immediately retire");
    assert_ne!(evolved.agent_binding.resolved_snapshot_ref,old_binding.resolved_snapshot_ref);
    assert_eq!(evolved.metadata.companion,before.metadata.companion);
    assert_eq!(store.agent_binding_transitions(&session).await.unwrap().len(),1,
        "ordinary bundled evolution requires one canonical ContextOnly transition");
    assert!(store.active_capability_ids(&session).await.unwrap().iter().any(|id|id=="workspace.files"));
    assert!(store.active_capability_ids(&session).await.unwrap().iter().any(|id|id=="workspace.process"));
    assert!(server.state.requests.lock().unwrap().is_empty(),"onset contract preparation is local and does not call a model");
    request.captured_context=Some(captured.clone());
    assert!(backend.input_context_matches(&request).await.unwrap());

    let mut events=backend.dispatch(request.clone()).await.unwrap();
    let (done,speech)=tokio::time::timeout(Duration::from_secs(25),async {
        let mut done=false;let mut speech=String::new();
        while let Some(event)=events.recv().await {
            match event {
                nomifun_robot::services::TurnEvent::PublicSpeech {source,text}=>{source.validate().unwrap();speech.push_str(&text);},
                nomifun_robot::services::TurnEvent::Done=>{done=true;break;},
                nomifun_robot::services::TurnEvent::Failed {message,..}=>panic!("the first recording revoked its own contract: {message}"),
                other=>panic!("ordinary real backend output must retain its canonical public source: {other:?}"),
            }
        }
        (done,speech)
    }).await.unwrap();
    assert!(done);assert!(speech.contains("The independent message is answered."),
        "Done alone cannot prove that onset evolution admitted and completed the model Turn: {speech:?}");
    assert_eq!(server.state.requests.lock().unwrap().len(),1);
    assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding,evolved.agent_binding,
        "the same frozen recording must not advance its binding again during dispatch");
    assert!(backend.input_context_matches(&request).await.unwrap());
    assert_eq!(store.agent_binding_transitions(&session).await.unwrap().len(),1);
    assert!(backend.acquire_output_context(&request,None,None).await.unwrap().is_some(),
        "an actually completed original Turn retains its final speech permit without requiring an active or latest Turn");
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    assert_eq!(store.head(&flow.desktop_id).await.unwrap().last_seq,desktop_head);
    assert_eq!(store.head(&flow.robot_id).await.unwrap().last_seq,device_head);
    let sessions_after:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(sessions_after,sessions_before,"ordinary speech does not create a child or desktop Session");
    let snapshot_after:(String,String)=sqlx::query_as("SELECT content_json,envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(old_binding.resolved_snapshot_ref.snapshot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    let revision_after:(String,String)=sqlx::query_as("SELECT payload_json,revision_digest FROM agent_preset_revisions WHERE preset_id=? AND revision_no=?")
        .bind(old_binding.preset_revision_ref.preset_id.as_ref()).bind(old_binding.preset_revision_ref.revision as i64)
        .fetch_one(flow.fixture.pool()).await.unwrap();
    let opening_after:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND seq=1")
        .bind(session.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(snapshot_after,old_snapshot);assert_eq!(revision_after,old_revision);assert_eq!(opening_after,opening_before);
    flow.finish().await;server.close().await;
}
