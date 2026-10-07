use super::*;

#[tokio::test]
async fn voice_input_retention_keeps_original_immutable_wording_after_media_ends(){
    let (_dir,journal)=fixture().await;
    let(link,_)=journal.reserve_trigger("voice".into(),1,"long-lived".into(),"revision-1".into()).await.unwrap();
    journal.record_pending_intent("owner".into(),"session".into(),1,0,link.operation_key.clone(),"original complete work input".into()).await.unwrap();
    journal.close("voice".into(),1,VoiceTermination{reason:VoiceCloseReason::UserEnded,finalization_confirmed:true,message:None}).await.unwrap();
    journal.retain_since(i64::MAX).await.unwrap();
    let(input,duplicate)=journal.record_pending_intent("owner".into(),"session".into(),1,0,link.operation_key,"original complete work input".into()).await.unwrap();
    assert!(duplicate);assert_eq!(input.text,"original complete work input");assert_eq!(input.phase,VoicePendingPhase::Queued);
}
use serde_json::Value;

async fn linked(journal:&VoiceJournal,trigger:&str)->String {
    journal.reserve_trigger("voice".into(),1,trigger.into(),"revision-1".into()).await.unwrap().0.operation_key
}
async fn fixture()->(tempfile::TempDir,VoiceJournal) {
    let dir=tempfile::tempdir().unwrap();let journal=VoiceJournal::open(dir.path()).unwrap();journal.activate(VoiceActivationFact {voice_session_id:"voice".into(),epoch:1,owner_id:"owner".into(),agent_session_id:"session".into(),binding_version:1,context_floor:Some(0),lease_revision:Some("lease".into()),route_digest:"a".repeat(64),started_ms:now_ms()}).await.unwrap();(dir,journal)
}
#[tokio::test]
async fn voice_input_facts_freeze_cross_database_dispatch_and_never_claim_canonical_running() {
    let (dir,journal)=fixture().await;let key=linked(&journal,"input").await;
    let (input,duplicate)=journal.record_pending_intent("owner".into(),"session".into(),1,0,key.clone(),"original complete input".into()).await.unwrap();assert!(!duplicate);assert_eq!(input.phase,VoicePendingPhase::Queued);
    let (revised,_)=journal.control_pending_intent("owner".into(),"session".into(),key.clone(),"correction".into(),Some((1,"corrected complete input".into())),false).await.unwrap();assert_eq!(revised.revision,2);
    assert!(journal.begin_dispatch_intent("owner".into(),"session".into(),key.clone(),1).await.unwrap()==false);
    assert!(journal.begin_dispatch_intent("owner".into(),"session".into(),key.clone(),2).await.unwrap());
    assert!(journal.control_pending_intent("owner".into(),"session".into(),key.clone(),"late-revision".into(),Some((2,"must not create new work".into())),false).await.is_err());
    assert!(journal.queued_intents().await.unwrap().is_empty());let unknown=journal.pending_intent("owner".into(),"session".into(),key.clone()).await.unwrap().unwrap();
    assert_eq!(unknown.phase,VoicePendingPhase::Dispatched);assert!(unknown.canonical_operation_id.is_none());
    assert!(journal.record_pending_intent("other-owner".into(),"session".into(),1,0,key.clone(),"original complete input".into()).await.is_err());
    drop(journal);let journal=VoiceJournal::open(dir.path()).unwrap();assert!(journal.queued_intents().await.unwrap().is_empty(),"an unknown dispatch is lookup-only, not replayed");
    journal.mark_dispatched("owner".into(),"session".into(),key.clone(),2,"real-turn".into(),"real-turn".into()).await.unwrap();
    let actual=journal.pending_intent("owner".into(),"session".into(),key.clone()).await.unwrap().unwrap();assert_eq!(actual.canonical_operation_id.as_deref(),Some("real-turn"));
    let original:Value=journal.run(move|conn|{let body:String=conn.query_row("SELECT payload FROM voice_facts WHERE event_key=?1",[format!("pending:{key}:1")],|row|row.get(0)).map_err(failure)?;serde_json::from_str(&body).map_err(failure)}).await.unwrap();
    assert_eq!(original["text"],"original complete input");assert!(!dir.path().join("nomifun-backend.db").exists());
}
#[tokio::test]
async fn voice_input_cancel_and_context_change_preserve_exact_original_intent() {
    let (_dir,journal)=fixture().await;let key=linked(&journal,"cancel").await;
    journal.record_pending_intent("owner".into(),"session".into(),1,7,key.clone(),"queued text".into()).await.unwrap();
    let (cancelled,duplicate)=journal.control_pending_intent("owner".into(),"session".into(),key.clone(),"cancel-key".into(),None,false).await.unwrap();assert_eq!(cancelled.phase,VoicePendingPhase::Cancelled);assert!(!duplicate);
    assert!(journal.control_pending_intent("owner".into(),"session".into(),key.clone(),"cancel-key".into(),None,false).await.unwrap().1);
    assert!(!journal.begin_dispatch_intent("owner".into(),"session".into(),key.clone(),1).await.unwrap());
    assert!(journal.record_pending_intent("owner".into(),"session".into(),1,8,key.clone(),"queued text".into()).await.is_err(),"a new context cannot change an old input replay");
}

#[tokio::test]
async fn voice_input_original_namespace_survives_restart_and_unknown_lease_is_never_backfilled() {
    let (dir,journal)=fixture().await;let legacy=linked(&journal,"legacy-namespace").await;
    journal.record_pending_intent("owner".into(),"session".into(),1,0,legacy.clone(),"old explicit input".into()).await.unwrap();
    journal.activate(VoiceActivationFact {voice_session_id:"known".into(),epoch:1,owner_id:"owner".into(),agent_session_id:"session".into(),binding_version:1,
        context_floor:Some(0),lease_revision:Some(format!("ns:{}:{}","a".repeat(64),"b".repeat(64))),route_digest:"a".repeat(64),started_ms:now_ms()}).await.unwrap();
    let known=journal.reserve_trigger("known".into(),1,"known-input".into(),"revision-1".into()).await.unwrap().0.operation_key;
    journal.record_pending_intent("owner".into(),"session".into(),1,0,known.clone(),"known explicit input".into()).await.unwrap();
    assert_eq!(journal.original_input_namespace("owner".into(),"session".into(),known.clone()).await.unwrap(),Some("a".repeat(64)),"the credential/profile portion is not canonical input provenance");
    assert!(journal.original_input_namespace("other-owner".into(),"session".into(),known.clone()).await.is_err());
    assert_eq!(journal.original_input_namespace("owner".into(),"session".into(),legacy.clone()).await.unwrap(),None);
    drop(journal);let journal=VoiceJournal::open(dir.path()).unwrap();
    assert_eq!(journal.original_input_namespace("owner".into(),"session".into(),known).await.unwrap(),Some("a".repeat(64)));
    assert_eq!(journal.original_input_namespace("owner".into(),"session".into(),legacy).await.unwrap(),None);
    let unchanged:String=journal.run(|conn|conn.query_row("SELECT lease_revision FROM voice_activations WHERE voice_session_id='voice'",[],|row|row.get(0)).map_err(failure)).await.unwrap();
    assert_eq!(unchanged,"lease","process recovery does not infer a new namespace for old facts");
}

#[tokio::test]
async fn voice_input_frozen_work_policy_survives_restart_and_key_replay_cannot_change_it() {
    use nomifun_voice_contracts::WorkSteeringPolicy;
    let(dir,journal)=fixture().await;let key=linked(&journal,"policy").await;
    let(input,_)=journal.record_pending_intent_with_policy("owner".into(),"session".into(),1,0,key.clone(),"explicit immediate task".into(),WorkSteeringPolicy::SupersedeModelStep).await.unwrap();
    assert_eq!(input.work_steering_policy,WorkSteeringPolicy::SupersedeModelStep);
    assert!(journal.record_pending_intent("owner".into(),"session".into(),1,0,key.clone(),"explicit immediate task".into()).await.is_err());
    drop(journal);let journal=VoiceJournal::open(dir.path()).unwrap();
    assert_eq!(journal.pending_intent("owner".into(),"session".into(),key).await.unwrap().unwrap().work_steering_policy,WorkSteeringPolicy::SupersedeModelStep);
}
