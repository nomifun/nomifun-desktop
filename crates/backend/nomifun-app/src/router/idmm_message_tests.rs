use super::*;
use nomifun_agent_contracts::{IdmmDecisionModel, IdmmDecisionSource, IdmmQuestionRef};

fn decision() -> IdmmDecisionExplanation {
    IdmmDecisionExplanation {
        intervention_id: Uuid::now_v7().to_string(),
        source: IdmmDecisionSource::BypassModel,
        reason_code: "bypass_model_decision".into(),
        rationale: "用户已明确希望采用简单难度".into(),
        model: Some(IdmmDecisionModel { provider_id: Uuid::now_v7().to_string(), model: "step-3.7-flash".into() }),
        question: Some(IdmmQuestionRef { message_id: Uuid::now_v7().to_string(), sequence: 8, fingerprint: "a".repeat(64) }),
    }
}

fn projection(message_id: &str, intent: &str, state: &str, extra: Value) -> MessageProjection {
    let mut document = json!({ "correlation_id":message_id,"state":state,"content":"简单", "display_at_ms":1234 });
    for (key,value) in extra.as_object().unwrap() { document[key] = value.clone(); }
    MessageProjection {
        session_id: AgentSessionId::from(Uuid::now_v7().to_string()),
        projection_id: format!("{}:{message_id}",if intent == "idmm_notice" { "idmm" } else { "message" }),
        first_seq:9,last_seq:9,presentation_intent:intent.into(),message_type:None,message_status:None,
        projection:document,semantic_digest:"b".repeat(64),
    }
}

#[test]
fn idmm_public_input_cannot_forge_server_authored_sources() {
    for input in [json!({"content":"hello","origin":"idmm"}),
        json!({"content":"hello","idmm_decision":decision()}),
        json!({"content":"hello","idmm_notice":null}),
        json!({"content":"hello","input_source":{"kind":"idmm"}})] {
        assert!(bounded_turn_input(input).is_err());
    }
    let input=bounded_turn_input(json!({"content":"human typed this"})).unwrap();
    assert_eq!(input.content,"human typed this");
    assert_eq!(input.origin,None);
}

#[test]
fn idmm_history_keeps_frozen_metadata_separate_from_answer_body() {
    let id=Uuid::now_v7().to_string();
    let explanation=decision();
    let row=projection(&id,"message","accepted",json!({"idmm_decision":explanation}));
    let response=canonical_message_response(&row.session_id.clone(),1,row).unwrap().unwrap();
    assert_eq!(response.content["content"],"简单");
    assert_eq!(response.content["idmm_decision"],serde_json::to_value(&explanation).unwrap());
    assert_eq!(response.position,Some(MessagePosition::Right));
    assert_eq!(response.status,Some(MessageStatus::Finish));
    assert_eq!(response.r#type,MessageType::Text);
    assert!(!response.content["idmm_decision"].as_object().unwrap().contains_key("confidence"));
}

#[test]
fn idmm_history_does_not_infer_metadata_from_origin_or_current_model() {
    let id=Uuid::now_v7().to_string();
    let row=projection(&id,"message","accepted",json!({"origin":"idmm","model":"unverified-model"}));
    let response=canonical_message_response(&row.session_id.clone(),1,row).unwrap().unwrap();
    assert!(response.content["idmm_decision"].is_null());
}

#[test]
fn idmm_notice_history_is_a_note_not_an_answer_or_turn_terminal() {
    let notice=IdmmDecisionNotice { decision:decision(),status:nomifun_agent_contracts::IdmmDecisionNoticeStatus::WaitingForHuman,created_at:1234 };
    let id=notice.decision.intervention_id.clone();
    let row=projection(&id,"idmm_notice","waiting_for_human",json!({"reference":notice}));
    let response=canonical_message_response(&row.session_id.clone(),1,row).unwrap().unwrap();
    assert_eq!(response.content["idmm_notice"],serde_json::to_value(&notice).unwrap());
    assert_eq!(response.r#type,MessageType::Tips);
    assert_eq!(response.position,Some(MessagePosition::Center));
    assert_eq!(response.status,Some(MessageStatus::Finish));
    assert!(response.content.get("turn_summary").is_none());
    assert!(response.content.get("turn_id").is_none());
}

#[test]
fn idmm_invalid_projection_metadata_fails_closed() {
    let id=Uuid::now_v7().to_string();
    let mut explanation=decision();
    explanation.source=IdmmDecisionSource::Rule;
    let row=projection(&id,"message","accepted",json!({"idmm_decision":explanation}));
    assert!(canonical_message_response(&row.session_id.clone(),1,row).is_err());
    let row=projection(&id,"idmm_notice","waiting_for_human",json!({"reference":{"status":"waiting_for_human","created_at":1234}}));
    assert!(canonical_message_response(&row.session_id.clone(),1,row).is_err());
}

fn accepted_input_event() -> nomifun_agent_contracts::SessionEventRecord {
    let message_id = Uuid::now_v7().to_string();
    nomifun_agent_contracts::SessionEventRecord {
        agent_session_id: Uuid::now_v7().to_string().into(), seq: 9,
        event_id: message_id.clone().into(), producer_id: "session_api".into(),
        idempotency_key: "accepted-input".into(),
        kind: nomifun_agent_contracts::SessionEventKind("message/user-accepted".into()),
        kind_version: 1, correlation_id: message_id.into(), causation_event_id: None,
        // The display adapter consumes the Store's resolved payload rather
        // than assuming every accepted event has inline JSON.
        payload: nomifun_agent_contracts::SessionEventPayloadRef::Stored("accepted-payload".into()),
    }
}

#[test]
fn canonical_accepted_input_preserves_every_surface_and_the_exact_display_body() {
    let event = accepted_input_event();
    let message_uuid = Uuid::parse_str(event.event_id.as_ref()).unwrap();
    let display_at_ms = message_uuid.as_bytes()[..6].iter()
        .fold(0_i64, |time, byte| (time << 8) | i64::from(*byte));
    let body = "用户消息\n\n[[NOMI_FILES]]\nD:/work/附件.png";
    for (origin, hidden, platform) in [
        (None, false, None),
        (Some("companion"), false, None),
        (Some("channel"), false, Some("telegram")),
        (Some("cron"), true, None),
    ] {
        let input = json!({"content":body,"hidden":hidden,"origin":origin,"channel_platform":platform});
        let wire = NomiCoreSessionOwner::canonical_accepted_input_wire_event(
            &event.agent_session_id, 1_000, &event, &input,
        ).unwrap();
        let wire = serde_json::to_value(wire).unwrap();
        assert_eq!(wire["name"], "message.userCreated");
        assert_eq!(wire["data"]["conversation_id"], event.agent_session_id.as_ref());
        assert_eq!(wire["data"]["msg_id"], event.event_id.as_ref());
        assert_eq!(wire["data"]["content"], body);
        assert_eq!(wire["data"]["hidden"], hidden);
        assert_eq!(wire["data"]["origin"], json!(origin));
        assert_eq!(wire["data"]["channel_platform"], json!(platform));
        assert_eq!(wire["data"]["created_at"], 1_009);
        assert_eq!(wire["data"]["display_at_ms"], display_at_ms);
        assert!(wire["data"]["idmm_decision"].is_null());
    }
}

#[test]
fn canonical_accepted_input_keeps_validated_idmm_provenance() {
    let event = accepted_input_event();
    let explanation = decision();
    let input = json!({"content":"简单","hidden":false,"origin":"idmm","idmm_decision":explanation});
    let wire = NomiCoreSessionOwner::canonical_accepted_input_wire_event(
        &event.agent_session_id, 1_000, &event, &input,
    ).unwrap();
    let wire = serde_json::to_value(wire).unwrap();
    assert_eq!(wire["data"]["content"], "简单");
    assert_eq!(wire["data"]["origin"], "idmm");
    assert_eq!(wire["data"]["idmm_decision"], serde_json::to_value(&explanation).unwrap());

    let mut invalid = explanation;
    invalid.source = IdmmDecisionSource::Rule;
    let invalid = json!({"content":"简单","idmm_decision":invalid});
    assert!(NomiCoreSessionOwner::canonical_accepted_input_wire_event(
        &event.agent_session_id, 1_000, &event, &invalid,
    ).is_err());
    assert!(NomiCoreSessionOwner::canonical_accepted_input_wire_event(
        &AgentSessionId::from(Uuid::now_v7().to_string()), 1_000, &event, &input,
    ).is_err());
    let mut invalid_event = event.clone();
    let invalid_id = Uuid::new_v4().to_string();
    invalid_event.event_id = invalid_id.clone().into();
    invalid_event.correlation_id = invalid_id.into();
    assert!(NomiCoreSessionOwner::canonical_accepted_input_wire_event(
        &invalid_event.agent_session_id, 1_000, &invalid_event, &input,
    ).is_err());
}
