use super::*;
use crate::{NativeExecutionClaim,NativeTurnMutationFence};

#[tokio::test]
async fn voice_off_cancel_and_steer_keep_original_payload_bytes_and_receipt_behavior() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_turn(&store,"voice-off-parity","plain-turn").await;
    let input=StrictJsonValue(json!({"content":"ordinary safe-boundary text"}));
    let (_,steer)=store.steer_active_turn(&session.agent_session_id,"plain-steer".into(),"session_api".into(),input.clone()).await.unwrap();
    let record=steer.record.unwrap();let expected=json!({"target_operation_id":"plain-turn","input":input});
    assert_eq!(serde_json::to_vec(&record.payload).unwrap(),serde_json::to_vec(&SessionEventPayloadRef::InlineJson(StrictJsonValue(expected))).unwrap());
    let (target,cancel)=store.cancel_active_turn(&session.agent_session_id,"plain-stop".into(),"session_api".into()).await.unwrap();assert_eq!(target.as_ref(),"plain-turn");
    let record=cancel.record.unwrap();let SessionEventPayloadRef::InlineJson(body)=&record.payload else {panic!("inline default cancellation")};
    let expected=json!({"target_operation_id":"plain-turn","finished_at_ms":body.0["finished_at_ms"]});
    assert_eq!(serde_json::to_vec(&record.payload).unwrap(),serde_json::to_vec(&SessionEventPayloadRef::InlineJson(StrictJsonValue(expected))).unwrap());
    let (_,replay)=store.cancel_active_turn(&session.agent_session_id,"plain-stop".into(),"session_api".into()).await.unwrap();assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,record.event_id);
    assert_eq!(store.read_turn_receipt(&session.agent_session_id,&"plain-turn".into()).await.unwrap().status,TurnReceiptStatus::Cancelled);
}

#[tokio::test]
async fn voice_native_fence_rejects_same_turn_recovery_generation_and_preserves_plain_api() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_turn(&store,"voice-native-generation","native-turn").await;
    let claim=|holder:&str|NativeExecutionClaim {owner:owner(),agent_session_id:session.agent_session_id.clone(),operation_id:"native-turn".into(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),
        active_set_generation:0,holder:holder.into(),expected_fence:0,checkpoint:None};
    let original=store.claim_native_execution(claim("original-owner")).await.unwrap();let fence=NativeTurnMutationFence {binding_version:session.agent_binding.binding_version,execution_generation:original.generation()};
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id='native-turn'").bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    let replacement=store.claim_native_empty_recovery(claim("replacement-owner")).await.unwrap();assert_ne!(original.generation(),replacement.generation());
    assert!(matches!(store.cancel_exact_native_turn(&session.agent_session_id,&"native-turn".into(),"stale-voice-stop".into(),"session_api".into(),&fence).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.steer_exact_native_turn(&session.agent_session_id,&"native-turn".into(),"stale-voice-steer".into(),"session_api".into(),StrictJsonValue(json!({"content":"stale correction"})),&fence).await,Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.read_turn_receipt(&session.agent_session_id,&"native-turn".into()).await.unwrap().status,TurnReceiptStatus::Running);
    store.cancel_exact_turn(&session.agent_session_id,&"native-turn".into(),"plain-exact-stop".into(),"session_api".into()).await.unwrap();
    assert_eq!(store.read_turn_receipt(&session.agent_session_id,&"native-turn".into()).await.unwrap().status,TurnReceiptStatus::Cancelled);
}

#[tokio::test]
async fn voice_native_fence_replay_keeps_original_proof_after_successor() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_turn(&store,"voice-native-replay","original-turn").await;
    let lease=store.claim_native_execution(NativeExecutionClaim {owner:owner(),agent_session_id:session.agent_session_id.clone(),operation_id:"original-turn".into(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),
        active_set_generation:0,holder:"voice-owner".into(),expected_fence:0,checkpoint:None}).await.unwrap();
    let fence=NativeTurnMutationFence {binding_version:session.agent_binding.binding_version,execution_generation:lease.generation()};
    let (_,first)=store.cancel_exact_native_turn(&session.agent_session_id,&"original-turn".into(),"voice-stop".into(),"session_api".into(),&fence).await.unwrap();
    store.start_turn(&session.agent_session_id,"session_api".into(),"successor-input".into(),"successor-turn".into(),StrictJsonValue(json!({"content":"independent task"}))).await.unwrap();
    let (_,replay)=store.cancel_exact_native_turn(&session.agent_session_id,&"original-turn".into(),"voice-stop".into(),"session_api".into(),&fence).await.unwrap();assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,first.record.unwrap().event_id);
    let changed=NativeTurnMutationFence {execution_generation:fence.execution_generation+1,..fence};
    assert!(matches!(store.cancel_exact_native_turn(&session.agent_session_id,&"original-turn".into(),"voice-stop".into(),"session_api".into(),&changed).await,Err(SessionStoreError::IdempotencyConflict(_))));
    assert_eq!(store.head(&session.agent_session_id).await.unwrap().active_turn_id.as_deref(),Some("successor-turn"));
}

#[tokio::test]
async fn voice_opening_fence_zero_is_real_and_is_rejected_after_native_claim() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_turn(&store,"voice-opening-zero","opening-turn").await;
    let zero=NativeTurnMutationFence {binding_version:session.agent_binding.binding_version,execution_generation:0};
    let lease=store.claim_native_execution(NativeExecutionClaim {owner:owner(),agent_session_id:session.agent_session_id.clone(),operation_id:"opening-turn".into(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),
        active_set_generation:0,holder:"opened-owner".into(),expected_fence:0,checkpoint:None}).await.unwrap();assert!(lease.generation()>0);
    assert!(matches!(store.cancel_exact_native_turn(&session.agent_session_id,&"opening-turn".into(),"old-zero".into(),"session_api".into(),&zero).await,Err(SessionStoreError::ExecutionFenced)));
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_turn(&store,"voice-fast-zero-stop","before-claim-turn").await;
    let zero=NativeTurnMutationFence {binding_version:session.agent_binding.binding_version,execution_generation:0};
    store.cancel_exact_native_turn(&session.agent_session_id,&"before-claim-turn".into(),"zero-stop".into(),"session_api".into(),&zero).await.unwrap();
    assert!(matches!(store.claim_native_execution(NativeExecutionClaim {owner:owner(),agent_session_id:session.agent_session_id.clone(),operation_id:"before-claim-turn".into(),snapshot:session.agent_binding.resolved_snapshot_ref.clone(),
        active_set_generation:0,holder:"must-not-start".into(),expected_fence:0,checkpoint:None}).await,Err(SessionStoreError::ExecutionFenced)));
}

#[tokio::test]
async fn voice_off_start_keeps_original_input_and_turn_payload_bytes() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,_)=create_ready(&store,"voice-off-start").await;
    let input=StrictJsonValue(json!({"content":"normal desktop input","files":[],"hidden":false}));
    let (message,turn)=store.start_turn(&session.agent_session_id,"session_api".into(),"plain-input".into(),"plain-operation".into(),input.clone()).await.unwrap();
    let message=message.record.unwrap();let turn=turn.record.unwrap();
    assert_eq!(serde_json::to_vec(&message.payload).unwrap(),serde_json::to_vec(&SessionEventPayloadRef::InlineJson(input.clone())).unwrap());
    let expected=SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"plain-operation","source_message_id":message.event_id})));
    assert_eq!(serde_json::to_vec(&turn.payload).unwrap(),serde_json::to_vec(&expected).unwrap());
    let (_,replay)=store.start_turn(&session.agent_session_id,"session_api".into(),"plain-input".into(),"plain-operation".into(),input).await.unwrap();assert!(replay.duplicate);assert_eq!(replay.record.unwrap().event_id,turn.event_id);
}

#[tokio::test]
async fn voice_start_context_fence_rejects_binding_or_clear_race_without_admission() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let (session,ready)=create_ready(&store,"voice-start-context").await;
    let input=StrictJsonValue(json!({"content":"explicit queued voice input"}));
    let stale_binding=crate::NativeInputContextFence {binding_version:session.agent_binding.binding_version+1,context_floor:0,supersede_model_step:false};
    assert!(matches!(store.start_voice_turn(&session.agent_session_id,"session_api".into(),"wrong-binding".into(),"wrong-binding-operation".into(),input.clone(),&stale_binding).await,Err(SessionStoreError::ExecutionFenced)));
    let clear=append(&session.agent_session_id,"voice-context-clear","session_api","voice-context-clear","context/cleared",session.agent_session_id.as_ref(),Some(ready),json!({}));
    let cleared=store.append_event(&clear).await.unwrap().cursor.seq;
    let stale=crate::NativeInputContextFence {binding_version:session.agent_binding.binding_version,context_floor:0,supersede_model_step:false};
    assert!(matches!(store.start_voice_turn(&session.agent_session_id,"session_api".into(),"old-context".into(),"old-context-operation".into(),input.clone(),&stale).await,Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.read_turn_receipt(&session.agent_session_id,&"old-context-operation".into()).await.unwrap().status,TurnReceiptStatus::NotFound);
    let exact=crate::NativeInputContextFence {context_floor:cleared,..stale};
    let (message,turn)=store.start_voice_turn(&session.agent_session_id,"session_api".into(),"current-context".into(),"current-context-operation".into(),input.clone(),&exact).await.unwrap();
    let SessionEventPayloadRef::InlineJson(value)=message.record.unwrap().payload else {panic!("voice input inline")};
    assert_eq!(value.0.pointer("/admission/voice_input_context"),Some(&serde_json::to_value(&exact).unwrap()));
    assert!(!turn.duplicate);
    let (_,replay)=store.start_voice_turn(&session.agent_session_id,"session_api".into(),"current-context".into(),"current-context-operation".into(),input,&exact).await.unwrap();assert!(replay.duplicate);
}

#[tokio::test]
async fn voice_immediate_writer_is_explicit_start_only_and_policy_replay_cannot_upgrade_plain_text() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_turn(&store,"voice-policy-writer","plain-policy-turn").await;
    let fence=NativeTurnMutationFence {binding_version:session.agent_binding.binding_version,execution_generation:0};let input=StrictJsonValue(json!({"content":"complete voice correction"}));
    assert!(matches!(store.steer_exact_native_immediate_turn(&session.agent_session_id,&"plain-policy-turn".into(),"plain-upgrade".into(),"session_api".into(),input.clone(),&fence).await,Err(SessionStoreError::ExecutionFenced)));
    store.cancel_exact_turn(&session.agent_session_id,&"plain-policy-turn".into(),"plain-policy-stop".into(),"session_api".into()).await.unwrap();
    let safe=crate::NativeInputContextFence {binding_version:session.agent_binding.binding_version,context_floor:0,supersede_model_step:false};
    assert_eq!(serde_json::to_value(&safe).unwrap(),json!({"binding_version":safe.binding_version,"context_floor":0}),"default voice policy does not add an admission field");
    let immediate=crate::NativeInputContextFence {supersede_model_step:true,..safe};
    let(_,started)=store.start_voice_turn(&session.agent_session_id,"session_api".into(),"explicit-immediate-input".into(),"explicit-immediate-turn".into(),StrictJsonValue(json!({"content":"explicit voice work"})),&immediate).await.unwrap();
    let SessionEventPayloadRef::InlineJson(payload)=started.record.unwrap().payload else{panic!("typed admission")};assert_eq!(payload.0.pointer("/admission/voice_input_context/supersede_model_step"),Some(&json!(true)));
    let(_,first)=store.steer_exact_native_immediate_turn(&session.agent_session_id,&"explicit-immediate-turn".into(),"explicit-correction".into(),"session_api".into(),input.clone(),&fence).await.unwrap();
    let SessionEventPayloadRef::InlineJson(payload)=first.record.unwrap().payload else{panic!("typed correction")};assert_eq!(payload.0["voice_model_step_supersede"],true);
    assert!(matches!(store.steer_exact_native_turn(&session.agent_session_id,&"explicit-immediate-turn".into(),"explicit-correction".into(),"session_api".into(),input.clone(),&fence).await,Err(SessionStoreError::IdempotencyConflict(_))));
    assert!(store.steer_exact_native_immediate_turn(&session.agent_session_id,&"explicit-immediate-turn".into(),"explicit-correction".into(),"session_api".into(),input,&fence).await.unwrap().1.duplicate);
    assert_eq!(store.head(&session.agent_session_id).await.unwrap().active_turn_id.as_deref(),Some("explicit-immediate-turn"));
}
