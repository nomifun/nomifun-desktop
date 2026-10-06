use super::*;
use nomifun_agent_contracts::{
    IdmmDecisionExplanation, IdmmDecisionNotice, IdmmDecisionNoticeStatus, IdmmDecisionSource,
    IdmmQuestionRef,
};

async fn question_fixture(store: &AgentSessionStore, key: &str) -> (AgentSessionLiveRecord, IdmmQuestionRef) {
    let (session, started) = create_turn(store, key, &format!("turn-{key}")).await;
    let message_id = Uuid::now_v7().to_string();
    let text = "你希望使用哪个方案？";
    let part = append(&session.agent_session_id, &format!("part-{key}"), "runtime-supervisor", &format!("part-{key}"),
        "message/content-part", &message_id, Some(started), json!({"content":text,"turn_id":format!("turn-{key}")}));
    store.append_event(&part).await.unwrap();
    let completed = append(&session.agent_session_id, &format!("message-completed-{key}"), "runtime-supervisor", &format!("message-completed-{key}"),
        "message/completed", &message_id, Some(part.event_id), json!({"part_count":1,"content_digest":digest_bytes(text.as_bytes())}));
    store.append_event(&completed).await.unwrap();
    let terminal = append(&session.agent_session_id, &format!("terminal-{key}"), "runtime-supervisor", &format!("terminal-{key}"),
        "turn/completed", &format!("turn-{key}"), Some(completed.event_id), json!({}));
    store.append_event(&terminal).await.unwrap();
    let projection = store.messages_after(&session.agent_session_id, 0).await.unwrap().into_iter()
        .find(|message| message.projection_id == format!("message:{message_id}")).unwrap();
    let question = IdmmQuestionRef { message_id, sequence: projection.last_seq, fingerprint: projection.semantic_digest };
    (session, question)
}

fn explanation(question: &IdmmQuestionRef) -> IdmmDecisionExplanation {
    IdmmDecisionExplanation {
        intervention_id: Uuid::now_v7().to_string(), source: IdmmDecisionSource::Rule,
        reason_code: "rule_cannot_answer".into(), rationale: "需要你补充决定依据".into(),
        model: None, question: Some(question.clone()),
    }
}

fn notice(question: &IdmmQuestionRef) -> IdmmDecisionNotice {
    IdmmDecisionNotice { decision: explanation(question), status: IdmmDecisionNoticeStatus::WaitingForHuman, created_at: 1_788_000_000_001 }
}

#[tokio::test]
async fn idmm_notice_is_canonical_metadata_and_never_changes_question_or_runtime_context() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, question) = question_fixture(&store, "idmm-notice").await;
    let original = store.messages_after(&session.agent_session_id, 0).await.unwrap();
    let notice = notice(&question);
    let result = store.append_idmm_notice(&owner(), &session.agent_session_id, "notice-once", notice.clone()).await.unwrap().unwrap();
    assert!(!result.duplicate);
    let event = result.record.unwrap();
    assert_eq!(event.kind.0, "idmm/notice-recorded");
    assert_eq!(event.event_id.as_ref(), notice.decision.intervention_id);
    assert_eq!(event.correlation_id.as_ref(), event.event_id.as_ref());
    let messages = store.messages_after(&session.agent_session_id, 0).await.unwrap();
    for before in original {
        let after = messages.iter().find(|message| message.projection_id == before.projection_id).unwrap();
        assert_eq!(after, &before);
    }
    let projected = messages.iter().find(|message| message.presentation_intent == "idmm_notice").unwrap();
    assert_eq!(projected.projection["reference"], serde_json::to_value(&notice).unwrap());
    assert!(projected.projection.get("content").is_none());
    assert_eq!(store.head(&session.agent_session_id).await.unwrap().status, "ready");
    let payload = serde_json::to_value(&notice).unwrap();
    let rebuilt = reduce_agent_messages(None, &event, &payload).unwrap();
    assert_eq!(&rebuilt, projected);
    let payloads = BTreeMap::from([(event.event_id.0.clone(), payload)]);
    assert!(crate::canonical_context_messages(&[event], &payloads, 0, u64::MAX, u64::MAX).unwrap().is_empty());
}

#[tokio::test]
async fn idmm_notice_replay_retains_first_metadata_but_conflicts_on_question_or_status() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, question) = question_fixture(&store, "idmm-replay").await;
    let original = notice(&question);
    let first = store.append_idmm_notice(&owner(), &session.agent_session_id, "once", original.clone()).await.unwrap().unwrap();
    let mut redelivery = original.clone();
    redelivery.decision.intervention_id = Uuid::now_v7().to_string();
    redelivery.decision.rationale = "再次检查后仍需你决定".into();
    redelivery.created_at += 10;
    let repeated = store.append_idmm_notice(&owner(), &session.agent_session_id, "once", redelivery.clone()).await.unwrap().unwrap();
    assert!(repeated.duplicate);
    assert_eq!(first.record, repeated.record);
    redelivery.status = IdmmDecisionNoticeStatus::Failed;
    assert!(matches!(store.append_idmm_notice(&owner(), &session.agent_session_id, "once", redelivery.clone()).await, Err(SessionStoreError::IdempotencyConflict(_))));
    redelivery.status = original.status;
    redelivery.decision.question.as_mut().unwrap().sequence += 1;
    assert!(matches!(store.append_idmm_notice(&owner(), &session.agent_session_id, "once", redelivery).await, Err(SessionStoreError::IdempotencyConflict(_))));
    store.start_turn(&session.agent_session_id, "session-api".into(), "human-after-notice".into(), "human-next".into(), StrictJsonValue(json!({"content":"使用方案二"}))).await.unwrap();
    assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "once", original.clone()).await.unwrap().unwrap().duplicate);
    assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "late", original).await.unwrap().is_none());
}

#[tokio::test]
async fn idmm_notice_requires_owner_and_exact_unanswered_question() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, question) = question_fixture(&store, "idmm-guards").await;
    let foreign = PrincipalRef { principal_kind: "user".into(), principal_id: "another-user".into() };
    assert!(matches!(store.append_idmm_notice(&foreign, &session.agent_session_id, "foreign", notice(&question)).await, Err(SessionStoreError::Conflict(_))));
    let mut stale = notice(&question);
    stale.decision.question.as_mut().unwrap().fingerprint = "b".repeat(64);
    assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "stale-digest", stale).await.unwrap().is_none());
    let mut stale = notice(&question);
    stale.decision.question.as_mut().unwrap().sequence += 1;
    assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "stale-sequence", stale).await.unwrap().is_none());
    assert!(!store.messages_after(&session.agent_session_id, 0).await.unwrap().iter().any(|message| message.presentation_intent == "idmm_notice"));
}

#[tokio::test]
async fn idmm_input_retains_explanation_and_replay_before_enforcing_question_cas() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, question) = question_fixture(&store, "idmm-input").await;
    let decision = explanation(&question);
    let input = StrictJsonValue(json!({"content":"2","origin":"idmm","idmm_decision":decision}));
    let (first, _) = store.start_turn(&session.agent_session_id, "session-api".into(), "idmm-answer".into(), "idmm-answer-operation".into(), input.clone()).await.unwrap();
    let (replay, _) = store.start_turn(&session.agent_session_id, "session-api".into(), "idmm-answer".into(), "idmm-answer-operation".into(), input.clone()).await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(first.record, replay.record);
    let root = first.record.unwrap();
    let projected = store.messages_after(&session.agent_session_id, 0).await.unwrap().into_iter()
        .find(|message| message.projection_id == format!("message:{}", root.event_id.as_ref())).unwrap();
    assert_eq!(projected.projection["idmm_decision"], input.0["idmm_decision"]);
    assert_eq!(projected.projection["content"], "2");
    assert!(matches!(store.start_turn(&session.agent_session_id, "session-api".into(), "new-idmm-answer".into(), "new-idmm-operation".into(), input).await, Err(SessionStoreError::Conflict(_))));
}

#[test]
fn idmm_projection_rejects_malformed_explanations_and_never_recovers_from_origin() {
    let session = session_id();
    let event = projection_event(&session, 3, "accepted-root", "message/user-accepted", "accepted-root", json!({}));
    let origin_only = reduce_agent_messages(None, &event, &json!({"content":"2","origin":"idmm"})).unwrap();
    assert!(origin_only.projection.get("idmm_decision").is_none());
    assert!(reduce_agent_messages(None, &event, &json!({"content":"2","idmm_decision":{"confidence":1.0}})).is_err());
}

#[tokio::test]
async fn idmm_question_cannot_cross_context_clear_or_agent_transition() {
    for clear_context in [true, false] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let suffix = if clear_context { "clear" } else { "agent-switch" };
        let (session, question) = question_fixture(&store, suffix).await;
        let original = notice(&question);
        store.append_idmm_notice(&owner(), &session.agent_session_id, "before-boundary", original.clone()).await.unwrap().unwrap();
        if clear_context {
            let cause = store.read_events(&session.agent_session_id, None, 500).await.unwrap().events.last().unwrap().event_id.clone();
            store.append_event(&append(&session.agent_session_id, "clear-idmm-context", "session-api", "clear-idmm-context",
                "context/cleared", session.agent_session_id.as_ref(), Some(cause), json!({"reason":"user_requested"}))).await.unwrap();
        } else {
            let expected = session.agent_binding.clone();
            let replacement = agent_replacement(&expected, "idmm-target");
            store.replace_session_agent_binding(&owner(), &session.agent_session_id,
                agent_switch(expected, replacement, "idmm-target-switch")).await.unwrap();
        }
        assert_eq!(store.head(&session.agent_session_id).await.unwrap().status, "ready");
        assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "after-boundary", notice(&question)).await.unwrap().is_none());
        let input = StrictJsonValue(json!({"content":"2","idmm_decision":explanation(&question)}));
        assert!(matches!(store.start_turn(&session.agent_session_id, "session-api".into(), "cross-boundary-answer".into(), "cross-boundary-turn".into(), input).await,
            Err(SessionStoreError::Conflict(_))));
        assert!(store.append_idmm_notice(&owner(), &session.agent_session_id, "before-boundary", original).await.unwrap().unwrap().duplicate);
    }
}

#[tokio::test]
async fn idmm_notice_accepts_formal_projected_assistant_questions_without_a_native_turn() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready) = create_ready(&store, "idmm-domain-question").await;
    let message_id = Uuid::now_v7().to_string();
    let projected = append(&session.agent_session_id, &message_id, "session-api", "domain-question",
        "message/assistant-projected", &message_id, Some(ready), json!({"content":"你希望报告包含哪些资料？"}));
    store.append_event(&projected).await.unwrap();
    let before = store.messages_after(&session.agent_session_id, 0).await.unwrap().into_iter()
        .find(|message| message.projection_id == format!("message:{message_id}")).unwrap();
    let question = IdmmQuestionRef { message_id, sequence: before.last_seq, fingerprint: before.semantic_digest.clone() };
    let result = store.append_idmm_notice(&owner(), &session.agent_session_id, "domain-question-notice", notice(&question)).await.unwrap().unwrap();
    assert_eq!(result.record.unwrap().causation_event_id, Some(projected.event_id));
    assert_eq!(store.messages_after(&session.agent_session_id, 0).await.unwrap().into_iter()
        .find(|message| message.projection_id == before.projection_id).unwrap(), before);
    let input = StrictJsonValue(json!({"content":"公开资料即可","idmm_decision":explanation(&question)}));
    assert!(store.start_turn(&session.agent_session_id, "session-api".into(), "domain-question-answer".into(),
        "domain-question-turn".into(), input).await.is_ok());
}

#[tokio::test]
async fn idmm_renderer_history_paginates_counts_and_rebuilds_notices_without_exposing_them_to_message_readers() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready) = create_ready(&store, "idmm-history-notices").await;
    let message_id = Uuid::now_v7().to_string();
    store.append_event(&append(&session.agent_session_id, &message_id, "session-api", "history-question",
        "message/assistant-projected", &message_id, Some(ready), json!({"content":"你希望缓存多久？"}))).await.unwrap();
    let source = store.messages_before(&session.agent_session_id, None, 1).await.unwrap().0.remove(0);
    let question = IdmmQuestionRef { message_id, sequence: source.last_seq, fingerprint: source.semantic_digest.clone() };
    store.append_idmm_notice(&owner(), &session.agent_session_id, "history-wait", notice(&question)).await.unwrap().unwrap();
    let mut failed = notice(&question);
    failed.status = IdmmDecisionNoticeStatus::Failed;
    store.append_idmm_notice(&owner(), &session.agent_session_id, "history-failure", failed).await.unwrap().unwrap();
    let mut original_pages = None;
    for rebuild in [false, true] {
        if rebuild { store.rebuild_projections(&session.agent_session_id).await.unwrap(); }
        let mut cursor = None;
        let mut pages = Vec::new();
        for has_more in [true, true, false] {
            let page = store.message_history_before(&session.agent_session_id, cursor, 1).await.unwrap();
            assert_eq!(page.0.len(), 1);
            assert_eq!(page.1, has_more);
            assert_eq!(page.2, 3);
            cursor = Some(page.0[0].first_seq);
            pages.push(page);
        }
        assert_eq!(pages[0].0[0].presentation_intent, "idmm_notice");
        assert_eq!(pages[0].0[0].projection["reference"]["status"], "failed");
        assert_eq!(pages[1].0[0].presentation_intent, "idmm_notice");
        assert_eq!(pages[1].0[0].projection["reference"]["status"], "waiting_for_human");
        assert_eq!(pages[2].0[0], source);
        if let Some(expected) = &original_pages { assert_eq!(&pages, expected); }
        else { original_pages = Some(pages); }
        let messages = store.messages_before(&session.agent_session_id, None, 10).await.unwrap();
        assert_eq!(messages.0, vec![source.clone()]);
        assert!(!messages.1);
        assert_eq!(messages.2, 1);
    }
}
