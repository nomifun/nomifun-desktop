fn clockless(id:&str,revision:u64,text:&str)->TranscriptFragment{let mut value=fragment(id,revision,text,0,0);value.media_range=None;value}
fn sourced(id:&str,args:serde_json::Value,source:&str,revision:u64)->WorkTrigger{let mut args=args;args["source_ref"]=json!({"fragment_id":format!("live:user:{source}"),"revision":revision});typed(id,args,None)}

#[tokio::test]
async fn source_target_unknown_late_audio_requires_real_current_context_confirmation_and_never_guesses_a_successor(){
    let(mut bridge,port,_dir)=fixture(false).await;bridge.note_context_target(Some(target("B"))).unwrap();bridge.transcript(clockless("late-a",1,"取消当前任务"));
    assert!(bridge.prepare_trigger(sourced("wrong-old-target",json!({"action":"cancel","target":target("A")}),"late-a",1)).is_err());assert!(bridge.required_source_context().is_none());
    let original=sourced("late-current",json!({"action":"cancel","target":target("B")}),"late-a",1);
    assert_eq!(bridge.prepare_trigger(original.clone()).err().unwrap().kind,VoiceErrorKind::SourceContextRequired);
    let requirement=bridge.required_source_context().unwrap();assert_eq!(requirement.context.target,Some(target("B")));assert!(port.calls.lock().unwrap().is_empty());
    let mut forged=original.clone();let WorkTrigger::TypedToolCall {arguments,..}=&mut forged else{unreachable!()};arguments["context_key"]=json!(requirement.context.context_key);
    assert_eq!(bridge.prepare_trigger(forged).err().unwrap().kind,VoiceErrorKind::SourceContextRequired,"vendor JSON cannot manufacture a user confirmation");
    // Even an obsolete echoed key is data only; the actual human proof uses
    // the service's current key and re-prepares this same immutable target.
    let mut echoed=original;let WorkTrigger::TypedToolCall {arguments,..}=&mut echoed else{unreachable!()};arguments["context_key"]=json!("stale-vendor-echo");assert!(bridge.prepare_trigger(echoed).is_err());
    bridge.confirm_source_context(requirement.source_ref.clone(),&requirement.context.context_key).unwrap();let replay=bridge.take_confirmed_source_trigger().unwrap();
    bridge.trigger(replay).await.unwrap();assert!(matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::Cancel {target:t} if t==&target("B")));assert!(bridge.required_source_context().is_none());
}
#[tokio::test]
async fn source_target_proven_a_cannot_be_confirmed_as_b_and_stale_confirmation_window_is_cleared(){
    let(mut bridge,port,_dir)=fixture(false).await;bridge.transcript(clockless("frozen-a",1,"取消当前任务"));bridge.note_context_target(Some(target("B"))).unwrap();
    assert!(bridge.prepare_trigger(sourced("cannot-retarget",json!({"action":"cancel","target":target("B")}),"frozen-a",1)).is_err());assert!(bridge.required_source_context().is_none());
    bridge.trigger(sourced("exact-a",json!({"action":"cancel","target":target("A")}),"frozen-a",1)).await.unwrap();assert_eq!(port.calls.lock().unwrap().len(),1);
    bridge.transcript(clockless("ambiguous",1,"取消当前任务"));assert!(bridge.prepare_trigger(sourced("context-b",json!({"action":"cancel","target":target("B")}),"ambiguous",1)).is_err());let required=bridge.required_source_context().unwrap();
    bridge.note_context_target(Some(target("C"))).unwrap();assert!(bridge.required_source_context().is_none());assert!(bridge.confirm_source_context(required.source_ref,&required.context.context_key).is_err());assert!(bridge.take_confirmed_source_trigger().is_none());
    bridge.transcript(clockless("revised-confirm",1,"取消当前任务"));assert!(bridge.prepare_trigger(sourced("revision-one-context",json!({"action":"cancel","target":target("C")}),"revised-confirm",1)).is_err());let old=bridge.required_source_context().unwrap();
    assert!(bridge.transcript_revision(clockless("revised-confirm",2,"取消当前任务")).unwrap().is_none());assert!(bridge.required_source_context().is_none());assert!(bridge.confirm_source_context(old.source_ref,&old.context.context_key).is_err());
}
#[tokio::test]
async fn source_target_explicit_task_and_normal_multiple_starts_remain_usable_after_clockless_context_changes(){
    let(mut bridge,port,_dir)=fixture(false).await;bridge.note_context_target(Some(target("B"))).unwrap();bridge.transcript(clockless("incomplete-a",1,"Cancel task A after task B"));
    assert!(bridge.prepare_trigger(sourced("incomplete-explicit-a",json!({"action":"cancel","target":target("A")}),"incomplete-a",1)).is_err(),"an incomplete or scheduled phrase cannot prove an immediate cancellation");
    bridge.transcript(clockless("named-a",1,"Cancel task original-task-A"));
    assert!(bridge.prepare_trigger(sourced("wrong-case-id",json!({"action":"cancel","target":target("original-task-a")}),"named-a",1)).is_err(),"action case does not permit changing the opaque operation identity");
    bridge.trigger(sourced("explicit-a",json!({"action":"cancel","target":target("original-task-A")}),"named-a",1)).await.unwrap();
    for (id,text) in [("new-one","Prepare a new Friday report"),("new-two","Write a new release note")] {
        bridge.transcript(clockless(id,1,text));bridge.trigger(sourced(id,json!({"action":"start","text":format!("Complete task: {text}")}),id,1)).await.unwrap();
    }
    assert_eq!(port.calls.lock().unwrap().iter().filter(|request|matches!(request,VoiceWorkRequest::Start{..})).count(),2);
    bridge.transcript(clockless("relative",1,"取消当前任务"));assert!(bridge.prepare_trigger(sourced("not-new-work",json!({"action":"start","text":"Cancel the current task"}),"relative",1)).is_err());
}
#[tokio::test]
async fn source_revision_new_tool_id_corrects_original_admission_once_and_never_replays_old_text(){
    for queued in [false,true] {
        let(mut bridge,port,_dir)=fixture(queued).await;bridge.transcript(fragment("original",1,"Prepare Friday report",0,10_000));
        let initial=sourced("initial",json!({"action":"start","text":"Prepare Friday report"}),"original",1);bridge.trigger(initial.clone()).await.unwrap();
        bridge.trigger(sourced("initial-alias",json!({"action":"start","text":"Prepare Friday report"}),"original",1)).await.unwrap();
        let generated=bridge.transcript_revision(fragment("original",2,"Prepare Thursday report",0,10_000)).unwrap().unwrap();
        bridge.trigger(sourced("new-call-revised-source",json!({"action":"start","text":"Please compose the report for Thursday"}),"original",2)).await.unwrap();
        bridge.trigger(generated).await.unwrap();assert_eq!(port.calls.lock().unwrap().len(),2);
        if queued {assert!(matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::ReviseQueued {text,expected_revision,..} if text=="Prepare Thursday report"&&*expected_revision==1));}
        else {assert!(matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer {target:t,text} if t==&target("A")&&text=="Prepare Thursday report"));}
        assert!(bridge.trigger(initial).await.unwrap().duplicate);assert_eq!(port.calls.lock().unwrap().len(),2,"old payload replay only looks up the original receipt");
    }
}
#[tokio::test]
async fn source_revision_never_renews_a_nonzero_native_generation_from_a_later_observation(){
    let(mut bridge,port,_dir)=fixture(false).await;bridge.transcript(fragment("bound",1,"Prepare original report",0,10_000));
    let accepted=bridge.trigger(sourced("bound-start",json!({"action":"start","text":"Prepare original report"}),"bound",1)).await.unwrap();
    let mut newer=accepted.clone();newer.target.as_mut().unwrap().execution_generation=2;newer.first_claim_generation=Some(1);bridge.note_receipt(&newer);
    let correction=bridge.transcript_revision(fragment("bound",2,"Prepare revised report",0,10_000)).unwrap().unwrap();
    let WorkTrigger::TypedToolCall {arguments,..}=&correction else{unreachable!()};assert_eq!(arguments["target"]["execution_generation"],1);
    bridge.trigger(correction).await.unwrap();assert!(matches!(&port.calls.lock().unwrap()[1],VoiceWorkRequest::Steer {target,..} if target.execution_generation==1));
    let source=bridge.trigger_sources.values().find(|source|source.admission_operation_key==accepted.operation_key).unwrap();assert_eq!(source.bound_target.as_ref().unwrap().execution_generation,1);
}
#[tokio::test]
async fn source_target_queued_late_lookup_missing_first_claim_proof_does_not_create_a_generation_fence(){
    let(mut bridge,_port,_dir)=fixture(true).await;bridge.transcript(fragment("queued-proof",1,"Prepare the queued report",0,10_000));
    let accepted=bridge.trigger(sourced("queued-original",json!({"action":"start","text":"Prepare the queued report"}),"queued-proof",1)).await.unwrap();
    let mut observation=accepted;observation.target=Some(WorkTarget {execution_generation:2,..target("A")});observation.status=VoiceWorkStatus::Accepted;observation.first_claim_generation=None;
    bridge.note_receipt(&observation);assert!(bridge.trigger_sources.values().all(|source|source.bound_target.is_none()));
    assert!(bridge.transcript_revision(fragment("queued-proof",2,"Prepare the changed queued report",0,10_000)).is_err());
    observation.first_claim_generation=Some(1);bridge.note_receipt(&observation);
    assert!(bridge.trigger_sources.values().all(|source|source.bound_target.as_ref().is_some_and(|target|target.execution_generation==1)),"new real proof binds only the first claim, never the current recovery generation");
}
#[tokio::test]
async fn source_target_opening_zero_advances_only_to_a_proved_first_claim_and_then_stays_frozen(){
    let(mut bridge,_port,_dir)=fixture(true).await;bridge.transcript(fragment("opening",1,"Prepare opening report",0,10_000));
    let accepted=bridge.trigger(sourced("opening-start",json!({"action":"start","text":"Prepare opening report"}),"opening",1)).await.unwrap();
    let mut observation=accepted;observation.target=Some(WorkTarget {execution_generation:0,..target("A")});observation.status=VoiceWorkStatus::Accepted;observation.first_claim_generation=None;bridge.note_receipt(&observation);
    assert!(bridge.trigger_sources.values().all(|source|source.bound_target.as_ref().is_some_and(|target|target.execution_generation==0)));
    observation.target.as_mut().unwrap().execution_generation=1;bridge.note_receipt(&observation);assert!(bridge.trigger_sources.values().all(|source|source.bound_target.as_ref().is_some_and(|target|target.execution_generation==0)));
    observation.first_claim_generation=Some(1);bridge.note_receipt(&observation);assert!(bridge.trigger_sources.values().all(|source|source.bound_target.as_ref().is_some_and(|target|target.execution_generation==1)));
    observation.target.as_mut().unwrap().execution_generation=2;bridge.note_receipt(&observation);assert!(bridge.trigger_sources.values().all(|source|source.bound_target.as_ref().is_some_and(|target|target.execution_generation==1)));
}
#[tokio::test]
async fn delegation_natural_corrections_never_start_another_task_and_explicit_new_work_is_distinct(){
    for text in ["帮我将日期改到周四","不是周五，是周四"] {
        let(mut bridge,port,_dir)=fixture(false).await;bridge.transcript(fragment("correct",1,text,0,1_000_000));bridge.trigger(delegated("correct",1000)).await.unwrap();
        assert!(matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::Steer {target:t,..} if t==&target("A")));
    }
    let(mut bridge,port,_dir)=fixture(false).await;bridge.transcript(fragment("new",1,"新任务：请整理发布说明",0,1_000_000));bridge.trigger(delegated("new",1000)).await.unwrap();assert!(matches!(&port.calls.lock().unwrap()[0],VoiceWorkRequest::Start{..}));
    let(mut bridge,port,_dir)=fixture(false).await;bridge.note_context_target(None).unwrap();bridge.open_context("no-task".into(),Some(VoiceWorkContextFact {target:None})).unwrap();
    let mut source=fragment("unbound-correction",1,"不是周五，是周四",0,1_000_000);source.fragment_id="no-task:user:correction".into();bridge.transcript(source);
    let trigger=WorkTrigger::DelegationTrigger {upstream_trigger_id:"unbound-correction".into(),target:"client".into(),offset_ms:1000,context_window_ref:Some("no-task".into())};assert!(bridge.trigger(trigger).await.is_err());assert!(port.calls.lock().unwrap().is_empty());
}
