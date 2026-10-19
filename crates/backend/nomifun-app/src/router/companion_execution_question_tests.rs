//! The connection actor acknowledges presentation acceptance only. This local
//! fixture does not synthesize audio or prove that a person heard the question.
use super::*;
#[path = "companion_direct_robot_question_tests.rs"]
mod companion_direct_robot_question_tests;

const QUESTION_INPUT:&str="QUESTION_BACKGROUND_REQUEST: Run the isolated report and ask me for its missing name.";
const QUESTION_GOAL:&str="QUESTION_BACKGROUND_WORK: Prepare the report; request the name before completing it.";
const QUESTION_TEXT:&str="What name should the background report use?";
const NATURAL_ANSWER:&str="小明";
const QUESTION_RESULT:&str="QUESTION-CANONICAL-51973: The report uses 小明.";
const BEFORE_PRESENTATION:&str="ANSWER_BEFORE_PRESENTATION";
const STALE_ANSWER:&str="STALE_EXECUTION_ANSWER";
const DIRECT_QUERY:&str="DIRECT_ROBOT_QUESTION: Ask me which name to use before answering.";
const DIRECT_QUESTION:&str="Which name should I use for this conversation?";
const DIRECT_ANSWER:&str="林晓";
const DIRECT_RESULT:&str="DIRECT-ROBOT-CANONICAL-31649: I will use 林晓.";

fn physical_request(body:&Value)->bool {
    body["messages"].as_array().unwrap().iter().filter(|message|message["role"]=="system")
        .any(|message|message["content"].to_string().contains("Physical Robot reply style:"))
}

pub(super) fn model_step(body:&Value,state:&ModelState,user:&str)->Option<(Value,&'static str)> {
    if !state.question_mode {return None;}
    if user.trim()==DIRECT_QUERY {
        return Some(tool_call("request_user_input","direct-ask-name",json!({
            "question":DIRECT_QUESTION,"remaining_work":"Answer the original request using the person's name."})));
    }
    if user.trim()==DIRECT_ANSWER {
        let history=body["messages"].to_string();
        assert!(history.contains(DIRECT_QUESTION),"the original typed question reaches the next real model request");
        assert!(history.contains("Explicit reference carried by the CURRENT accepted input"),
            "the accepted natural answer references its exact canonical question");
        return Some((json!({"role":"assistant","content":DIRECT_RESULT}),"stop"));
    }
    if user.contains(QUESTION_GOAL) {
        return Some(tool_call("request_user_input","background-ask-name",json!({
            "question":QUESTION_TEXT,"remaining_work":"Complete the isolated report using the exact human answer."})));
    }
    if user.contains(QUESTION_INPUT) {
        return Some(if tool_result(body,"question-background-fork").is_some() {
            (json!({"role":"assistant","content":"The background work is accepted."}),"stop")
        } else {selected_tool(body,"fork_agent","question-background-fork","agent/fork",json!({"goal":QUESTION_GOAL}))});
    }
    if !physical_request(body)&&user.trim()==NATURAL_ANSWER {
        return Some((json!({"role":"assistant","content":QUESTION_RESULT}),"stop"));
    }
    let id=if user.trim()=="嗯" {"question-backchannel-answer"}
        else if user.trim()==NATURAL_ANSWER {"question-natural-answer"}
        else if user.contains(BEFORE_PRESENTATION) {"question-unpresented-answer"}
        else if user.contains(STALE_ANSWER) {"question-stale-answer"}
        else {return None;};
    Some(if tool_result(body,id).is_some() {
        (json!({"role":"assistant","content":"The answer operation has its canonical receipt."}),"stop")
    } else {selected_tool(body,"answer_execution_question",id,"session.management/execution_answer_decision",json!({}))})
}

async fn wait_for_waiting(flow:&PhysicalEntryFixture)->nomifun_api_types::AgentExecutionDetail {
    tokio::time::timeout(Duration::from_secs(25),async {loop {
        let executions=flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
        if let Some(execution)=executions.into_iter().find(|execution|execution.lead_conversation_id.as_deref()==Some(flow.robot_id.as_ref())) {
            let detail=flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap();
            if detail.execution.status==nomifun_common::AgentExecutionStatus::WaitingInput {break detail;}
            assert!(!detail.execution.status.is_terminal(),"question task closed before its decision: {detail:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the isolated child must publish a canonical pending decision")
}

#[tokio::test]
async fn background_question_accepts_exact_socket_presentation_and_natural_answer_once_on_the_same_attempt() {
    let server=CompanionFlowModelServer::with_question_mode().await;
    let flow=PhysicalEntryFixture::new(&server,"unified-background-question").await;
    let robots=flow.fixture.services.robot.as_ref().unwrap();
    let mut playback=robots.registry.open_playback(&flow.device_id,&flow.connection).await.unwrap();
    let (presentation_tx,mut presentation_rx)=tokio::sync::mpsc::channel(4);
    let (queue_ack_tx,mut queue_ack_rx)=tokio::sync::mpsc::channel(4);
    let writer_release=Arc::new(tokio::sync::Notify::new());let actor_writer_release=writer_release.clone();
    let actor_backend=robots.conversation_backend_for_test();
    let actor_request=flow.request("canonical-question-playback","");
    let actor=tokio::spawn(async move {while let Some(command)=playback.recv().await {
        let authority=command.source.clone().zip(command.question_presentation_id.clone());
        let playback_context=command.context.clone();
        if authority.is_some() {
            presentation_tx.send((command.text.clone(),command.source.clone(),command.question_presentation_id.clone())).await.unwrap();
        }
        // Queue admission is prompt, like the production actor. Only actual
        // writer presentation is held while an unrelated real Turn runs.
        let _=command.accepted.send(Ok(()));
        if let Some((source,presentation_id))=authority {
            queue_ack_tx.send(()).await.unwrap();
            actor_writer_release.notified().await;
            let context=playback_context.expect("a background question carries its original playback scope");
            let mut request=actor_request.clone();
            assert_eq!(context.session_id,request.conversation_id);
            request.captured_context=Some(context.input_context);
            // This callback represents the exact displayed sentence entering
            // the original connection's writer. Queue ACK alone is insufficient.
            actor_backend.question_presented(&request,&source,&presentation_id).await.unwrap();
        }
    }});
    let utterance=flow.request("question-background-request",QUESTION_INPUT);
    let original=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,"question-background-input").await;
    let waiting=wait_for_waiting(&flow).await;
    assert_eq!(waiting.steps.len(),1);assert_eq!(waiting.attempts.len(),1);
    let attempt=waiting.attempts[0].clone();let execution_id=waiting.execution.execution_id.clone();
    let child=AgentSessionId::from(attempt.conversation_id.clone().unwrap());
    let store=flow.fixture.owner.canonical().store();
    let (text,speech,presentation_id)=tokio::time::timeout(Duration::from_secs(15),presentation_rx.recv()).await.unwrap().unwrap();
    assert_eq!(text,QUESTION_TEXT);let speech=speech.unwrap();let presentation_id=presentation_id.unwrap();
    let notice_facts=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    let notice=notice_facts.event_payloads.get(&speech.message_id).unwrap();
    let presentation:nomifun_voice_contracts::VoiceApprovalPresentation=serde_json::from_value(notice["decision"].clone()).unwrap();
    assert_eq!(presentation.target.presentation_id,presentation_id);
    assert_eq!(presentation.target.work_target.agent_session_id,child);
    assert_eq!(notice["origin"],"agent_execution_input_required");assert_eq!(notice["requires_desktop"],false);
    let question_operation=presentation.target.work_target.turn_operation_id.clone();
    let question_receipt=store.read_turn_receipt(&child,&question_operation).await.unwrap();
    let question_terminal=question_receipt.terminal_event.as_ref().unwrap().event_id.clone();
    assert_eq!(question_receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);

    // The model attempts the real answer Action before the actor accepts the
    // presentation. Its already accepted root cannot borrow the later proof.
    let mut early=flow.request("question-early",BEFORE_PRESENTATION);
    early.presented_question_id=Some(presentation_id.clone());
    early.captured_context=Some(flow.fixture.owner.capture_robot_input_context(&flow.owner,&early).await.unwrap());
    let early_operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&early,"question-early-input").await;
    let (_,early_input)=store.turn_accepted_input(&flow.robot_id,&early_operation).await.unwrap();
    assert!(early_input.0["input_source"]["presented_question_id"].is_null());
    assert_eq!(flow.fixture.voice_execution.get(&flow.owner,&execution_id).await.unwrap().execution.status,nomifun_common::AgentExecutionStatus::WaitingInput);
    tokio::time::timeout(Duration::from_secs(15),queue_ack_rx.recv()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(15),async {loop {
        let events=flow.fixture.voice_execution.events(&flow.owner,&execution_id,None,Some(128)).await.unwrap();
        if events.iter().any(|event|event.payload["change"]=="lead_input_notice_reconciled"
            &&event.payload["request_event_sequence"]==presentation.target.request_event_sequence) {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the queue acknowledgement must settle independently from writer presentation");
    let queued=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    assert!(!queued.events.iter().any(|event|event.kind.0=="robot/question-playback-accepted"),
        "queue acceptance must never author a presentation proof");
    assert!(flow.fixture.owner.capture_execution_question(&flow.owner,&flow.robot_id,&flow.connection,
        "queue-ack-recording",Some(&presentation_id)).await.unwrap().is_none(),
        "queue ACK does not grant authority to answer a question");
    writer_release.notify_one();
    let accepted_event=tokio::time::timeout(Duration::from_secs(15),async {loop {
        let facts=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
        if let Some(event)=facts.events.iter().find(|event|event.kind.0=="robot/question-playback-accepted"
            &&facts.event_payloads.get(event.event_id.as_ref()).is_some_and(|payload|payload["notice_message_id"]==speech.message_id
                &&payload["presentation_id"]==presentation_id&&payload["connection_id"]==flow.connection)) {break event.clone();}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original socket actor must acknowledge the exact canonical presentation");
    assert!(accepted_event.seq>speech.through_seq);
    assert!(flow.fixture.owner.capture_execution_question(&flow.owner,&flow.robot_id,"foreign-socket","foreign-recording",Some(&presentation_id)).await.unwrap().is_none());
    assert!(flow.fixture.owner.capture_execution_question(&flow.owner,&flow.robot_id,&flow.connection,"wrong-question",Some("execution:unrelated:decision:1")).await.unwrap().is_none());

    let mut backchannel=flow.request("question-backchannel","嗯");
    backchannel.presented_question_id=Some(presentation_id.clone());
    backchannel.captured_context=Some(flow.fixture.owner.capture_robot_input_context(&flow.owner,&backchannel).await.unwrap());
    send_and_complete_robot_turn(&flow.fixture,&flow.owner,&backchannel,"question-backchannel-input").await;
    let events=flow.fixture.voice_execution.events(&flow.owner,&execution_id,None,Some(128)).await.unwrap();
    assert_eq!(events.iter().filter(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered).count(),0);
    let requests=server.state.requests.lock().unwrap().clone();
    let rejection=requests.iter().find_map(|body|tool_result(body,"question-backchannel-answer")).unwrap();
    assert!(rejection["content"].to_string().contains("backchannels do not answer a task question"));

    let mut answer=flow.request("question-natural-answer",NATURAL_ANSWER);
    answer.presented_question_id=Some(presentation_id.clone());
    answer.captured_context=Some(flow.fixture.owner.capture_robot_input_context(&flow.owner,&answer).await.unwrap());
    let answer_operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&answer,"question-natural-answer-input").await;
    let (answer_root,answer_input)=store.turn_accepted_input(&flow.robot_id,&answer_operation).await.unwrap();
    assert_eq!(answer_input.0["content"],NATURAL_ANSWER);
    assert_eq!(answer_input.0["input_source"]["presented_question_id"],presentation_id);
    assert!(accepted_event.seq<answer_root.seq);
    let events=flow.fixture.voice_execution.events(&flow.owner,&execution_id,None,Some(128)).await.unwrap();
    let answers=events.iter().filter(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered).collect::<Vec<_>>();
    assert_eq!(answers.len(),1);assert_eq!(answers[0].step_id.as_deref(),Some(attempt.step_id.as_str()));
    assert_eq!(answers[0].attempt_id.as_deref(),Some(attempt.attempt_id.as_str()));
    assert_eq!(answers[0].payload["request_event_sequence"],presentation.target.request_event_sequence);
    let (completed,report)=tokio::time::timeout(Duration::from_secs(25),async {loop {
        let detail=flow.fixture.voice_execution.get(&flow.owner,&execution_id).await.unwrap();
        if detail.execution.status==nomifun_common::AgentExecutionStatus::Completed {
            let facts=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
            if let Some(report)=facts.event_payloads.values().find(|payload|payload["origin"]=="agent_execution_report"
                &&payload["delegation_report"]["execution_id"]==execution_id) {break (detail,report.clone());}
        }
        assert!(!matches!(detail.execution.status,nomifun_common::AgentExecutionStatus::Failed|nomifun_common::AgentExecutionStatus::Cancelled),"answered task failed: {detail:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the same Attempt must consume the answer and directly project its report");
    assert_eq!(completed.steps.len(),1);assert_eq!(completed.attempts.len(),1);
    assert_eq!(completed.attempts[0].attempt_id,attempt.attempt_id);assert_eq!(completed.attempts[0].step_id,attempt.step_id);
    assert!(report["content"].as_str().unwrap().contains(QUESTION_RESULT));
    let report:nomifun_agent_execution::AgentExecutionReport=serde_json::from_value(report["delegation_report"].clone()).unwrap();
    assert_eq!(report.source.turn_operation_id,original.as_ref());assert_eq!(report.attempts.len(),1);
    let continued=OperationId::from(report.attempts[0].turn_operation_id.clone());assert_ne!(continued,question_operation);
    let (_,continuation_input)=store.turn_accepted_input(&child,&continued).await.unwrap();
    assert_eq!(continuation_input.0["content"],NATURAL_ANSWER);assert_eq!(continuation_input.0["origin"],"agent_execution_decision");
    assert_eq!(store.read_turn_receipt(&child,&question_operation).await.unwrap().terminal_event.unwrap().event_id,question_terminal);
    let turns_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(turns_before,4,"original, unpresented, backchannel and real answer inputs are the only source model Turns");
    let calls=server.state.requests.lock().unwrap().len();
    let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&answer,"question-natural-answer-input",robot_message(NATURAL_ANSWER)).await.unwrap();
    assert!(replay.replayed);assert_eq!(replay.message_id,answer_root.event_id.as_ref());
    assert_eq!(server.state.requests.lock().unwrap().len(),calls);
    let after=flow.fixture.voice_execution.events(&flow.owner,&execution_id,None,Some(128)).await.unwrap();
    assert_eq!(after.iter().filter(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered).count(),1);
    assert!(flow.fixture.owner.capture_execution_question(&flow.owner,&flow.robot_id,&flow.connection,"stale-recording",Some(&presentation_id)).await.unwrap().is_none(),
        "an old presentation never upgrades a later recording to a different task");
    let requests=server.state.requests.lock().unwrap().clone();
    let applied=requests.iter().find_map(|body|tool_result(body,"question-natural-answer")).unwrap();
    let applied:Value=serde_json::from_str(applied["content"].as_str().unwrap()).unwrap();assert_eq!(applied["status"],"applied");
    let calls=requests.iter().flat_map(|body|body["messages"].as_array().unwrap()).flat_map(|message|
        message["tool_calls"].as_array().into_iter().flatten()).filter(|call|call["id"]=="question-natural-answer").collect::<Vec<_>>();
    assert!(!calls.is_empty());assert!(calls.iter().all(|call|call["function"]["arguments"]=="{}"),"model supplies neither replacement answer text nor task scope");
    record_fixture_cost("physical_background_question",&requests,&flow);

    // A recording captured before clear is rejected without admission. A
    // newly captured input remains usable and does not inherit a question.
    let report_operation=format!("exec-lead-report:{execution_id}:event:{}",report.execution_event_sequence);
    tokio::time::timeout(Duration::from_secs(15),async {loop {
        let events=flow.fixture.voice_execution.events(&flow.owner,&execution_id,None,Some(128)).await.unwrap();
        if events.iter().any(|event|event.payload["change"]=="lead_report_delivered"
            &&event.payload["operation_id"]==report_operation) {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("context clear waits for the exact report outbox delivery settlement");
    let mut delayed=answer.clone();delayed.request_id="delayed-before-clear".into();delayed.text=STALE_ANSWER.into();
    api(&flow.fixture._router,&format!("/api/agent-sessions/{}/clear-context",flow.robot_id.as_ref()),json!({"idempotency_key":"question-clear"})).await;
    let before_reject=store.head(&flow.robot_id).await.unwrap().last_seq;
    let rejected=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&delayed,"delayed-before-clear",robot_message(STALE_ANSWER)).await;
    assert!(matches!(rejected,Err(nomifun_common::AppError::Conflict(_))));
    assert_eq!(store.head(&flow.robot_id).await.unwrap().last_seq,before_reject,"late ASR never receives a fresh scope or retry Turn");
    let mut fresh=flow.request("fresh-after-clear",FOLLOWUP_INPUT);
    fresh.captured_context=Some(flow.fixture.owner.capture_robot_input_context(&flow.owner,&fresh).await.unwrap());
    assert!(fresh.captured_context.as_ref().unwrap().presented_question_id.is_none());
    let fresh_operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&fresh,"fresh-after-clear").await;
    assert!(store.turn_accepted_input(&flow.robot_id,&fresh_operation).await.unwrap().1.0.get("reply_to").is_none());
    flow.finish().await;actor.abort();let _=actor.await;server.close().await;
}
