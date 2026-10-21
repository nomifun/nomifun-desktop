use super::*;
use nomifun_agent_contracts::{SessionInputDeliveryIntent as Intent,SessionInputQueueAction as Action};

#[tokio::test]
async fn input_placement_is_read_only_and_distinguishes_idle_root_queue_and_exact_steering() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"input-placement").await;
    let sid=&session.agent_session_id;
    let first=save(&store,sid,"first",Intent::Queue {}).await;
    let second=save(&store,sid,"second",Intent::Queue {}).await;
    let cursor=store.current_cursor(sid).await.unwrap();
    let queue=store.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(queue.items[0].input_delivery["placement"],"transcript");
    assert_eq!(queue.items[1].input_delivery["placement"],"queued");
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor,"reading placement never starts a Turn");
    let started=store.claim_session_input_as_turn(&owner(),sid,&first.event_id,&"running".into(),StrictJsonValue(json!({}))).await.unwrap();
    let receipt=store.session_input_delivery(sid,&first.event_id).await.unwrap();
    assert_eq!(receipt["turn_id"],first.event_id.as_ref());assert_eq!(receipt["kind"],"root");
    assert_eq!(receipt["order_seq"],started.record.unwrap().seq);
    let before=store.session_input_queue(&owner(),sid).await.unwrap();
    let promoted=store.update_session_input_queue(&owner(),sid,&"promote".into(),&Action::Promote {message_id:second.event_id.clone(),turn_id:first.event_id.clone()},Some(before.version)).await.unwrap();
    assert_eq!(promoted.items[0].message_id,second.event_id);
    assert_eq!(promoted.items[0].input_delivery["placement"],"steering");
    assert_eq!(promoted.items[0].input_delivery["turn_id"],first.event_id.as_ref());
    store.claim_session_input_as_steering(&owner(),sid,&second.event_id,&"running".into()).await.unwrap();
    let claimed=store.session_input_delivery(sid,&second.event_id).await.unwrap();
    assert_eq!(claimed["state"],"pending","claim is not native delivery");
    assert_eq!(claimed["placement"],"steering");assert_eq!(claimed["kind"],"steering");
    assert_eq!(claimed["turn_id"],first.event_id.as_ref());
}

#[tokio::test]
async fn atomic_queue_replacement_preserves_envelope_position_hold_and_replay_identity() {
    use nomifun_agent_contracts::SessionInputQueueCommand as Command;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"replace-input").await;
    let sid=&session.agent_session_id;
    root(&store,sid,"active-before-edit").await;
    let original=store.accept_session_input(&owner(),sid,"session_api".into(),"edit-original".into(),StrictJsonValue(json!({
        "content":"old body","files":["reference.png"],"inject_skills":["review"],"delivery_intent":{"kind":"queue"}
    }))).await.unwrap().record.unwrap();
    let tail=save(&store,sid,"tail",Intent::Queue {}).await;
    let before=store.session_input_queue(&owner(),sid).await.unwrap();
    let held=store.update_session_input_queue(&owner(),sid,&"hold".into(),&Action::Pause {},Some(before.version)).await.unwrap();
    let command=Command::Replace {message_id:original.event_id.clone(),content:"Edited\n  text".into()};
    let replaced=store.update_session_input_queue_command(&owner(),sid,&"replace".into(),&command,Some(held.version)).await.unwrap();
    assert_eq!(replaced.items.len(),2);assert_eq!(replaced.items[1].message_id,tail.event_id);
    let new=&replaced.items[0];assert_ne!(new.message_id,original.event_id);assert!(new.held);
    assert_eq!(new.input.0["content"],"Edited\n  text");assert_eq!(new.input.0["files"],json!(["reference.png"]));
    assert_eq!(new.input.0["inject_skills"],json!(["review"]));
    let removed=store.session_input_delivery(sid,&original.event_id).await.unwrap();
    assert_eq!(removed["state"],"cancelled");assert_eq!(removed["placement"],"queued","editing a queued item never adds the old body to chat");
    assert_eq!(store.session_input(sid,&original.event_id).await.unwrap().input.0["content"],"old body");
    let replay=store.update_session_input_queue_command(&owner(),sid,&"replace".into(),&command,Some(held.version)).await.unwrap();
    assert_eq!(replay.items[0].message_id,new.message_id);assert_eq!(replay.version,replaced.version);
    let conflict=Command::Replace {message_id:original.event_id,content:"another edit".into()};
    assert!(store.update_session_input_queue_command(&owner(),sid,&"replace".into(),&conflict,Some(held.version)).await.is_err());
    let resumed=store.update_session_input_queue(&owner(),sid,&"resume".into(),&Action::Resume {},Some(replaced.version)).await.unwrap();
    assert!(resumed.items.iter().all(|item|!item.held));
    store.cancel_exact_turn(sid,&"active-before-edit".into(),"end-edit-owner".into(),"session_api".into()).await.unwrap();
    let resumed=store.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(resumed.items[0].input_delivery["placement"],"transcript");
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,new.message_id);
}

#[tokio::test]
async fn replacement_losing_to_claim_cannot_cancel_or_duplicate_the_running_input() {
    use nomifun_agent_contracts::SessionInputQueueCommand as Command;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"replace-claim-race").await;
    let sid=&session.agent_session_id;let input=save(&store,sid,"root",Intent::Queue {}).await;
    let observed=store.session_input_queue(&owner(),sid).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&"original".into(),StrictJsonValue(json!({}))).await.unwrap();
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.update_session_input_queue_command(&owner(),sid,&"late-edit".into(),&Command::Replace {message_id:input.event_id.clone(),content:"late".into()},Some(observed.version)).await.is_err());
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
    assert_eq!(store.session_input_delivery(sid,&input.event_id).await.unwrap()["state"],"claimed");
}

#[tokio::test]
async fn replacement_preserves_position_across_domain_inputs_and_stop_placement_never_regresses() {
    use nomifun_agent_contracts::SessionInputQueueCommand as Command;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"replacement-domain-order").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let first=save(&store,sid,"first",Intent::Queue {}).await;
    let domain=store.accept_session_input(&owner(),sid,"session_api".into(),"domain".into(),StrictJsonValue(json!({"content":"domain-owned input"}))).await.unwrap().record.unwrap();
    let tail=save(&store,sid,"tail",Intent::Queue {}).await;
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    let replaced=store.update_session_input_queue_command(&owner(),sid,&"edit-first".into(),&Command::Replace {message_id:first.event_id,content:"edited first".into()},Some(snapshot.version)).await.unwrap();
    let edited=replaced.items[0].message_id.clone();
    let promoted=store.update_session_input_queue(&owner(),sid,&"guide-first".into(),&Action::Promote {message_id:edited.clone(),turn_id:active.event_id},Some(replaced.version)).await.unwrap();
    let guided_seq=promoted.items[0].input_delivery["event_seq"].as_u64().unwrap();
    store.cancel_exact_turn_with_input_hold(sid,&"active".into(),"stop".into(),"session_api".into()).await.unwrap();
    let stopped=store.session_input_delivery(sid,&edited).await.unwrap();
    assert_eq!(stopped["placement"],"queued");assert!(stopped["event_seq"].as_u64().unwrap()>guided_seq);
    let updates=store.messages_with_turns_after(sid,guided_seq).await.unwrap();
    let updated=updates.iter().find(|row|row.projection_id==format!("message:{}",edited.as_ref())).unwrap();
    assert_eq!(updated.projection["input_delivery"],stopped);
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    store.update_session_input_queue(&owner(),sid,&"resume".into(),&Action::Resume {},Some(snapshot.version)).await.unwrap();
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap().iter().map(|item|item.event.event_id.clone()).collect::<Vec<_>>(),vec![edited.clone(),domain.event_id,tail.event_id]);
    assert_eq!(store.session_input_delivery(sid,&edited).await.unwrap()["placement"],"transcript");
    assert!(store.session_input_delivery(sid,&edited).await.unwrap()["order_seq"].as_u64().unwrap()>guided_seq,
        "a resumed independent root must not return to its earlier queue-save position");
}

#[tokio::test]
async fn queue_pause_and_clear_do_not_withdraw_visible_steering_or_idle_preparation() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-control-scope").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let steering=save(&store,sid,"steering",Intent::PreferCurrent {turn_id:active.event_id}).await;
    let queued=save(&store,sid,"queued",Intent::Queue {}).await;
    let before=store.session_input_queue(&owner(),sid).await.unwrap();
    let paused=store.update_session_input_queue(&owner(),sid,&"pause".into(),&Action::Pause {},Some(before.version)).await.unwrap();
    assert!(!paused.items.iter().find(|item|item.message_id==steering.event_id).unwrap().held);
    assert!(paused.items.iter().find(|item|item.message_id==queued.event_id).unwrap().held);
    let cleared=store.update_session_input_queue(&owner(),sid,&"clear".into(),&Action::Clear {},Some(paused.version)).await.unwrap();
    assert_eq!(cleared.items.len(),1);assert_eq!(cleared.items[0].message_id,steering.event_id);
    assert_eq!(cleared.items[0].input_delivery["placement"],"steering");
    assert_eq!(store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.unwrap()[0].event.event_id,steering.event_id);
    assert_eq!(store.session_input_delivery(sid,&queued.event_id).await.unwrap()["state"],"cancelled");
    store.cancel_exact_turn(sid,&"active".into(),"end-active".into(),"session_api".into()).await.unwrap();
    let starting=save(&store,sid,"starting",Intent::Queue {}).await;
    // The old unclaimed steering becomes the first eligible independent root.
    store.update_session_input_queue(&owner(),sid,&"remove-old".into(),&Action::Remove {message_id:steering.event_id},None).await.unwrap();
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(snapshot.items[0].message_id,starting.event_id);assert_eq!(snapshot.items[0].input_delivery["placement"],"transcript");
    let cleared=store.update_session_input_queue(&owner(),sid,&"clear-queue-only".into(),&Action::Clear {},Some(snapshot.version)).await.unwrap();
    assert_eq!(cleared.items[0].message_id,starting.event_id);
}

async fn save(store:&AgentSessionStore,sid:&AgentSessionId,key:&str,delivery:Intent)->SessionEventRecord {
    store.accept_session_input(&owner(),sid,"session_api".into(),key.into(),StrictJsonValue(json!({
        "content":format!("input {key}"),"files":[],"inject_skills":[],"delivery_intent":delivery,
    }))).await.unwrap().record.unwrap()
}

async fn root(store:&AgentSessionStore,sid:&AgentSessionId,key:&str)->SessionEventRecord {
    let input=save(store,sid,key,Intent::Queue {}).await;
    store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&key.into(),StrictJsonValue(json!({}))).await.unwrap();input
}

#[tokio::test]
async fn queue_never_enters_current_turn_and_steering_crosses_multiple_metadata_pages() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-eligibility").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let mut queued=Vec::new();for n in 0..140 {queued.push(save(&store,sid,&format!("queue-{n}"),Intent::Queue {}).await);}
    let queue=store.session_input_queue(&owner(),sid).await.unwrap();
    store.update_session_input_queue(&owner(),sid,&"promote-after-page".into(),&Action::Promote {message_id:queued[139].event_id.clone(),turn_id:active.event_id.clone()},Some(queue.version)).await.unwrap();
    let supplement=save(&store,sid,"supplement",Intent::PreferCurrent {turn_id:active.event_id}).await;
    let eligible=store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.unwrap();
    assert_eq!(eligible.len(),2);assert_eq!(eligible[0].event.event_id,queued[139].event_id);assert_eq!(eligible[1].event.event_id,supplement.event_id);
    assert!(matches!(store.claim_session_input_as_steering(&owner(),sid,&queued[0].event_id,&"active".into()).await,Err(SessionStoreError::ExecutionFenced)));
    store.claim_session_input_as_steering(&owner(),sid,&supplement.event_id,&"active".into()).await.unwrap();
    let facts=store.chat_causality_facts(sid,&"active".into()).await.unwrap();
    assert!(!facts.events.iter().any(|event|event.event_id==queued[0].event_id));
    store.cancel_exact_turn(sid,&"active".into(),"finish-active".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&queued[0].event_id,&"successor-a".into(),StrictJsonValue(json!({}))).await.unwrap();
    let a_facts=store.chat_causality_facts(sid,&"successor-a".into()).await.unwrap();
    let a_context=crate::canonical_context_messages(&owner(),&a_facts.events,&a_facts.event_payloads,0,0,u64::MAX).unwrap();
    assert!(!a_context.iter().any(|message|message.content=="input queue-1"),"B was saved before A's claim but is not A's context");
    assert!(store.pending_session_inputs_for_steering(sid,&"successor-a".into(),None).await.unwrap().is_empty());
    assert!(store.session_input_claim(sid,&queued[1].event_id).await.unwrap().is_none());
    store.cancel_exact_turn(sid,&"successor-a".into(),"finish-successor-a".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&queued[1].event_id,&"successor-b".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert_eq!(store.session_input_claim(sid,&queued[1].event_id).await.unwrap().unwrap().operation_id.as_ref(),"successor-b");
}

#[tokio::test]
async fn late_prefer_current_retains_original_target_and_only_unclaimed_input_can_start_a_successor() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-target").await;
    let sid=&session.agent_session_id;let old=root(&store,sid,"old").await;
    store.cancel_exact_turn(sid,&"old".into(),"finish-old".into(),"session_api".into()).await.unwrap();
    let other=root(&store,sid,"other").await;
    let late=save(&store,sid,"late",Intent::PreferCurrent {turn_id:old.event_id.clone()}).await;
    assert!(store.pending_session_inputs_for_steering(sid,&"other".into(),None).await.unwrap().is_empty());
    assert!(store.claim_session_input_as_steering(&owner(),sid,&late.event_id,&"other".into()).await.is_err());
    store.cancel_exact_turn(sid,&"other".into(),"finish-other".into(),"session_api".into()).await.unwrap();
    store.claim_session_input_as_turn(&owner(),sid,&late.event_id,&"late-successor".into(),StrictJsonValue(json!({}))).await.unwrap();
    let claimed=save(&store,sid,"claimed",Intent::PreferCurrent {turn_id:late.event_id.clone()}).await;
    store.claim_session_input_as_steering(&owner(),sid,&claimed.event_id,&"late-successor".into()).await.unwrap();
    store.cancel_exact_turn(sid,&"late-successor".into(),"finish-late".into(),"session_api".into()).await.unwrap();
    assert!(store.claim_session_input_as_turn(&owner(),sid,&claimed.event_id,&"must-not-replay".into(),StrictJsonValue(json!({}))).await.is_err());
    assert_eq!(store.session_input_delivery(sid,&claimed.event_id).await.unwrap()["reason"],"TURN_ENDED_BEFORE_INPUT_DELIVERY");
    assert_eq!(store.session_input(sid,&other.event_id).await.unwrap().input.0["delivery_intent"]["kind"],"queue");
}

#[tokio::test]
async fn composer_stop_holds_existing_batch_and_replay_never_holds_new_input() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-stop").await;
    let sid=&session.agent_session_id;root(&store,sid,"active").await;
    let old=save(&store,sid,"old-queued",Intent::Queue {}).await;
    store.cancel_exact_turn_with_input_hold(sid,&"active".into(),"user-stop".into(),"session_api".into()).await.unwrap();
    assert!(store.pending_session_inputs(sid,None).await.unwrap().is_empty());
    assert!(store.claim_session_input_as_turn(&owner(),sid,&old.event_id,&"must-stay-held".into(),StrictJsonValue(json!({}))).await.is_err());
    let new=save(&store,sid,"new-request",Intent::Queue {}).await;
    let before=store.current_cursor(sid).await.unwrap();
    store.cancel_exact_turn_with_input_hold(sid,&"active".into(),"user-stop".into(),"session_api".into()).await.unwrap();
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,new.event_id);
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();assert!(snapshot.items[0].held);assert!(!snapshot.items[1].held);
    store.update_session_input_queue(&owner(),sid,&"resume".into(),&Action::Resume {},Some(snapshot.version)).await.unwrap();
    assert_eq!(store.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,old.event_id);
}

#[tokio::test]
async fn explicit_single_promotion_releases_only_that_held_input() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-promote-held").await;
    let sid=&session.agent_session_id;root(&store,sid,"old-active").await;
    let first=save(&store,sid,"held-first",Intent::Queue {}).await;let second=save(&store,sid,"held-second",Intent::Queue {}).await;
    store.cancel_exact_turn_with_input_hold(sid,&"old-active".into(),"stop".into(),"session_api".into()).await.unwrap();
    let active=root(&store,sid,"new-explicit-task").await;
    let queue=store.session_input_queue(&owner(),sid).await.unwrap();
    let promoted=store.update_session_input_queue(&owner(),sid,&"promote-held".into(),&Action::Promote {message_id:first.event_id.clone(),turn_id:active.event_id},Some(queue.version)).await.unwrap();
    assert!(!promoted.items.iter().find(|item|item.message_id==first.event_id).unwrap().held);
    assert!(promoted.items.iter().find(|item|item.message_id==second.event_id).unwrap().held);
    let eligible=store.pending_session_inputs_for_steering(sid,&"new-explicit-task".into(),None).await.unwrap();
    assert_eq!(eligible.len(),1);assert_eq!(eligible[0].event.event_id,first.event_id);
}

#[tokio::test]
async fn queue_controls_preserve_original_identity_and_cancelled_key_cannot_revive() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-controls").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let first=save(&store,sid,"first",Intent::Queue {}).await;let second=save(&store,sid,"second",Intent::Queue {}).await;
    let initial=store.session_input_queue(&owner(),sid).await.unwrap();
    let reordered=store.update_session_input_queue(&owner(),sid,&"reorder".into(),&Action::Reorder {message_ids:vec![second.event_id.clone(),first.event_id.clone()]},Some(initial.version)).await.unwrap();
    assert_eq!(reordered.items[0].message_id,second.event_id);
    assert!(store.update_session_input_queue(&owner(),sid,&"stale-remove".into(),&Action::Remove {message_id:first.event_id.clone()},Some(initial.version)).await.is_err());
    let promoted=store.update_session_input_queue(&owner(),sid,&"promote".into(),&Action::Promote {message_id:second.event_id.clone(),turn_id:active.event_id},Some(reordered.version)).await.unwrap();
    assert_eq!(promoted.items.len(),2);assert_eq!(promoted.items[0].message_id,second.event_id);
    assert_eq!(store.session_input(sid,&second.event_id).await.unwrap().input.0["delivery_intent"]["kind"],"queue","original accepted intent is immutable");
    assert_eq!(store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.unwrap()[0].event.event_id,second.event_id);
    let removed=store.update_session_input_queue(&owner(),sid,&"remove".into(),&Action::Remove {message_id:first.event_id.clone()},Some(promoted.version)).await.unwrap();
    assert_eq!(removed.items.len(),1);
    let replay=save(&store,sid,"first",Intent::Queue {}).await;assert_eq!(replay.event_id,first.event_id);
    assert_eq!(store.session_input_delivery(sid,&first.event_id).await.unwrap()["state"],"cancelled");
    let replayed=store.update_session_input_queue(&owner(),sid,&"remove".into(),&Action::Remove {message_id:first.event_id},Some(promoted.version)).await.unwrap();
    assert_eq!(replayed.items.len(),1);
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.read_session_input_queue_receipt(&owner(),sid,&"promote".into(),&Action::Promote {
        message_id:second.event_id,turn_id:store.turn_accepted_input(sid,&"active".into()).await.unwrap().0.event_id,
    },Some(reordered.version)).await.unwrap().is_some());
    assert!(store.read_session_input_queue_receipt(&owner(),sid,&"new-key".into(),&Action::Clear {},Some(replayed.version)).await.unwrap().is_none());
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
}

#[tokio::test]
async fn reordered_or_held_preparation_loses_its_claim_without_an_execution_error() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-reorder-claim").await;
    let sid=&session.agent_session_id;root(&store,sid,"prior-owner").await;
    let first=save(&store,sid,"first",Intent::Queue {}).await;let second=save(&store,sid,"second",Intent::Queue {}).await;
    let initial=store.session_input_queue(&owner(),sid).await.unwrap();
    let ordered=store.update_session_input_queue(&owner(),sid,&"reverse".into(),&Action::Reorder {message_ids:vec![second.event_id.clone(),first.event_id.clone()]},Some(initial.version)).await.unwrap();
    assert!(store.try_claim_session_input_as_turn(&owner(),sid,&first.event_id,&"stale-first".into(),StrictJsonValue(json!({}))).await.unwrap().is_none());
    store.update_session_input_queue(&owner(),sid,&"hold".into(),&Action::Pause {},Some(ordered.version)).await.unwrap();
    store.cancel_exact_turn(sid,&"prior-owner".into(),"release-prior".into(),"session_api".into()).await.unwrap();
    let paused=store.session_input_queue(&owner(),sid).await.unwrap();
    assert!(store.try_claim_session_input_as_turn(&owner(),sid,&second.event_id,&"held-second".into(),StrictJsonValue(json!({}))).await.unwrap().is_none());
    store.update_session_input_queue(&owner(),sid,&"resume".into(),&Action::Resume {},Some(paused.version)).await.unwrap();
    assert!(store.try_claim_session_input_as_turn(&owner(),sid,&second.event_id,&"actual-second".into(),StrictJsonValue(json!({}))).await.unwrap().is_some());
}

#[tokio::test]
async fn replaying_an_old_resume_never_releases_a_later_hold() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-resume-replay").await;
    let sid=&session.agent_session_id;root(&store,sid,"busy-owner").await;save(&store,sid,"queued",Intent::Queue {}).await;
    let first=store.session_input_queue(&owner(),sid).await.unwrap();
    let held=store.update_session_input_queue(&owner(),sid,&"hold-first".into(),&Action::Pause {},Some(first.version)).await.unwrap();
    let resumed=store.update_session_input_queue(&owner(),sid,&"resume-original".into(),&Action::Resume {},Some(held.version)).await.unwrap();
    let later=store.update_session_input_queue(&owner(),sid,&"hold-again".into(),&Action::Pause {},Some(resumed.version)).await.unwrap();
    let replay=store.update_session_input_queue(&owner(),sid,&"resume-original".into(),&Action::Resume {},Some(held.version)).await.unwrap();
    assert_eq!(replay.version,later.version);assert!(replay.items[0].held);
}

#[tokio::test]
async fn pending_remove_after_claim_never_cancels_the_actual_turn() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-remove-race").await;
    let sid=&session.agent_session_id;let input=save(&store,sid,"original",Intent::Queue {}).await;
    let version=store.session_input_queue(&owner(),sid).await.unwrap().version;
    store.claim_session_input_as_turn(&owner(),sid,&input.event_id,&"actual-owner".into(),StrictJsonValue(json!({}))).await.unwrap();
    assert!(store.update_session_input_queue(&owner(),sid,&"late-remove".into(),&Action::Remove {message_id:input.event_id},Some(version)).await.is_err());
    assert_eq!(store.head(sid).await.unwrap().active_turn_id.as_deref(),Some("actual-owner"));
    assert_eq!(store.read_turn_receipt(sid,&"actual-owner".into()).await.unwrap().status,TurnReceiptStatus::Running);
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn concurrent_remove_and_claim_have_one_winner_without_stopping_a_raced_owner() {
    let store=AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();let(session,_)=create_ready(&store,"queue-concurrent-remove").await;
    let sid=&session.agent_session_id;let input=save(&store,sid,"racing-input",Intent::Queue {}).await;
    let version=store.session_input_queue(&owner(),sid).await.unwrap().version;
    let remove_action=Action::Remove {message_id:input.event_id.clone()};let remove_key="racing-remove".into();let claim_operation="racing-owner".into();
    let principal=owner();
    let(removed,claimed)=tokio::join!(
        store.update_session_input_queue(&principal,sid,&remove_key,&remove_action,Some(version)),
        store.claim_session_input_as_turn(&principal,sid,&input.event_id,&claim_operation,StrictJsonValue(json!({}))),
    );
    assert_ne!(removed.is_ok(),claimed.is_ok(),"the original writer must serialize cancellation and claim");
    if claimed.is_ok() {
        assert_eq!(store.read_turn_receipt(sid,&claim_operation).await.unwrap().status,TurnReceiptStatus::Running);
    } else {
        assert_eq!(store.session_input_delivery(sid,&input.event_id).await.unwrap()["state"],"cancelled");
        assert!(store.head(sid).await.unwrap().active_turn_id.is_none());
    }
}

#[tokio::test]
async fn original_submission_freezes_mode_and_rejects_cross_session_observed_targets() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-frozen").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let input=save(&store,sid,"same-key",Intent::Queue {}).await;
    let mut changed=store.session_input(sid,&input.event_id).await.unwrap().input.0;changed["delivery_intent"]=json!({"kind":"prefer_current","turn_id":active.event_id});
    assert!(matches!(store.accept_session_input(&owner(),sid,"session_api".into(),"same-key".into(),StrictJsonValue(changed)).await,Err(SessionStoreError::IdempotencyConflict(_))));
    let(other,_)=create_ready(&store,"different-session").await;
    let bad=StrictJsonValue(json!({"content":"must reject","delivery_intent":{"kind":"prefer_current","turn_id":active.event_id}}));
    assert!(store.accept_session_input(&owner(),&other.agent_session_id,"session_api".into(),"wrong-target".into(),bad).await.is_err());
    assert!(store.pending_session_inputs(&other.agent_session_id,None).await.unwrap().is_empty());
}

#[tokio::test]
async fn blocked_input_rejects_new_promotion_but_replays_its_original_command() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-blocked-promotion").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;let input=save(&store,sid,"queued",Intent::Queue {}).await;
    let original=store.session_input_queue(&owner(),sid).await.unwrap();
    let action=Action::Promote {message_id:input.event_id.clone(),turn_id:active.event_id};
    store.update_session_input_queue(&owner(),sid,&"original-promotion".into(),&action,Some(original.version)).await.unwrap();
    store.mark_session_input_blocked(sid,&input.event_id,crate::SessionInputBlockedReason::SkillUnavailable).await.unwrap();
    let blocked=store.session_input_queue(&owner(),sid).await.unwrap();let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.update_session_input_queue(&owner(),sid,&"new-promotion".into(),&action,Some(blocked.version)).await.is_err());
    let replay=store.update_session_input_queue(&owner(),sid,&"original-promotion".into(),&action,Some(original.version)).await.unwrap();
    assert_eq!(replay.items[0].input_delivery["state"],"blocked");assert_eq!(store.current_cursor(sid).await.unwrap(),cursor);
}

#[tokio::test]
async fn malformed_routing_metadata_fails_closed_before_any_steering_claim() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-corrupt-routing").await;
    let sid=&session.agent_session_id;root(&store,sid,"active").await;let input=save(&store,sid,"queued",Intent::Queue {}).await;
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.delivery_intent',json('{\"kind\":\"unknown\"}')) WHERE session_id=? AND event_id=?")
        .bind(sid.as_ref()).bind(input.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
    let cursor=store.current_cursor(sid).await.unwrap();
    assert!(store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.is_err());
    assert!(store.session_input_claim(sid,&input.event_id).await.is_err(),"all typed reads must reject the damaged accepted input");
    assert_eq!(store.current_cursor(sid).await.unwrap(),cursor,"failure cannot append a claim or replace the damaged input");
}

#[tokio::test]
async fn corrupted_observed_target_is_rejected_on_reads_and_never_becomes_a_successor() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-corrupt-target").await;
    let sid=&session.agent_session_id;root(&store,sid,"active").await;let input=save(&store,sid,"queued",Intent::Queue {}).await;
    let original=store.session_input(sid,&input.event_id).await.unwrap().input.0;
    let(other,_)=create_ready(&store,"queue-foreign-target").await;let foreign=root(&store,&other.agent_session_id,"foreign-active").await;
    for target in ["invalid-target".to_owned(),Uuid::now_v7().to_string(),foreign.event_id.as_ref().to_owned()] {
        let mut corrupted=original.clone();corrupted["delivery_intent"]=json!({"kind":"prefer_current","turn_id":target});
        sqlx::query("UPDATE agent_events SET inline_json=? WHERE session_id=? AND event_id=?")
            .bind(serde_json::to_string(&corrupted).unwrap()).bind(sid.as_ref()).bind(input.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        assert!(store.session_input(sid,&input.event_id).await.is_err());
        assert!(store.pending_session_inputs(sid,None).await.is_err());
        assert!(store.try_claim_session_input_as_turn(&owner(),sid,&input.event_id,&"must-not-run".into(),StrictJsonValue(json!({}))).await.is_err());
    }
}

#[tokio::test]
async fn queue_pending_order_and_stop_hold_survive_database_restart() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("queue-restart.db");
    let database=nomifun_db::init_database(&path).await.unwrap();
    let store=AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();let(session,_)=create_ready(&store,"queue-restart").await;
    let sid=&session.agent_session_id;root(&store,sid,"active").await;
    let first=save(&store,sid,"first",Intent::Queue {}).await;let second=save(&store,sid,"second",Intent::Queue {}).await;
    let version=store.session_input_queue(&owner(),sid).await.unwrap().version;
    store.update_session_input_queue(&owner(),sid,&"reorder".into(),&Action::Reorder {message_ids:vec![second.event_id.clone(),first.event_id.clone()]},Some(version)).await.unwrap();
    store.cancel_exact_turn_with_input_hold(sid,&"active".into(),"stop-before-exit".into(),"session_api".into()).await.unwrap();
    let before=store.session_input_queue(&owner(),sid).await.unwrap();
    drop(store);database.close().await;
    let reopened=AgentSessionStore::connect_existing(&path).await.unwrap();
    let restored=reopened.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(restored.version,before.version);assert_eq!(restored.items[0].message_id,second.event_id);
    assert!(restored.items.iter().all(|input|input.held));assert!(reopened.pending_session_inputs(sid,None).await.unwrap().is_empty());
    reopened.rebuild_projections(sid).await.unwrap();
    assert_eq!(reopened.session_input_queue(&owner(),sid).await.unwrap().version,before.version);
    reopened.update_session_input_queue(&owner(),sid,&"resume-after-restart".into(),&Action::Resume {},Some(restored.version)).await.unwrap();
    assert_eq!(reopened.pending_session_inputs(sid,None).await.unwrap()[0].event.event_id,second.event_id);
}

#[tokio::test]
async fn mixed_creation_facts_are_not_queue_inputs_and_runtime_logs_do_not_change_queue_version() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-mixed-projection").await;
    let sid=&session.agent_session_id;
    store.start_turn(sid,"session_api".into(),"creation-domain".into(),"creation-turn".into(),StrictJsonValue(json!({"content":"domain creation input"}))).await.unwrap();
    store.cancel_exact_turn(sid,&"creation-turn".into(),"close-creation".into(),"session_api".into()).await.unwrap();
    let active=root(&store,sid,"chat-active").await;
    let queued=save(&store,sid,"chat-queued",Intent::Queue {}).await;
    let before=store.session_input_queue(&owner(),sid).await.unwrap();
    assert_eq!(before.items.len(),1);assert_eq!(before.items[0].message_id,queued.event_id);
    let cause=store.session_input_claim(sid,&active.event_id).await.unwrap().unwrap().claim_event_id;
    store.append_event(&append(sid,"runtime-progress-without-queue-change","runtime_supervisor","runtime-progress-without-queue-change",
        "runtime/progress-recorded","chat-active",Some(cause),json!({"producer_seq":1,"event":{"event":"text_delta","text":"ongoing model text"}}))).await.unwrap();
    assert_eq!(store.session_input_queue(&owner(),sid).await.unwrap().version,before.version);
}

#[tokio::test]
async fn queue_control_and_delivery_metadata_are_not_canonical_context_text() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"queue-context-control").await;
    let sid=&session.agent_session_id;root(&store,sid,"active").await;
    let a=save(&store,sid,"a",Intent::Queue {}).await;let b=save(&store,sid,"b",Intent::Queue {}).await;
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    let paused=store.update_session_input_queue(&owner(),sid,&"pause".into(),&Action::Pause {},Some(snapshot.version)).await.unwrap();
    store.update_session_input_queue(&owner(),sid,&"reorder".into(),&Action::Reorder {message_ids:vec![b.event_id,a.event_id]},Some(paused.version)).await.unwrap();
    let page=store.read_events(sid,None,100).await.unwrap();
    assert!(page.events.iter().any(|event|event.kind.0=="message/input-queue-changed"));
    let payloads=store.resolved_event_payloads(sid,&page.events.iter().map(|event|event.event_id.clone()).collect::<Vec<_>>()).await.unwrap();
    // Exercise the canonical DATA reader across the complete fact set, with
    // claimed roots rendered as text rather than supplied by native replay.
    let context=crate::canonical_context_messages(&owner(),&page.events,&payloads,0,page.next_cursor.seq,u64::MAX).unwrap();
    assert_eq!(context.len(),1);assert_eq!(context[0].content,"input active");
}

#[tokio::test]
async fn queue_routing_fold_does_not_hydrate_unselected_replacement_envelopes_but_selected_ancestry_fails_closed() {
    use nomifun_agent_contracts::SessionInputQueueCommand as Command;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"routing-body-isolation").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let original=save(&store,sid,"original",Intent::Queue {}).await;
    let tail=save(&store,sid,"tail",Intent::Queue {}).await;
    let original_body=store.session_input(sid,&original.event_id).await.unwrap().input.0;
    let mut replacement=original.event_id.clone();
    for (key,content) in [("edit-one","first revision"),("edit-two","second revision")] {
        let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
        let edited=store.update_session_input_queue_command(&owner(),sid,&key.into(),&Command::Replace {
            message_id:replacement,content:content.into()},Some(snapshot.version)).await.unwrap();
        replacement=edited.items.iter().find(|item|item.input.0["content"]==content).unwrap().message_id.clone();
    }
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    store.update_session_input_queue(&owner(),sid,&"reorder".into(),&Action::Reorder {
        message_ids:vec![replacement.clone(),tail.event_id]},Some(snapshot.version)).await.unwrap();
    let supplement=save(&store,sid,"supplement",Intent::PreferCurrent {turn_id:active.event_id}).await;
    // The exact edited lineage is now damaged in a non-routing field. An
    // unrelated current-turn input must not decode it just to fold controls.
    let mut damaged=original_body;damaged["files"]=json!(["changed-after-acceptance.txt"]);
    sqlx::query("UPDATE agent_events SET inline_json=? WHERE session_id=? AND event_id=?")
        .bind(serde_json::to_string(&damaged).unwrap()).bind(sid.as_ref()).bind(original.event_id.as_ref())
        .execute(store.test_pool()).await.unwrap();
    let eligible=store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.unwrap();
    assert_eq!(eligible.iter().map(|input|&input.event.event_id).collect::<Vec<_>>(),vec![&supplement.event_id]);
    store.claim_session_input_as_steering(&owner(),sid,&supplement.event_id,&"active".into()).await.unwrap();
    assert_eq!(store.session_input_delivery(sid,&supplement.event_id).await.unwrap()["placement"],"steering");
    assert!(store.session_input(sid,&replacement).await.is_err(),
        "selection must still verify every original envelope in the edited lineage");
}

#[tokio::test]
async fn queue_routing_fold_rejects_changed_replacement_provenance_before_steering() {
    use nomifun_agent_contracts::SessionInputQueueCommand as Command;
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"routing-provenance").await;
    let sid=&session.agent_session_id;let active=root(&store,sid,"active").await;
    let original=save(&store,sid,"original",Intent::Queue {}).await;
    let snapshot=store.session_input_queue(&owner(),sid).await.unwrap();
    let edited=store.update_session_input_queue_command(&owner(),sid,&"edit".into(),&Command::Replace {
        message_id:original.event_id,content:"revision".into()},Some(snapshot.version)).await.unwrap();
    let replacement=&edited.items[0].message_id;
    save(&store,sid,"supplement",Intent::PreferCurrent {turn_id:active.event_id}).await;
    sqlx::query("UPDATE agent_events SET correlation_id='different-input' WHERE session_id=? AND event_id=?")
        .bind(sid.as_ref()).bind(replacement.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.pending_session_inputs_for_steering(sid,&"active".into(),None).await.is_err(),
        "routing-only reads still reject altered canonical input identity");
}
