//! Unified Session intake on the installed Host, including commit cutpoints.
use super::*;

fn principal(fixture:&Fixture)->PrincipalRef {
    PrincipalRef {principal_kind:"user".into(),principal_id:fixture.host.options.user_id.clone()}
}

async fn save(fixture:&Fixture,key:&str,text:&str)->nomifun_agent_contracts::EventId {
    fixture.owner.canonical().accept_session_input(&principal(fixture),&fixture.host.options.conversation_id.clone().into(),key,
        json!({"content":text,"files":[],"inject_skills":[]})).await.unwrap().ack.unwrap().event_id
}

#[tokio::test]
async fn a_shared_session_defers_other_channel_receivers_and_desktop_input_to_separate_turns() {
    let fixture=Fixture::new("session-input-receiver-isolation").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let principal=principal(&fixture);
    let store=fixture.owner.canonical().store();
    fixture.owner.canonical().cancel_exact_turn(&principal,&session,"close-initial-fixture",&"cleanup-retry-turn".into()).await.unwrap();
    let plugin=uuid::Uuid::now_v7().to_string();let user=uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO channel_plugins(channel_plugin_id,type,name,enabled,config,owner_domain,group_access_mode,created_at,updated_at) VALUES(?,'lark','Receiver fixture',1,'{}','companion','allowlist',1,1)")
        .bind(&plugin).execute(fixture.pool()).await.unwrap();
    sqlx::query("INSERT INTO channel_users(channel_user_id,platform_user_id,platform_type,channel_plugin_id,authorization_kind,authorized_at) VALUES(?,'fixture-human','lark',?,'approved',1)")
        .bind(&user).bind(&plugin).execute(fixture.pool()).await.unwrap();
    let mut sources=Vec::new();for chat in ["receiver-a","receiver-b"] {
        let binding=uuid::Uuid::now_v7().to_string();
        sqlx::query("INSERT INTO channel_sessions(channel_session_id,channel_user_id,agent_type,conversation_id,chat_id,channel_plugin_id,chat_kind,created_at,last_activity) VALUES(?,?,'nomi',?,?,?,'direct',1,1)")
            .bind(&binding).bind(&user).bind(session.as_ref()).bind(chat).bind(&plugin).execute(fixture.pool()).await.unwrap();
        sources.push(nomifun_agent_contracts::ChannelInputSource {platform:"lark".into(),channel_plugin_id:plugin.clone(),chat_id:chat.into(),
            channel_session_id:binding,inbound_message_key:format!("channel:{chat}:original")});
    }
    let root=fixture.owner.canonical().accept_session_input(&principal,&session,"channel-root",json!({"content":"Original receiver A request","channel_source":sources[0]})).await.unwrap().ack.unwrap().event_id;
    let operation:nomifun_agent_contracts::OperationId="receiver-output-turn".into();
    store.claim_session_input_as_turn(&principal,&session,&root,&operation,nomifun_agent_contracts::StrictJsonValue(json!({
        "route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}))).await.unwrap();
    let generation=store.native_execution_generation(&session,&operation).await.unwrap();
    let runtime=fixture.services.agent_runtime_sessions.get_or_create_runtime_for_turn(session.as_ref(),generation,
        CancellationToken::new(),fixture.registry_options()).await.unwrap();
    let message=SendMessageData {content:"Original receiver A request".into(),msg_id:root.as_ref().into(),
        source_message_id:Some(root.as_ref().into()),files:Vec::new(),inject_skills:Vec::new(),origin:None};
    let request=fixture.host.prepare_turn(&message,CancellationToken::new()).await.unwrap();
    fixture.host.record_event(&message,&nomifun_agent_runtime::AgentEngineEvent::TurnStarted {binding:fixture.host.engine_binding.clone(),turn_operation_id:operation}).await.unwrap();
    let second=fixture.owner.canonical().accept_session_input(&principal,&session,"channel-b-input",json!({"content":"Private receiver B input","channel_source":sources[1]})).await.unwrap().ack.unwrap().event_id;
    let desktop=save(&fixture,"desktop-only-supplement","Desktop private input").await;
    let same=fixture.owner.canonical().accept_session_input(&principal,&session,"channel-a-supplement",json!({"content":"Same receiver A supplement","channel_source":sources[0]})).await.unwrap().ack.unwrap().event_id;
    let port=request.input_port.unwrap();let delivered=port.take(&request.model_request.causality,false).await.unwrap();
    assert_eq!(delivered.len(),1);assert_eq!(delivered[0].message_id,same.as_ref());assert_eq!(delivered[0].text,"Same receiver A supplement");
    for id in [&second,&desktop] {assert!(store.session_input_claim(&session,id).await.unwrap().is_none());}
    assert!(!port.has_pending(&request.model_request.causality).await.unwrap(),"other receivers do not keep this model sampling");
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn ordinary_message_acceptance_and_original_key_replay_do_not_depend_on_the_active_turn() {
    let fixture=Fixture::new("session-input-active-acceptance").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let body=json!({"idempotency_key":"one-original-intent","delivery":{"kind":"prefer_current","turn_id":fixture.message.source_message_id},"input":{"content":"A safe-step decision"}});
    let (first,retry)=tokio::join!(api(&fixture._router,&path,body.clone()),api(&fixture._router,&path,body));
    assert_eq!(first["status"],"accepted");assert_eq!(first["message_id"],retry["message_id"]);
    assert!(first.get("operation_id").is_none());
    let input=first["message_id"].as_str().unwrap();
    assert_eq!(fixture.owner.canonical().store().head(&session).await.unwrap().active_turn_id.as_deref(),Some("cleanup-retry-turn"));
    let port=request.input_port.unwrap();let delivered=port.take(&request.model_request.causality,false).await.unwrap();
    assert_eq!(delivered.len(),1);assert_eq!(delivered[0].message_id,input);assert_eq!(delivered[0].text,"A safe-step decision");
    assert!(port.take(&request.model_request.causality,false).await.unwrap().is_empty());
    assert_eq!(fixture.owner.canonical().store().session_input_delivery(&session,&input.into()).await.unwrap()["state"],"claimed");
    let claim_count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted' AND kind_version=2")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(claim_count,1);
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn initial_only_messages_are_rejected_at_acceptance_without_becoming_late_supplements() {
    let fixture=Fixture::new("session-input-initial-only-api").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let before=fixture.owner.canonical().store().current_cursor(&session).await.unwrap();
    let response=fixture._router.clone().oneshot(Request::builder().method("POST")
        .uri(format!("/api/agent-sessions/{}/messages",session.as_ref()))
        .header("x-nomi-local-trust","cleanup-fixture-local-trust").header("content-type","application/json").header("x-nomifun-initial-delivery","1")
        .body(Body::from(json!({"idempotency_key":"late-initial-request","input":{"content":"A late welcome handoff"}}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::CONFLICT);
    assert_eq!(fixture.owner.canonical().store().current_cursor(&session).await.unwrap(),before);
    assert!(fixture.owner.canonical().session_input_by_key(&principal(&fixture),&session,"late-initial-request").await.unwrap().is_none());
    fixture.finish().await;
}

#[tokio::test]
async fn closed_input_scope_accepts_pending_information_without_reopening_the_original_turn() {
    let fixture=Fixture::new("session-input-closed-scope").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    assert!(request.input_port.as_ref().unwrap().take(&request.model_request.causality,true).await.unwrap().is_empty());
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let body=json!({"idempotency_key":"after-close","input":{"content":"Keep this next intent"}});
    let saved=api(&fixture._router,&path,body.clone()).await;
    let replay=api(&fixture._router,&path,body).await;assert_eq!(saved["message_id"],replay["message_id"]);
    let id=nomifun_agent_contracts::EventId::from(saved["message_id"].as_str().unwrap());
    assert!(fixture.owner.canonical().store().session_input_claim(&session,&id).await.unwrap().is_none());
    assert_eq!(fixture.owner.canonical().store().session_input_delivery(&session,&id).await.unwrap()["state"],"pending");
    assert!(!request.input_port.as_ref().unwrap().has_pending(&request.model_request.causality).await.unwrap());
    assert!(!fixture.host.active.lock().await.as_ref().unwrap().steering.permits_resource_dispatch());
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn unavailable_selected_skill_is_saved_and_publicly_blocked_without_failing_the_active_turn() {
    let fixture=Fixture::new("session-input-skill-preparation").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let saved=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),
        json!({"idempotency_key":"unavailable-input-skill","delivery":{"kind":"prefer_current","turn_id":fixture.message.source_message_id},"input":{"content":"Use the selected reference", "inject_skills":["not-selected-in-this-snapshot"]}})).await;
    let id=nomifun_agent_contracts::EventId::from(saved["message_id"].as_str().unwrap());
    let original=fixture.owner.canonical().store().session_input(&session,&id).await.unwrap();
    assert_eq!(original.input.0["delivery_intent"]["kind"],"prefer_current");
    assert!(original.input.0.get("delivery").is_none(),"intent must not shadow the current-generation content delivery envelope");
    assert_eq!(original.input.0["inject_skills"],json!(["not-selected-in-this-snapshot"]));
    let cursor=fixture.owner.canonical().store().current_cursor(&session).await.unwrap().seq;
    let port=request.input_port.unwrap();assert!(port.take(&request.model_request.causality,false).await.unwrap().is_empty());
    let state=fixture.owner.canonical().store().session_input_delivery(&session,&id).await.unwrap();
    assert_eq!(state["state"],"blocked");assert_eq!(state["reason"],"SKILL_UNAVAILABLE");assert!(state["event_seq"].as_u64().unwrap()>cursor);
    assert_eq!(fixture.owner.canonical().store().read_turn_receipt(&session,&"cleanup-retry-turn".into()).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Running);
    let response=fixture._router.clone().oneshot(Request::builder().method("GET")
        .uri(format!("/api/agent-sessions/{}/messages?after_seq={cursor}",session.as_ref()))
        .header("x-nomi-local-trust","cleanup-fixture-local-trust").body(Body::empty()).unwrap()).await.unwrap();
    assert!(response.status().is_success());
    let body:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap()).unwrap();
    assert!(body["data"]["messages"].as_array().unwrap().iter().any(|message|message["projection"]["input_delivery"]==state));
    let later=save(&fixture,"valid-after-blocked","An independent valid decision").await;
    let delivered=port.take(&request.model_request.causality,false).await.unwrap();assert_eq!(delivered.len(),1);assert_eq!(delivered[0].message_id,later.as_ref());
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn a_quoted_closed_question_is_context_data_for_its_exact_source_not_the_latest_question() {
    let fixture=Fixture::new("session-input-exact-question-context").await;
    let (_request,runtime)=prepare_live_steering(&fixture).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let mut message=fixture.message.clone();message.msg_id=message.source_message_id.clone().unwrap();
    let question=nomifun_agent_contracts::AgentUserInputRequest {request_id:"original-question-a".into(),question:"Exact original question A".into(),remaining_work:"Verify A".into()};
    let seq=fixture.host.record_event(&message,&AgentEngineEvent::UserInputRequested {step:1,request:question.clone()}).await.unwrap().unwrap();
    fixture.host.cleanup_turn(&message,&nomifun_ai_agent::engine_sdk::EngineTurnTerminal::InputRequired {request:question.clone()}).await.unwrap();
    fixture.host.record_event(&message,&AgentEngineEvent::TurnInputRequired {model_steps:1,request:question}).await.unwrap();
    let store=fixture.owner.canonical().store();
    let (_,started)=store.start_turn(&session,"session_api".into(),"newer-question-task".into(),"newer-question-turn".into(),StrictJsonValue(json!({"content":"A different newer topic"}))).await.unwrap();
    store.append_event(&nomifun_agent_contracts::SessionEventAppend {agent_session_id:session.clone(),event_id:"newer-question-b".into(),producer_id:"runtime_supervisor".into(),idempotency_key:"newer-question-b".into(),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:nomifun_agent_contracts::SessionEventKind("runtime/progress-recorded".into()),kind_version:1,correlation_id:"newer-question-turn".into(),causation_event_id:Some(started.ack.unwrap().event_id),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,
                "request":{"request_id":"newer-question-b","question":"Unrelated latest question B","remaining_work":"Verify B"}}})))}}).await.unwrap();
    let accepted=fixture.owner.canonical().accept_session_input(&principal(&fixture),&session,"quote-a-context",json!({"content":"My reply refers to A","files":[],"inject_skills":[],
        "reply_to":{"turn_id":message.source_message_id,"question_seq":seq}})).await.unwrap().ack.unwrap().event_id;
    let original=store.session_input(&session,&accepted).await.unwrap();
    let context=super::super::question_reference_context(store,&session,&original.input.0).await.unwrap().unwrap();
    assert!(context.contains("Exact original question A"));assert!(!context.contains("Unrelated latest question B"));
    assert!(store.session_input_claim(&session,&accepted).await.unwrap().is_none());
    store.cancel_active_turn(&session,"close-newer-question".into(),"session_api".into()).await.unwrap();
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn dropping_safe_take_after_claim_keeps_native_delivery_and_prevents_a_second_claim() {
    let fixture=Fixture::new("session-input-owned-take").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    let id=save(&fixture,"owned-take-input","Retain this exact original input").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let reached=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
    INPUT_CLAIM_BARRIERS.get_or_init(Default::default).lock().unwrap().insert(session.as_ref().into(),(reached.clone(),release.clone()));
    let port=request.input_port.unwrap();let caller_port=port.clone();let cause=request.model_request.causality;
    let caller_cause=cause.clone();let waiter=tokio::spawn(async move {caller_port.take(&caller_cause,false).await});
    tokio::time::timeout(Duration::from_secs(2),reached.notified()).await.unwrap();
    assert!(fixture.owner.canonical().store().session_input_claim(&session,&id).await.unwrap().is_some());
    assert_eq!(fixture.owner.canonical().store().session_input_delivery(&session,&id).await.unwrap()["state"],"pending");
    waiter.abort();let _=waiter.await;
    let closing=tokio::spawn(async move {port.take(&cause,true).await});
    tokio::task::yield_now().await;assert!(!closing.is_finished(),"close must wait for the owned commit-to-native boundary");
    release.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(3),closing).await.unwrap().unwrap().unwrap().is_empty());
    assert_eq!(fixture.owner.canonical().store().session_input_delivery(&session,&id).await.unwrap()["state"],"claimed");
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted' AND kind_version=2")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(count,1);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&"cleanup-retry-turn".into()).await.unwrap();
    assert!(facts.event_payloads.values().any(|payload|payload.pointer("/event/inputs/0/message_id").and_then(Value::as_str)==Some(id.as_ref())));
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn native_delivery_write_failure_never_hides_the_claimed_original_input() {
    let fixture=Fixture::new("session-input-native-write-failure").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    let id=save(&fixture,"journal-failure-input","Saved original input after a real DB failure").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    sqlx::query("CREATE TRIGGER reject_native_input_delivery BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='steering_inputs' BEGIN SELECT RAISE(FAIL,'fixture native input delivery failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let result=request.input_port.unwrap().take(&request.model_request.causality,false).await;
    assert!(result.unwrap_err().to_string().contains("fixture native input delivery failure"));
    assert!(fixture.owner.canonical().store().session_input_claim(&session,&id).await.unwrap().is_some());
    assert_eq!(fixture.owner.canonical().store().session_input_delivery(&session,&id).await.unwrap()["state"],"pending","the claim does not falsely ACK native delivery");
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&"cleanup-retry-turn".into()).await.unwrap();
    assert_eq!(facts.event_payloads[id.as_ref()]["content"],"Saved original input after a real DB failure");
    let context=nomifun_agent_session::canonical_context_messages(&principal(&fixture),&facts.events,&facts.event_payloads,0,0,u64::MAX).unwrap();
    assert!(context.iter().any(|message|message.content=="Saved original input after a real DB failure"),"claim-only content remains canonical DATA rather than disappearing behind native dedup");
    assert!(!fixture.host.active.lock().await.as_ref().unwrap().steering.permits_resource_dispatch());
    sqlx::query("DROP TRIGGER reject_native_input_delivery").execute(fixture.pool()).await.unwrap();
    let mut message=fixture.message.clone();message.msg_id=message.source_message_id.clone().unwrap();
    fixture.host.suspend_after_cleanup_failure(&message).await.unwrap();
    assert_eq!(fixture.owner.canonical().store().head(&session).await.unwrap().status,"paused");
    let replay=save(&fixture,"journal-failure-input","Saved original input after a real DB failure").await;assert_eq!(replay,id);
    assert!(fixture.owner.canonical().store().pending_session_inputs(&session,Some(&id)).await.unwrap().is_empty());
    // The deliberately uncertain native journal must stay paused. Teardown
    // may report that exact unproven cleanup; it must never manufacture it.
    let _=runtime.kill_and_wait(None).await;let _=fixture.services.shutdown_nomi_core_host().await;
}

#[tokio::test]
async fn exhausted_turn_input_budget_defers_the_seventeenth_input_without_permanent_blocking() {
    let fixture=Fixture::new("session-input-next-turn-budget").await;
    let (request,runtime)=prepare_live_steering(&fixture).await;
    let port=request.input_port.unwrap();let cause=request.model_request.causality;
    for index in 0..16 {
        save(&fixture,&format!("supplement-{index}"),&format!("User supplement {index}")).await;
        assert_eq!(port.take(&cause,false).await.unwrap().len(),1);
    }
    let last=save(&fixture,"seventeenth","A legal next Turn intent").await;
    assert!(port.take(&cause,false).await.unwrap().is_empty());
    assert!(!port.has_pending(&cause).await.unwrap(),"a depleted Turn must still be able to complete");
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let store=fixture.owner.canonical().store();
    assert!(store.session_input_claim(&session,&last).await.unwrap().is_none());
    assert_eq!(store.session_input_delivery(&session,&last).await.unwrap()["state"],"pending");
    assert_eq!(store.pending_session_inputs(&session,None).await.unwrap().iter().map(|input|input.event.event_id.clone()).collect::<Vec<_>>(),[last.clone()]);
    // This fixture manually owns a prepared Turn and has no running model
    // driver. Close that exact owner through its real cleanup/terminal path;
    // killing an idle SDK handle cannot invent a missing driver terminal.
    let mut original=fixture.message.clone();original.msg_id=original.source_message_id.clone().unwrap();
    fixture.host.cleanup_turn(&original,&nomifun_ai_agent::engine_sdk::EngineTurnTerminal::Cancelled).await.unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::TurnCancelled {model_steps:0}).await.unwrap();
    runtime.kill_and_wait(None).await.unwrap();
    store.claim_session_input_as_turn(&principal(&fixture),&session,&last,&"next-input-budget-turn".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input_claim(&session,&last).await.unwrap().unwrap().operation_id.as_ref(),"next-input-budget-turn");
    store.cancel_active_turn(&session,"close-budget-successor".into(),"session_api".into()).await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn a_postclaim_snapshot_read_failure_settles_only_the_original_undispatched_turn() {
    let fixture=Fixture::new("session-input-dispatch-read-failure").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let store=fixture.owner.canonical().store();
    fixture.owner.canonical().cancel_exact_turn(&principal(&fixture),&session,"close-initial-fixture",&"cleanup-retry-turn".into()).await.unwrap();
    let id=save(&fixture,"postclaim-config-read","Original input before configuration read failure").await;
    let operation=nomifun_agent_contracts::OperationId::from("turn:postclaim-config-read");
    let claimed=store.claim_session_input_as_turn(&principal(&fixture),&session,&id,&operation,
        StrictJsonValue(json!({"route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}))).await.unwrap();
    let snapshot=fixture.host.snapshot_ref.snapshot_id.as_ref();
    let envelope:String=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?").bind(snapshot).fetch_one(fixture.pool()).await.unwrap();
    sqlx::query("UPDATE agent_runtime_snapshots SET envelope_json='{}' WHERE snapshot_id=?").bind(snapshot).execute(fixture.pool()).await.unwrap();
    let request=serde_json::from_value::<nomifun_api_types::SendMessageRequest>(json!({"content":"Original input before configuration read failure"})).unwrap();
    assert!(fixture.owner.deliver_claimed_turn(&fixture.host.options.user_id,&session,request,
        nomifun_conversation::AgentTurnReceipt {operation_id:operation.clone(),cursor:claimed.cursor,duplicate:false}).await.is_err());
    sqlx::query("UPDATE agent_runtime_snapshots SET envelope_json=? WHERE snapshot_id=?").bind(envelope).bind(snapshot).execute(fixture.pool()).await.unwrap();
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Failed);
    assert_eq!(store.head(&session).await.unwrap().status,"ready");
    assert!(!store.has_unsettled_effects(&session).await.unwrap());
    assert_eq!(save(&fixture,"postclaim-config-read","Original input before configuration read failure").await,id);
    assert_eq!(store.session_input_claim(&session,&id).await.unwrap().unwrap().operation_id,operation);
    assert!(store.pending_session_inputs(&session,Some(&id)).await.unwrap().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn a_queued_input_reads_the_predecessors_later_output_and_question_once_at_its_claim_boundary() {
    let fixture=Fixture::new("session-input-queued-history-boundary").await;
    let(request,runtime)=prepare_live_steering(&fixture).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let store=fixture.owner.canonical().store();
    let mut original=fixture.message.clone();original.msg_id=original.source_message_id.clone().unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::ModelStepStarted {step:1,
        operation_id:"cleanup-retry-turn:model:1".into(),correction_source:None}).await.unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::OutputTextDelta {step:1,text:"A output before queued input. ".into(),source:None}).await.unwrap();
    // The old Turn has closed its input boundary, but still owns its final
    // output. Saving B here must neither reopen A nor freeze A's history early.
    assert!(request.input_port.unwrap().take(&request.model_request.causality,true).await.unwrap().is_empty());
    let content="Current queued B input must be supplied only as the new requirement";
    let input=save(&fixture,"queued-history-b",content).await;
    let accepted=store.session_input(&session,&input).await.unwrap();
    assert!(store.session_input_claim(&session,&input).await.unwrap().is_none());
    fixture.host.record_event(&original,&AgentEngineEvent::OutputTextDelta {step:1,text:"A complete output after queued input.".into(),source:None}).await.unwrap();
    let question=nomifun_agent_contracts::AgentUserInputRequest {request_id:"queued-history-a-question".into(),
        question:"A exact question committed after B was saved".into(),remaining_work:"Verify the user's eventual decision".into()};
    fixture.host.record_event(&original,&AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {
        call_id:question.request_id.clone(),name:"request_user_input".into(),provider_metadata:None,
        arguments:StrictJsonValue(json!({"question":question.question,"remaining_work":question.remaining_work})),
    }}).await.unwrap();
    let question_seq=fixture.host.record_event(&original,&AgentEngineEvent::UserInputRequested {step:1,request:question.clone()}).await.unwrap().unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,
        result:nomifun_agent_runtime::AgentToolResult::text(question.request_id.clone(),
            "Input request recorded. This is a pending input handoff, not a user answer or task-completion receipt.",false)}).await.unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::ToolResultsOrdered {step:1,call_ids:vec![question.request_id.clone()]}).await.unwrap();
    fixture.host.cleanup_turn(&original,&nomifun_ai_agent::engine_sdk::EngineTurnTerminal::InputRequired {request:question.clone()}).await.unwrap();
    fixture.host.record_event(&original,&AgentEngineEvent::TurnInputRequired {model_steps:1,request:question.clone()}).await.unwrap();
    let terminal=store.read_turn_receipt(&session,&"cleanup-retry-turn".into()).await.unwrap().terminal_event.unwrap();
    let operation:nomifun_agent_contracts::OperationId="queued-history-b-turn".into();
    let claimed=store.claim_session_input_as_turn(&principal(&fixture),&session,&input,&operation,
        StrictJsonValue(json!({"route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}))).await.unwrap();
    assert!(accepted.event.seq<question_seq && question_seq<terminal.seq && terminal.seq<claimed.cursor.seq);
    let message=SendMessageData {content:content.into(),msg_id:input.as_ref().into(),source_message_id:Some(input.as_ref().into()),
        files:Vec::new(),inject_skills:Vec::new(),origin:None};
    let engines=&fixture.host.session_host;
    let receipt=engines.read_turn_receipt(&fixture.host.options,&fixture.host.binding,&fixture.host.snapshot_ref,&message).await.unwrap();
    let before=store.current_cursor(&session).await.unwrap();
    let window=engines.read_history(&receipt,8).await.unwrap();
    assert_eq!(window.turns.len(),1);assert_eq!(window.turns[0].operation_id,"cleanup-retry-turn");
    assert_eq!(window.turns[0].receipt_status,"completed");
    assert!(window.context_messages.iter().all(|item|!item.content.contains(content)),"the new current root must not duplicate itself as historical DATA");
    let records=window.turns[0].records.iter().map(|record|serde_json::from_str::<Value>(&record.event_json).unwrap()).collect::<Vec<_>>();
    assert!(records.iter().any(|event|event["event"]=="user_input_requested" && event["request"]["question"]==question.question));
    assert!(records.iter().any(|event|event["event"]=="turn_input_required"));
    assert!(records.iter().any(|event|event["event"]=="output_text_delta" && event["text"]=="A complete output after queued input."));
    let history=super::super::super::unified_runtime_history::load(window,engines,&receipt,1024*1024).await.unwrap();
    let text=history.messages.iter().flat_map(|message|&message.content).filter_map(|part|match part {
        nomifun_chat_model_broker::ChatContentPart::Text {text}=>Some(text.as_str()),_=>None,
    }).collect::<Vec<_>>();
    assert_eq!(text.iter().filter(|text|text.contains("A exact question committed after B was saved")).count(),1);
    assert_eq!(text.iter().filter(|text|text.contains("A output before queued input. A complete output after queued input.")).count(),1);
    assert!(text.iter().all(|text|!text.contains(content)));
    let addressed=engines.read_history_exact(&receipt,"cleanup-retry-turn").await.unwrap();
    assert_eq!(addressed.turns.len(),1,"exact history uses the same actual claim boundary");
    let current=engines.read_history_exact(&receipt,operation.as_ref()).await.unwrap();
    assert!(current.turns.is_empty() && current.context_messages.is_empty(),
        "the current Turn is ineligible and supplies no closed history");
    assert_eq!(store.current_cursor(&session).await.unwrap(),before,"history reads cannot create execution or delivery facts");
    store.cancel_active_turn(&session,"close-queued-history-b".into(),"session_api".into()).await.unwrap();
    runtime.kill_and_wait(None).await.unwrap();fixture.finish().await;
}

#[tokio::test]
async fn queued_turn_history_merges_intervening_domain_data_at_the_claim_not_the_accepted_input() {
    let fixture=Fixture::new("session-input-claim-merge-order").await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());let store=fixture.owner.canonical().store();
    // No Runtime has dispatched A or B. Their real canonical cancellation
    // receipts make them data-only history without invented native records.
    let b_text="Queued B original requirement";
    let b=fixture.owner.canonical().accept_session_input(&principal(&fixture),&session,"merge-b",json!({"content":b_text,
        "files":["queued-reference.txt"],"inject_skills":[]})).await.unwrap().ack.unwrap();
    let(_,a_terminal)=store.cancel_exact_turn(&session,&"cleanup-retry-turn".into(),"merge-close-a".into(),"session_api".into()).await.unwrap();
    let a_terminal=a_terminal.ack.unwrap();
    let notice="Domain notice committed after A and before B claimed execution";
    let notice_id=uuid::Uuid::now_v7().to_string();
    let notice_ack=store.append_event(&nomifun_agent_contracts::SessionEventAppend {agent_session_id:session.clone(),
        event_id:notice_id.clone().into(),producer_id:"session_api".into(),idempotency_key:notice_id.clone().into(),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:nomifun_agent_contracts::SessionEventKind("message/assistant-projected".into()),kind_version:1,
            correlation_id:notice_id.into(),causation_event_id:Some(a_terminal.event_id),
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"content":notice})))}}).await.unwrap().ack.unwrap();
    let b_operation:nomifun_agent_contracts::OperationId="merge-b-turn".into();
    let b_claim=store.claim_session_input_as_turn(&principal(&fixture),&session,&b.event_id,&b_operation,
        StrictJsonValue(json!({"route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}))).await.unwrap().ack.unwrap();
    store.cancel_exact_turn(&session,&b_operation,"merge-close-b".into(),"session_api".into()).await.unwrap();
    let c_text="Current C input must not repeat as historical data";
    let c=save(&fixture,"merge-c",c_text).await;let c_operation:nomifun_agent_contracts::OperationId="merge-c-turn".into();
    store.claim_session_input_as_turn(&principal(&fixture),&session,&c,&c_operation,
        StrictJsonValue(json!({"route_identity":fixture.host.route,"resolved_snapshot_ref":fixture.host.snapshot_ref}))).await.unwrap();
    assert!(b.seq<a_terminal.seq && a_terminal.seq<notice_ack.seq && notice_ack.seq<b_claim.seq);
    let message=SendMessageData {content:c_text.into(),msg_id:c.as_ref().into(),source_message_id:Some(c.as_ref().into()),
        files:Vec::new(),inject_skills:Vec::new(),origin:None};
    let engines=&fixture.host.session_host;
    let receipt=engines.read_turn_receipt(&fixture.host.options,&fixture.host.binding,&fixture.host.snapshot_ref,&message).await.unwrap();
    let before=store.current_cursor(&session).await.unwrap();
    let window=engines.read_history(&receipt,8).await.unwrap();
    assert_eq!(window.turns.len(),2);assert_eq!(window.turns[0].claim_seq,b_claim.seq);
    assert!(window.turns.iter().all(|turn|turn.unstarted_terminal && turn.records.is_empty()));
    let history=super::super::super::unified_runtime_history::load(window,engines,&receipt,1024*1024).await.unwrap();
    let text=history.messages.iter().flat_map(|message|&message.content).filter_map(|part|match part {
        nomifun_chat_model_broker::ChatContentPart::Text {text}=>Some(text.as_str()),_=>None,
    }).collect::<Vec<_>>();
    let position=|expected:&str|text.iter().position(|text|*text==expected).unwrap();
    assert!(position(&fixture.message.content)<position(notice) && position(notice)<position(b_text),"history must merge A → domain notice → B at the execution claim");
    assert_eq!(text.iter().filter(|text|**text==b_text).count(),1);
    assert_eq!(text.iter().filter(|text|text.contains("queued-reference.txt")).count(),1,"the one accepted payload supplies attachment references");
    assert!(text.iter().all(|text|!text.contains(c_text)));
    let cause=nomifun_chat_model_broker::ChatCausality {agent_session_id:session.clone(),turn_operation_id:c_operation.clone(),
        causation_event_id:c,resolved_snapshot_ref:fixture.host.snapshot_ref.clone(),route_identity:fixture.host.route.clone(),operation_id:"merge-c-turn:model:1".into()};
    let port=super::super::history_port::HistoryPort {host:engines.clone(),receipt:Arc::new(receipt),cancellation:CancellationToken::new()};
    let addressed=nomifun_agent_runtime::AgentHistoryPort::read_exact(&port,&cause,b_operation.as_ref()).await.unwrap().unwrap().turn.unwrap();
    let requirement=serde_json::to_string(&addressed.requirement).unwrap();
    assert!(requirement.contains(b_text) && requirement.contains("queued-reference.txt"),"the scoped history port decodes the same single accepted payload");
    assert_eq!(store.current_cursor(&session).await.unwrap(),before);
    store.cancel_exact_turn(&session,&c_operation,"merge-close-c".into(),"session_api".into()).await.unwrap();
    fixture.finish().await;
}
