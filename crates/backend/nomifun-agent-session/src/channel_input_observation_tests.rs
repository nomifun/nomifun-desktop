use super::*;
use crate::ChannelInputDeliveryDisposition;
use nomifun_agent_contracts::{ChannelInputSource,ChannelInputDeliverySubject,ChannelInputQuestionDeliveredV1};

async fn fixture()->(AgentSessionStore,AgentSessionId,ChannelInputSource) {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"channel-observer").await;
    let plugin=Uuid::now_v7().to_string();let user=Uuid::now_v7().to_string();let binding=Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO channel_plugins(channel_plugin_id,type,name,enabled,config,owner_domain,group_access_mode,created_at,updated_at) \
        VALUES(?,'lark','Observer fixture',1,'{}','companion','allowlist',1,1)")
        .bind(&plugin).execute(store.test_pool()).await.unwrap();
    sqlx::query("INSERT INTO channel_users(channel_user_id,platform_user_id,platform_type,channel_plugin_id,authorization_kind,authorized_at) \
        VALUES(?,'transport-human','lark',?,'approved',1)").bind(&user).bind(&plugin).execute(store.test_pool()).await.unwrap();
    let sid=session.agent_session_id;
    sqlx::query("INSERT INTO channel_sessions(channel_session_id,channel_user_id,agent_type,conversation_id,chat_id,channel_plugin_id,chat_kind,created_at,last_activity) \
        VALUES(?,?,'nomi',?,'original-chat',?,'direct',1,1)").bind(&binding).bind(user)
        .bind(sid.as_ref()).bind(&plugin).execute(store.test_pool()).await.unwrap();
    let source=ChannelInputSource {platform:"lark".into(),channel_plugin_id:plugin,chat_id:"original-chat".into(),
        channel_session_id:binding,inbound_message_key:"channel:original-provider-message".into()};
    (store,sid,source)
}
async fn accepted(store:&AgentSessionStore,sid:&AgentSessionId,source:&ChannelInputSource)->EventId {
    store.accept_session_input(&owner(),sid,"session_api".into(),"original-inbound".into(),StrictJsonValue(json!({
        "content":"Original private request","files":[],"inject_skills":[],"channel_source":source})))
        .await.unwrap().record.unwrap().event_id
}
async fn closed(store:&AgentSessionStore,sid:&AgentSessionId,input:&EventId)->OperationId {
    let operation:OperationId="original-output-turn".into();
    let start=store.claim_session_input_as_turn(&owner(),sid,input,&operation,StrictJsonValue(json!({}))).await.unwrap().ack.unwrap().event_id;
    store.append_turn_terminal(&append(sid,"original-output-completed","runtime_supervisor","original-output-completed","turn/completed",
        operation.as_ref(),Some(start),json!({"result_text":"Original complete reply"})),&operation).await.unwrap();
    operation
}

#[tokio::test]
async fn accepted_cutpoint_restores_only_the_original_transport_and_retains_receipt_after_revocation() {
    let(store,sid,source)=fixture().await;
    let mut foreign=source.clone();foreign.chat_id="other-chat".into();
    assert!(store.accept_session_input(&owner(),&sid,"session_api".into(),"forged".into(),StrictJsonValue(json!({
        "content":"No rerouting","channel_source":foreign}))).await.is_err());
    let input=accepted(&store,&sid,&source).await;
    let recovered=store.recoverable_channel_inputs(&owner()).await.unwrap();
    assert_eq!(recovered.len(),1);assert_eq!(recovered[0].input_event_id,input);assert_eq!(recovered[0].source,source);
    sqlx::query("UPDATE channel_users SET authorization_kind='auto_group'").execute(store.test_pool()).await.unwrap();
    assert_eq!(accepted(&store,&sid,&source).await,input,"replaying acceptance confirms its old receipt");
    closed(&store,&sid,&input).await;
    assert!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.is_err(),
        "revocation stops automatic transport delivery");
}

#[tokio::test]
async fn intent_cutpoint_is_unknown_and_never_grants_a_second_outbound_attempt() {
    let(store,sid,source)=fixture().await;let input=accepted(&store,&sid,&source).await;closed(&store,&sid,&input).await;
    assert_eq!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.unwrap(),ChannelInputDeliveryDisposition::Ready);
    let restarted=AgentSessionStore::from_pool(store.test_pool().clone()).await.unwrap();
    assert_eq!(restarted.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.unwrap(),ChannelInputDeliveryDisposition::Unknown);
    assert!(restarted.recoverable_channel_inputs(&owner()).await.unwrap().is_empty());
    assert!(restarted.finish_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal,true,vec!["late-guess".into()]).await.is_err());
}

#[tokio::test]
async fn acknowledged_cutpoint_stays_confirmed_after_reopen_and_cannot_republish() {
    let(store,sid,source)=fixture().await;let input=accepted(&store,&sid,&source).await;closed(&store,&sid,&input).await;
    assert_eq!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.unwrap(),ChannelInputDeliveryDisposition::Ready);
    store.finish_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal,true,vec!["actual-provider-ack".into()]).await.unwrap();
    let restarted=AgentSessionStore::from_pool(store.test_pool().clone()).await.unwrap();
    assert_eq!(restarted.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.unwrap(),ChannelInputDeliveryDisposition::Confirmed);
    assert!(restarted.recoverable_channel_inputs(&owner()).await.unwrap().is_empty());
    assert_eq!(store.session_input(&sid,&input).await.unwrap().input.0["content"],"Original private request");
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='channel/input-delivery-state'")
        .bind(sid.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(count,2);
}

#[tokio::test]
async fn clearing_context_fences_a_retained_observer_before_its_first_outbound_attempt() {
    let(store,sid,source)=fixture().await;let input=accepted(&store,&sid,&source).await;closed(&store,&sid,&input).await;
    let request=crate::ClearSessionContext {idempotency_key:"clear-before-channel-delivery".into(),reason:"user_requested".into()};
    let crate::ContextClearPreparation::Ready(boundary)=store.preflight_context_clear(&owner(),&sid,&request).await.unwrap() else {panic!("fresh clear")};
    store.clear_context(&owner(),&sid,&request,&boundary).await.unwrap();
    assert!(store.recoverable_channel_inputs(&owner()).await.unwrap().is_empty());
    assert!(matches!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await,
        Err(SessionStoreError::ExecutionFenced)),"an already registered observer cannot bypass the clear boundary");
}

#[tokio::test]
async fn original_question_provider_proof_settles_an_ack_cut_without_sending_again() {
    let(store,sid,source)=fixture().await;let input=accepted(&store,&sid,&source).await;
    let operation:OperationId="question-output-turn".into();
    let start=store.claim_session_input_as_turn(&owner(),&sid,&input,&operation,StrictJsonValue(json!({}))).await.unwrap().ack.unwrap().event_id;
    let request=nomifun_agent_contracts::AgentUserInputRequest {request_id:"question".into(),question:"Authorize in the browser.".into(),remaining_work:"Verify authorization.".into()};
    let question=store.append_event(&append(&sid,"source-question","runtime_supervisor","source-question","runtime/progress-recorded",operation.as_ref(),Some(start),
        json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":request}}))).await.unwrap().cursor.seq;
    assert_eq!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Question {question_seq:question}).await.unwrap(),ChannelInputDeliveryDisposition::Ready);
    store.record_channel_input_question_delivery(&owner(),ChannelInputQuestionDeliveredV1 {agent_session_id:sid.clone(),owner_ref:owner(),operation_id:operation,
        source_message_id:input.clone(),question_seq:question,request_id:"question".into(),platform:source.platform,channel_plugin_id:source.channel_plugin_id,
        chat_id:source.chat_id,provider_message_id:"actual-question-provider-ack".into()}).await.unwrap();
    assert_eq!(store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Question {question_seq:question}).await.unwrap(),ChannelInputDeliveryDisposition::Confirmed);
}

#[tokio::test]
async fn different_channel_receivers_and_desktop_inputs_cannot_join_the_same_output_turn() {
    let(store,sid,source)=fixture().await;let root=accepted(&store,&sid,&source).await;
    let operation:OperationId="receiver-a-turn".into();
    store.claim_session_input_as_turn(&owner(),&sid,&root,&operation,StrictJsonValue(json!({}))).await.unwrap();
    let mut other=source.clone();other.chat_id="receiver-b".into();other.channel_session_id=Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO channel_sessions(channel_session_id,channel_user_id,agent_type,conversation_id,chat_id,channel_plugin_id,chat_kind,created_at,last_activity) \
        SELECT ?,channel_user_id,agent_type,conversation_id,?,channel_plugin_id,chat_kind,1,1 FROM channel_sessions WHERE channel_session_id=?")
        .bind(&other.channel_session_id).bind(&other.chat_id).bind(&source.channel_session_id).execute(store.test_pool()).await.unwrap();
    let accept=|text:&str,source:Option<&ChannelInputSource>|StrictJsonValue(json!({"content":text,"channel_source":source}));
    let second=store.accept_session_input(&owner(),&sid,"session_api".into(),"receiver-b-input".into(),accept("Private receiver B input",Some(&other))).await.unwrap().record.unwrap().event_id;
    let desktop=store.accept_session_input(&owner(),&sid,"session_api".into(),"desktop-input".into(),StrictJsonValue(json!({"content":"Desktop-only input"}))).await.unwrap().record.unwrap().event_id;
    for input in [&second,&desktop] {assert!(matches!(store.claim_session_input_as_steering(&owner(),&sid,input,&operation).await,Err(SessionStoreError::ExecutionFenced)));}
    let same=store.accept_session_input(&owner(),&sid,"session_api".into(),"receiver-a-supplement".into(),accept("Same receiver A supplement",Some(&source))).await.unwrap().record.unwrap().event_id;
    store.claim_session_input_as_steering(&owner(),&sid,&same,&operation).await.unwrap();
    assert!(store.session_input_claim(&sid,&second).await.unwrap().is_none());assert!(store.session_input_claim(&sid,&desktop).await.unwrap().is_none());
    store.cancel_active_turn(&sid,"receiver-a-finished".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),&sid,&second,&"receiver-b-turn".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input_claim(&sid,&second).await.unwrap().unwrap().operation_id.as_ref(),"receiver-b-turn");
}

#[tokio::test]
async fn recovery_filters_settled_metadata_before_large_bodies_and_pages_every_pending_identity() {
    let(store,sid,source)=fixture().await;
    let mut settled=Vec::new();
    for index in 0..24 {
        let input=store.accept_session_input(&owner(),&sid,"session_api".into(),format!("settled-{index}").into(),StrictJsonValue(json!({
            "content":format!("Previously delivered request {index}"),"channel_source":source}))).await.unwrap().ack.unwrap().event_id;
        let operation:OperationId=format!("settled-output-{index}").into();
        let start=store.claim_session_input_as_turn(&owner(),&sid,&input,&operation,StrictJsonValue(json!({}))).await.unwrap().ack.unwrap().event_id;
        store.append_turn_terminal(&append(&sid,&format!("settled-terminal-{index}"),"runtime_supervisor",&format!("settled-terminal-{index}"),"turn/completed",
            operation.as_ref(),Some(start),json!({})),&operation).await.unwrap();
        store.begin_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal).await.unwrap();
        store.finish_channel_input_delivery(&owner(),&sid,&input,input.as_ref(),ChannelInputDeliverySubject::Terminal,true,vec![format!("provider-ack-{index}")]).await.unwrap();
        settled.push(input);
    }
    // These already confirmed bodies must never be hydrated by recovery. A
    // scalar body cannot decode as a current accepted source; its original
    // delivery metadata alone fences any new outbound attempt.
    let unreadable_body=serde_json::to_string(&"unrelated large historical body".repeat(32_768)).unwrap();
    for input in settled {
        sqlx::query("UPDATE agent_events SET inline_json=? WHERE session_id=? AND event_id=?")
            .bind(&unreadable_body).bind(sid.as_ref()).bind(input.as_ref()).execute(store.test_pool()).await.unwrap();
    }
    let mut pending=Vec::new();
    for index in 0..129 {
        pending.push(store.accept_session_input(&owner(),&sid,"session_api".into(),format!("pending-{index}").into(),StrictJsonValue(json!({
            "content":format!("Undelivered request {index}"),"channel_source":source}))).await.unwrap().ack.unwrap().event_id);
    }
    let recovered=store.recoverable_channel_inputs(&owner()).await.unwrap();
    assert_eq!(recovered.into_iter().map(|input|input.input_event_id).collect::<Vec<_>>(),pending,
        "keyset pages continue beyond the first 64 pending inputs without a persistent cursor");
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE session_id=? AND event_id=?")
        .bind(json!({"content":"Corrupt selected input","channel_source":{"invalid":true}}).to_string()).bind(sid.as_ref()).bind(pending[0].as_ref())
        .execute(store.test_pool()).await.unwrap();
    assert!(store.recoverable_channel_inputs(&owner()).await.is_err(),"a selected malformed source fails closed instead of disappearing from recovery");
}
