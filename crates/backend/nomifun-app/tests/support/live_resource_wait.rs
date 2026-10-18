//! Real model and real owned process, with a fixture-controlled external authorization.
//! No Feishu account, browser credential, production data or human approval is fabricated.
use super::*;
use nomifun_agent_contracts::{AgentSessionId, AgentTurnOutcome, OperationId};
use nomifun_agent_session::{AgentSessionStore, TurnReceiptStatus};

fn failure(phase: &'static str, code: &'static str) -> SmokeFailure {
    SmokeFailure::new(phase, code, 422)
}

async fn facts(store: &AgentSessionStore, session: &str, operation: &str)
    -> Result<nomifun_agent_session::ChatCausalityFacts, SmokeFailure> {
    store.turn_output_facts(&AgentSessionId::from(session), &OperationId::from(operation)).await
        .map_err(|_| failure("resource_wait.facts", "RESOURCE_WAIT_FACTS_INVALID"))
}

fn count_event(facts: &nomifun_agent_session::ChatCausalityFacts, kind: &str, operation: &str) -> usize {
    facts.events.iter().filter(|event| event.kind.0 == kind && event.correlation_id.as_ref() == operation).count()
}

fn model_steps(facts: &nomifun_agent_session::ChatCausalityFacts, operation: &str) -> usize {
    facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded"
        && event.correlation_id.as_ref() == operation
        && facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload| payload.pointer("/event/event"))
            .and_then(Value::as_str) == Some("model_step_started")).count()
}

fn assert_initial_process_evidence(facts: &nomifun_agent_session::ChatCausalityFacts, operation: &str) -> Result<(), SmokeFailure> {
    let start = facts.events.iter().find(|event| event.kind.0 == "turn/started"
        && event.correlation_id.as_ref() == operation)
        .ok_or_else(|| failure("resource_wait.evidence", "RESOURCE_WAIT_START_MISSING"))?;
    let launches = facts.events.iter().filter(|event| event.kind.0 == "tool/call-started"
        && event.causation_event_id.as_ref() == Some(&start.event_id)
        && facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload| payload.get("action_id"))
            .and_then(Value::as_str) == Some("workspace.process/start")).count();
    if launches == 0 || count_event(facts, "turn/failed", operation) != 0 {
        return Err(failure("resource_wait.evidence", "RESOURCE_WAIT_FAILED_OR_REPLAYED"));
    }
    if facts.events.iter().any(|event|event.kind.0=="tool/call-started"
        && event.causation_event_id.as_ref()==Some(&start.event_id)
        && matches!(facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("action_id")).and_then(Value::as_str),
            Some("workspace.files/write"|"workspace.files/patch"|"workspace.files/delete"|"workspace.files/rename"|"workspace.process/input"))) {
        return Err(failure("resource_wait.evidence","RESOURCE_WAIT_AUTHORIZATION_MUTATED_BY_AGENT"));
    }
    Ok(())
}

async fn assert_helper_launched_once(workspace:&Path)->Result<(),SmokeFailure> {
    // Other legitimate inspection commands are allowed. Count the actual
    // authorization helper's own launches, rather than all process tools.
    let launches=tokio::fs::read_to_string(workspace.join("authorization-launches.txt")).await
        .map_err(|_|failure("resource_wait.evidence","RESOURCE_WAIT_LAUNCH_LOG_MISSING"))?;
    if launches!="launch\n" {return Err(failure("resource_wait.evidence","RESOURCE_WAIT_HELPER_REPEATED"));}
    Ok(())
}

/// The computational Turn must close before any browser/user action occurs.
async fn await_question(store: &AgentSessionStore, session: &str, operation: &str)
    -> Result<String, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let receipt = store.read_turn_receipt(&session.into(), &operation.into()).await
            .map_err(|_| failure("resource_wait.question", "RESOURCE_WAIT_RECEIPT_INVALID"))?;
        let outcome = store.turn_outcome(&session.into(), &operation.into()).await
            .map_err(|_| failure("resource_wait.question", "RESOURCE_WAIT_OUTCOME_INVALID"))?;
        match outcome {
            Some(AgentTurnOutcome::InputRequired { request_id }) if receipt.status == TurnReceiptStatus::Completed => return Ok(request_id),
            Some(AgentTurnOutcome::Failed { .. } | AgentTurnOutcome::Cancelled { .. } | AgentTurnOutcome::Paused { .. }
                | AgentTurnOutcome::Answered { .. }) => return Err(failure("resource_wait.question", "RESOURCE_WAIT_QUESTION_NOT_HANDED_BACK")),
            _ => {},
        }
        if tokio::time::Instant::now() >= deadline { return Err(failure("resource_wait.question", "RESOURCE_WAIT_DECLARATION_DEADLINE")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn submit_message(router: &Router, pool: &nomifun_db::SqlitePool, session: &str, prompt: String) -> Result<String,SmokeFailure> {
    let key = uuid::Uuid::now_v7().to_string();
    let body = json!({"idempotency_key":key,"input":{"content":prompt}});
    let accepted = successful_json(router,"resource_wait.intake",Method::POST,format!("/api/agent-sessions/{session}/messages"),
        Some(body.clone()),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let accepted = envelope_data("resource_wait.intake",accepted)?;
    let message = required_string("resource_wait.intake",&accepted,"/message_id","RESOURCE_WAIT_ACCEPTED_MESSAGE_MISSING")?;
    if accepted["status"] != "accepted" || accepted["agent_session_id"] != session {
        return Err(failure("resource_wait.intake","RESOURCE_WAIT_ACCEPTANCE_INVALID"));
    }
    let replay = successful_json(router,"resource_wait.intake_replay",Method::POST,format!("/api/agent-sessions/{session}/messages"),
        Some(body),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let replay = envelope_data("resource_wait.intake_replay",replay)?;
    if replay["message_id"] != message || replay["replayed"] != true {
        return Err(failure("resource_wait.intake_replay","RESOURCE_WAIT_ACCEPTANCE_REPEATED"));
    }
    let deadline = tokio::time::Instant::now()+Duration::from_secs(60);
    loop {
        let operation: Option<String> = nomifun_db::sqlx::query_scalar("SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=?")
            .bind(session).bind(&message).fetch_optional(pool).await
            .map_err(|_|failure("resource_wait.intake","RESOURCE_WAIT_INPUT_CLAIM_INVALID"))?;
        if let Some(operation)=operation { return Ok(operation); }
        if tokio::time::Instant::now()>=deadline { return Err(failure("resource_wait.intake","RESOURCE_WAIT_INPUT_NOT_DISPATCHED")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn await_settled(store: &AgentSessionStore, session: &str, operation: &str, cancelled: bool)
    -> Result<&'static str, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let receipt = store.read_turn_receipt(&session.into(), &operation.into()).await
            .map_err(|_| failure("resource_wait.settle", "RESOURCE_WAIT_RECEIPT_INVALID"))?;
        if receipt.status == TurnReceiptStatus::Failed {
            return Err(failure("resource_wait.settle", "RESOURCE_WAIT_TASK_MARKED_FAILED"));
        }
        if cancelled && receipt.status == TurnReceiptStatus::Cancelled { return Ok("cancelled"); }
        if !cancelled && receipt.status == TurnReceiptStatus::Completed { return Ok("completed"); }
        if !cancelled && matches!(store.turn_outcome(&session.into(), &operation.into()).await
            .map_err(|_| failure("resource_wait.settle", "RESOURCE_WAIT_OUTCOME_INVALID"))?,
            Some(AgentTurnOutcome::Paused { .. })) {
            return Err(failure("resource_wait.settle", "RESOURCE_WAIT_NORMAL_DEPENDENCY_PAUSED"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failure("resource_wait.settle", "RESOURCE_WAIT_SETTLEMENT_DEADLINE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn owner_confirmed_authorization(facts: &nomifun_agent_session::ChatCausalityFacts) -> bool {
    facts.event_payloads.values().any(|payload| {
        let poll = payload.pointer("/event/observation/poll");
        if poll.is_some_and(|poll| poll["state"] == "exited" && poll["exit_code"] == 0
            && poll.pointer("/cleanup/reaped") == Some(&json!(true))
            && poll.pointer("/output/text").and_then(Value::as_str).is_some_and(|text| text.contains("AUTHORIZATION_CONFIRMED"))) {
            return true;
        }
        let Some(result) = payload.get("output").filter(|result| result.is_object()) else { return false; };
        let Ok(result) = serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(result.clone()) else { return false; };
        let Ok(poll) = serde_json::from_str::<Value>(&result.output_text()) else { return false; };
        !result.is_error && poll["state"] == "exited" && poll["exit_code"] == 0
            && poll.pointer("/cleanup/reaped") == Some(&json!(true))
            && poll.pointer("/output/text").and_then(Value::as_str).is_some_and(|text| text.contains("AUTHORIZATION_CONFIRMED"))
    })
}

async fn assert_public_question(router:&Router,session:&str,request:&str,url:&str)->Result<(),SmokeFailure> {
    let (messages,_)=session_messages_after(router,"resource_wait.public_messages",session,0).await?;
    if !messages.iter().any(|message|message.pointer("/projection/user_input_request/request_id").and_then(Value::as_str)==Some(request)
        && message.pointer("/projection/user_input_request/question").and_then(Value::as_str).is_some_and(|question|question.contains(url))
        && message.pointer("/projection/turn_outcome/kind").and_then(Value::as_str)==Some("input_required")) {
        return Err(failure("resource_wait.public_messages","RESOURCE_WAIT_QUESTION_NOT_PUBLIC_BEFORE_FINISH"));
    }
    let history=successful_json(router,"resource_wait.renderer_history",Method::GET,
        format!("/api/agent-sessions/{session}/message-history?page_size=100"),None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    if !envelope_data("resource_wait.renderer_history",history)?.get("items").and_then(Value::as_array).is_some_and(|items|
        items.iter().any(|message|message.pointer("/content/user_input_request/request_id").and_then(Value::as_str)==Some(request)
            && message.pointer("/content/user_input_request/question").and_then(Value::as_str).is_some_and(|question|question.contains(url)))) {
        return Err(failure("resource_wait.renderer_history","RESOURCE_WAIT_RENDERER_QUESTION_MISSING"));
    }
    Ok(())
}

fn followup_marker_ids<'a>(public:&'a nomifun_agent_session::CanonicalTurnPublicResult,marker:&str)
    ->Result<&'a [String],SmokeFailure> {
    let Some(AgentTurnOutcome::Answered {message_ids})=&public.turn_outcome else {
        return Err(failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_NOT_ANSWERED"));
    };
    if public.result_text.as_deref().map(str::trim)!=Some(marker) || message_ids.len()!=1 {
        return Err(failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_MARKER_MISMATCH"));
    }
    Ok(message_ids)
}

fn is_instruction_scope_preparation(step:u16,name:&str,capability:&str,action:&str,args:&Value,
    effect:Option<nomifun_agent_contracts::EffectClass>)->bool {
    let Some(args)=args.as_object() else {return false;};
    step==0 && name=="read_file" && capability=="workspace.files" && action=="workspace.files/read"
        && effect==Some(nomifun_agent_contracts::EffectClass::ReadLocal)
        && args.get("format").and_then(Value::as_str)==Some("instruction_scope")
        && args.get("path").and_then(Value::as_str).is_some_and(|path|!path.is_empty()&&path.len()<=4096&&!path.contains('\0'))
        && args.keys().all(|key|matches!(key.as_str(),"path"|"format"|"recursive"|"execution_identity"))
        && args.get("recursive").is_none_or(Value::is_boolean)
        && args.get("execution_identity").is_none_or(|identity|identity.as_str()==Some("current_user"))
}

fn is_instruction_page_preparation(step:u16,name:&str,capability:&str,action:&str,args:&Value,
    effect:Option<nomifun_agent_contracts::EffectClass>)->bool {
    let Some(args)=args.as_object() else {return false;};
    let Some(path)=args.get("path").and_then(Value::as_str) else {return false;};
    let Some(offset)=args.get("offset").and_then(Value::as_u64) else {return false;};
    let leaf=std::path::Path::new(path).file_name().and_then(|name|name.to_str());
    step==0 && name=="read_file" && capability=="workspace.files" && action=="workspace.files/read"
        && effect==Some(nomifun_agent_contracts::EffectClass::ReadLocal)
        && matches!(leaf,Some("AGENTS.md"|"AGENTS.override.md")) && path.len()<=4096 && !path.contains('\0')
        && args.keys().all(|key|matches!(key.as_str(),"path"|"offset"|"limit"|"missing_ok"|"expected_sha256"|"execution_identity"))
        && args.get("limit").and_then(Value::as_u64)==Some(16384) && args.get("missing_ok")==Some(&Value::Bool(true))
        && offset<=16384 && if offset==0 {args.get("expected_sha256").is_none()} else {
            args.get("expected_sha256").and_then(Value::as_str).is_some_and(|digest|digest.len()==64
                && digest.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)))
        }
        && args.get("execution_identity").is_none_or(|identity|identity.as_str()==Some("current_user"))
}

fn static_tool_name(name:&str)->&'static str {
    match name {"read_file"=>"read_file","start_process"=>"start_process","write_file"=>"write_file",
        "apply_patch"=>"apply_patch","delete_path"=>"delete_path","rename_path"=>"rename_path",
        "write_process_stdin"=>"write_process_stdin","update_plan"=>"update_plan","report_completion"=>"report_completion",
        _=>"other_tool"}
}

async fn read_followup_snapshot(pool:&nomifun_db::SqlitePool,reference:&nomifun_agent_contracts::ResolvedSnapshotRef)
    ->Result<nomifun_agent_contracts::ResolvedSnapshotEnvelope,SmokeFailure> {
    let phase="resource_wait.followup_reply";
    let envelope:String=nomifun_db::sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=? AND snapshot_digest=?")
        .bind(reference.snapshot_id.as_ref()).bind(reference.snapshot_digest.as_ref()).fetch_one(pool).await
        .map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_MISSING"))?;
    let snapshot:nomifun_agent_contracts::ResolvedSnapshotEnvelope=serde_json::from_str(&envelope)
        .map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_INVALID"))?;
    snapshot.validate().map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_INVALID"))?;
    if &snapshot.snapshot_ref!=reference {return Err(failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_INVALID"));}
    Ok(snapshot)
}

async fn assert_followup_effect_safety(store:&AgentSessionStore,pool:&nomifun_db::SqlitePool,
    current:&nomifun_agent_session::ChatCausalityFacts,operation:&str)->Result<(),SmokeFailure> {
    use nomifun_agent_runtime::AgentEngineEvent;
    let phase="resource_wait.followup_reply";
    let source=current.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation)
        .ok_or_else(||failure(phase,"RESOURCE_WAIT_FOLLOWUP_START_MISSING"))?;
    let reference:nomifun_agent_contracts::ResolvedSnapshotRef=serde_json::from_value(current.event_payloads.get(source.event_id.as_ref())
        .and_then(|payload|payload.get("resolved_snapshot_ref")).cloned()
        .ok_or_else(||failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_MISSING"))?)
        .map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_INVALID"))?;
    // Read the original immutable contract, never today's catalog or a tool
    // name heuristic. This isolated database connection is read-only.
    let snapshot=read_followup_snapshot(pool,&reference).await?;
    let mut calls=std::collections::BTreeMap::new();let mut results=std::collections::BTreeMap::new();let mut started=Vec::new();
    for event in current.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==operation) {
        let value=current.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event"))
            .ok_or_else(||failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_FACT_INVALID"))?;
        // The canonical progress stream also contains Host-only dispatch and
        // settlement families. Their Store validation is authoritative; only
        // these three Runtime tool facts belong to this audit's typed enum.
        if !matches!(value.get("event").and_then(Value::as_str),Some("tool_call_completed"|"tool_started"|"tool_completed")) {continue;}
        let typed:AgentEngineEvent=serde_json::from_value(value.clone()).map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_FACT_INVALID"))?;
        match typed {
            AgentEngineEvent::ToolCallCompleted {step,call}=>{if calls.insert(call.call_id.as_ref().to_owned(),(event.seq,step,call)).is_some() {
                return Err(failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_FACT_INVALID"));}},
            AgentEngineEvent::ToolStarted {step,call_id,capability_id,action_id}=>started.push((event.seq,step,call_id,capability_id,action_id)),
            AgentEngineEvent::ToolCompleted {step,result,..}=>{if results.insert(result.call_id.as_ref().to_owned(),(event.seq,step,result)).is_some() {
                return Err(failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_FACT_INVALID"));}},
            _=>{},
        }
    }
    let mut diagnostics=std::collections::BTreeMap::new();let mut invalid=false;
    for (start_seq,step,id,capability,action) in started {
        let(call_seq,call_step,call)=calls.get(id.as_ref()).ok_or_else(||failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_FACT_INVALID"))?;
        let(result_seq,result_step,result)=results.get(id.as_ref()).ok_or_else(||failure(phase,"RESOURCE_WAIT_FOLLOWUP_TOOL_UNSETTLED"))?;
        let descriptor=snapshot.content.enabled_capabilities.iter().find(|selected|selected.capability.id==capability
            && selected.action_allowlist.contains(&action)).and_then(|selected|selected.actions.iter().find(|descriptor|descriptor.action_id==action));
        let effect=descriptor.map(|descriptor|descriptor.effect_class);
        let preparation=if is_instruction_scope_preparation(step,&call.name,capability.as_ref(),action.as_ref(),&call.arguments.0,effect) {
            Some("platform_instruction_scope_readonly")
        } else if is_instruction_page_preparation(step,&call.name,capability.as_ref(),action.as_ref(),&call.arguments.0,effect) {
            Some("platform_instruction_page_readonly")
        } else {None};
        let allowed=*call_step==step && *result_step==step && *call_seq<start_seq && start_seq<*result_seq && !result.is_error && preparation.is_some();
        let class=if allowed {preparation.expect("verified preparation")} else {"not_permitted_preparation"};
        let effect_class=match effect {Some(nomifun_agent_contracts::EffectClass::ReadLocal)=>"read_local",
            Some(nomifun_agent_contracts::EffectClass::Pure)=>"pure",None=>"missing_contract",_=>"other_effect"};
        *diagnostics.entry((static_tool_name(&call.name),class,effect_class)).or_insert(0usize)+=1;
        invalid|=!allowed;
    }
    // Only compiled-in names and classifications leave the fixture. No call
    // IDs, arguments, response text, file paths or provider secrets are logged.
    for ((tool,class,effect),count) in diagnostics {eprintln!("NOMIFUN_RESOURCE_WAIT_FOLLOWUP_TOOL tool={tool} class={class} effect={effect} count={count}");}
    if invalid {return Err(failure(phase,"RESOURCE_WAIT_FOLLOWUP_UNEXPECTED_EFFECT_OR_TOOL"));}
    if store.list_effects(&current.session.agent_session_id).await.map_err(|_|failure(phase,"RESOURCE_WAIT_FOLLOWUP_EFFECTS_INVALID"))?
        .iter().any(|effect|effect.turn_id.as_ref()==operation) {
        return Err(failure(phase,"RESOURCE_WAIT_FOLLOWUP_EFFECT_RECORDED"));
    }
    Ok(())
}

async fn await_followup_marker(router:&Router,store:&AgentSessionStore,pool:&nomifun_db::SqlitePool,session:&str,operation:&str,
    marker:&str)->Result<(),SmokeFailure> {
    let deadline=tokio::time::Instant::now()+TURN_RESULT_DEADLINE;
    loop {
        let receipt=store.read_turn_receipt(&session.into(),&operation.into()).await
            .map_err(|_|failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_RECEIPT_INVALID"))?;
        let current=facts(store,session,operation).await?;
        let public=nomifun_agent_session::canonical_turn_public_result(&current,&receipt)
            .map_err(|_|failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_OUTCOME_INVALID"))?;
        // The new operation is the authority. An earlier authorization timeout
        // remains truthful history and cannot decide this independent reply.
        if matches!(receipt.status,TurnReceiptStatus::Failed|TurnReceiptStatus::Cancelled)
            || matches!(public.turn_outcome.as_ref(),Some(AgentTurnOutcome::Failed {..}|AgentTurnOutcome::Cancelled {..}|AgentTurnOutcome::Paused {..})) {
            return Err(failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_TURN_FAILED"));
        }
        if receipt.status==TurnReceiptStatus::Completed {
            assert_followup_effect_safety(store,pool,&current,operation).await?;
            let message_ids=followup_marker_ids(&public,marker)?;
            let source=receipt.started_event.as_ref().ok_or_else(||failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_START_MISSING"))?;
            let(messages,_)=session_messages_after(router,"resource_wait.followup_reply",session,source.seq).await?;
            let answers=messages.iter().filter_map(assistant_text_projection).filter(|answer|
                answer.get("correlation_id").and_then(Value::as_str).is_some_and(|id|message_ids.iter().any(|expected|expected==id)))
                .collect::<Vec<_>>();
            if answers.len()!=1 || answers[0].get("content").and_then(Value::as_str).map(str::trim)!=Some(marker) {
                return Err(failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_NOT_PUBLIC"));
            }
            return Ok(());
        }
        if tokio::time::Instant::now()>=deadline {
            return Err(failure("resource_wait.followup_reply","RESOURCE_WAIT_FOLLOWUP_DEADLINE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn general_resources() -> Value {
    let mut selections=vec![json!({"resource_kind":"workspace","resource_id":"default-workspace"}),
        json!({"resource_kind":"process_session","resource_id":"managed-process-session"}),
        json!({"resource_kind":"project_memory","resource_id":"default-project-memory"}),
        json!({"resource_kind":"asset_library","resource_id":"creative-studio-assets"}),
        json!({"resource_kind":"scheduler","resource_id":"installation-scheduler"})];
    if cfg!(feature="browser-use") {selections.push(json!({"resource_kind":"browser","resource_id":"managed-browser"}));}
    if cfg!(feature="computer-use") {selections.push(json!({"resource_kind":"computer","resource_id":"local-desktop"}));}
    Value::Array(selections)
}

pub(super) async fn run(router: &Router, api_key: &str, model: &str, root: &Path) -> Result<(), SmokeFailure> {
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    // Use the actual full General template. The test must exercise ordinary
    // tool selection rather than make an interactive launch the only option.
    let preset=successful_json(router,"resource_wait.preset",Method::POST,"/api/agent-presets/from-template/assistant.general",
        Some(json!({"reuse_existing":false,"display_name":"Live general authorization lifecycle",
            "model":{"provider_id":provider,"model":model}})),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let preset=required_string("resource_wait.preset",&envelope_data("resource_wait.preset",preset)?,
        "/preset/preset_id","RESOURCE_WAIT_PRESET_MISSING")?;
    let pool = nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new()
            .filename(root.join("data/nomifun-backend.db")).read_only(true)).await
        .map_err(|_| failure("resource_wait.bootstrap", "RESOURCE_WAIT_READER_FAILED"))?;
    let store = AgentSessionStore::from_pool(pool.clone()).await
        .map_err(|_| failure("resource_wait.bootstrap", "RESOURCE_WAIT_READER_FAILED"))?;
    for case in ["authorize", "cancel", "expired"] {
        let case_workspace=root.join("authorization-cases").join(case);
        tokio::fs::create_dir_all(&case_workspace).await.map_err(|_|failure("resource_wait.workspace","RESOURCE_WAIT_WORKSPACE_CREATE_FAILED"))?;
        let (session, binding) = create_session_in_workspace(router, &preset, &provider, model,
            general_resources(),Some(&case_workspace)).await?;
        let authorization_url=format!("https://example.invalid/nomifun-authorization/{case}");
        // This external program's TTL is independent of the host observation
        // window and human reply lifetime. Only the fixture acts as the user.
        let expiry=if case=="expired" {1500} else {0};
        let helper = format!(r#"const fs = require('node:fs');
fs.appendFileSync('authorization-launches.txt', 'launch\n');
let expiryTimer;
setTimeout(() => {{
  console.log('AUTHORIZATION_READY {authorization_url}');
  setInterval(() => console.log('WAITING_FOR_USER_ACTION'), 500);
}}, 2000);
setInterval(() => {{
  if (fs.existsSync('authorization.accepted')) {{
    console.log('AUTHORIZATION_CONFIRMED');
    process.exit(0);
  }}
  if ({expiry} && !expiryTimer && fs.existsSync('authorization.expiry-armed')) {{
    expiryTimer = setTimeout(() => {{
      fs.writeFileSync('authorization.expired', 'expired');
      console.log('AUTHORIZATION_EXPIRED');
      process.exit(2);
    }}, {expiry});
  }}
}}, 100);
"#);
        tokio::fs::write(case_workspace.join("setup.cjs"),helper.as_bytes()).await
            .map_err(|_|failure("resource_wait.workspace","RESOURCE_WAIT_HELPER_CREATE_FAILED"))?;
        let prompt = "请帮我配置这个测试应用，安装向导是当前目录的 setup.cjs，用 Node 运行即可。程序会输出浏览器授权地址，请把地址发给我，我会在外部操作，回来后继续验证配置。".to_owned();
        let operation = submit_message(router,&pool,&session,prompt).await?;
        let request = await_question(&store,&session,&operation).await?;
        let before = facts(&store,&session,&operation).await?;
        assert_initial_process_evidence(&before,&operation)?;
        assert_helper_launched_once(&case_workspace).await?;
        assert_public_question(router,&session,&request,&authorization_url).await?;
        let steps = model_steps(&before,&operation);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let stable = facts(&store,&session,&operation).await?;
        if model_steps(&stable,&operation)!=steps || count_event(&stable,"turn/completed",&operation)!=1
            || store.head(&session.clone().into()).await.map_err(|_|failure("resource_wait.idle","RESOURCE_WAIT_HEAD_INVALID"))?.active_turn_id.is_some() {
            return Err(failure("resource_wait.idle","RESOURCE_WAIT_HUMAN_WAIT_KEPT_TURN_ACTIVE"));
        }
        let next = match case {
            "authorize" => {
                tokio::fs::write(case_workspace.join("authorization.accepted"),b"fixture-user-action").await
                    .map_err(|_|failure("resource_wait.authorize","RESOURCE_WAIT_AUTHORIZATION_WRITE_FAILED"))?;
                let next=submit_message(router,&pool,&session,"已完成网页操作，请继续验证配置结果。".into()).await?;
                await_settled(&store,&session,&next,false).await?;
                if !owner_confirmed_authorization(&facts(&store,&session,&next).await?) {
                    return Err(failure("resource_wait.authorize","RESOURCE_WAIT_ORIGINAL_PROCESS_NOT_VERIFIED"));
                }
                next
            },
            "cancel" => {
                let next=submit_message(router,&pool,&session,"我不继续配置了，请停止刚才的授权进程。".into()).await?;
                await_settled(&store,&session,&next,false).await?;
                let current=facts(&store,&session,&next).await?;
                if !current.event_payloads.values().any(|payload|payload.pointer("/event/call/name").and_then(Value::as_str)==Some("cancel_process")) {
                    return Err(failure("resource_wait.cancel","RESOURCE_WAIT_PROCESS_CANCEL_MISSING"));
                }
                next
            },
            _ => {
                // Arm the helper's own TTL after the finite handoff, so model
                // and network latency cannot expire it before this scenario.
                tokio::fs::write(case_workspace.join("authorization.expiry-armed"),b"fixture-external-expiry").await
                    .map_err(|_|failure("resource_wait.expiry","RESOURCE_WAIT_EXPIRY_ARM_FAILED"))?;
                let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
                while !case_workspace.join("authorization.expired").exists() {
                    if tokio::time::Instant::now()>=deadline {return Err(failure("resource_wait.expiry","RESOURCE_WAIT_EXTERNAL_EXPIRY_MISSING"));}
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                let expired=facts(&store,&session,&operation).await?;
                if model_steps(&expired,&operation)!=steps || count_event(&expired,"turn/started",&operation)!=1 {
                    return Err(failure("resource_wait.expiry","RESOURCE_WAIT_EXTERNAL_EXPIRY_REOPENED_MODEL"));
                }
                assert_public_question(router,&session,&request,&authorization_url).await?;
                let next=submit_message(router,&pool,&session,"暂时不继续授权，换个话题。本轮只确认收到，精确回复 NOMIFUN_RESOURCE_WAIT_FOLLOWUP_OK。".into()).await?;
                await_followup_marker(router,&store,&pool,&session,&next,"NOMIFUN_RESOURCE_WAIT_FOLLOWUP_OK").await?;
                next
            },
        };
        if next==operation {return Err(failure("resource_wait.continuation","RESOURCE_WAIT_OLD_TURN_REOPENED"));}
        assert_helper_launched_once(&case_workspace).await?;
        assert_session_binding(router,"resource_wait.binding",&session,&binding).await?;
        eprintln!("NOMIFUN_RESOURCE_WAIT_CASE_OK model={model} case={case} launches=1 finite_handoff=true unified_intake=true context_retained=true");
    }
    pool.close().await;
    eprintln!("NOMIFUN_RESOURCE_WAIT_LIVE_OK model={model} cases=3 full_general_tools=true finite_handoff=true natural_request=true independent_expiry=true credentials_audited_by_runner=true");
    Ok(())
}

#[tokio::test]
async fn resource_wait_fixture_binds_the_current_general_project_without_model_io() {
    let root=tempfile::tempdir().unwrap();
    let fixture=build_fixture(&root,None).await.unwrap();
    let router=fixture.application.router();
    let result=async {
        let model="step-5-preview";
        let provider=configure_stepfun(&router,"fixture-only-no-provider-call",STEPFUN_PLAN_BASE_URL,model).await?;
        let preset=successful_json(&router,"resource_wait.preset",Method::POST,"/api/agent-presets/from-template/assistant.general",
            Some(json!({"reuse_existing":false,"display_name":"General project fixture",
                "model":{"provider_id":provider,"model":model}})),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
        let preset=required_string("resource_wait.preset",&envelope_data("resource_wait.preset",preset)?,
            "/preset/preset_id","RESOURCE_WAIT_PRESET_MISSING")?;
        let(_,binding)=create_session_in_workspace(&router,&preset,&provider,model,general_resources(),Some(&root.path().join("work"))).await?;
        let reference:nomifun_agent_contracts::ResolvedSnapshotRef=serde_json::from_value(binding["resolved_snapshot_ref"].clone())
            .map_err(|_|failure("resource_wait.fixture","RESOURCE_WAIT_FOLLOWUP_SNAPSHOT_INVALID"))?;
        let pool=nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
            .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new().filename(root.path().join("data/nomifun-backend.db")).read_only(true))
            .await.map_err(|_|failure("resource_wait.fixture","RESOURCE_WAIT_READER_FAILED"))?;
        let snapshot=read_followup_snapshot(&pool,&reference).await?;
        assert!(snapshot.content.enabled_capabilities.iter().any(|capability|capability.capability.id.as_ref()=="workspace.files"
            && capability.action_allowlist.contains(&"workspace.files/read".into())
            && capability.actions.iter().any(|action|action.action_id.as_ref()=="workspace.files/read"
                && action.effect_class==nomifun_agent_contracts::EffectClass::ReadLocal)));
        pool.close().await;
        Ok::<(),SmokeFailure>(())
    }.await;
    drop(router);
    fixture.application.close().await.unwrap();
    result.unwrap();
}

#[test]
fn followup_marker_requires_this_turns_exact_canonical_answer() {
    use nomifun_agent_session::CanonicalTurnPublicResult;
    let marker="FOLLOWUP_OK";
    let answered=CanonicalTurnPublicResult {turn_outcome:Some(AgentTurnOutcome::Answered {message_ids:vec!["new-turn-answer".into()]}),
        result_text:Some(marker.into()),user_input_request:None};
    assert_eq!(followup_marker_ids(&answered,marker).unwrap(),&["new-turn-answer".to_owned()]);
    for (outcome,text) in [(AgentTurnOutcome::Failed {message_ids:vec![]},Some(marker.into())),
        (AgentTurnOutcome::Cancelled {message_ids:vec![]},Some(marker.into())),
        (AgentTurnOutcome::Paused {message_ids:vec![]},Some(marker.into())),
        (AgentTurnOutcome::InputRequired {request_id:"next-question".into()},Some(marker.into())),
        (AgentTurnOutcome::Silent {},None),
        (AgentTurnOutcome::Answered {message_ids:vec!["new-turn-answer".into()]},Some("WRONG_MARKER".into())),
        (AgentTurnOutcome::Answered {message_ids:vec!["a".into(),"b".into()]},Some(marker.into()))] {
        assert!(followup_marker_ids(&CanonicalTurnPublicResult {turn_outcome:Some(outcome),result_text:text,user_input_request:None},marker).is_err());
    }
}

#[test]
fn followup_allows_only_platform_scope_preparation_with_a_frozen_readlocal_contract() {
    use nomifun_agent_contracts::EffectClass;
    let args=json!({"path":".","format":"instruction_scope","recursive":false});
    assert!(is_instruction_scope_preparation(0,"read_file","workspace.files","workspace.files/read",&args,Some(EffectClass::ReadLocal)));
    for (step,name,capability,action,arguments,effect) in [
        (1,"read_file","workspace.files","workspace.files/read",args.clone(),Some(EffectClass::ReadLocal)),
        (0,"read_file","workspace.files","workspace.files/read",args.clone(),Some(EffectClass::ExecuteLocal)),
        (0,"read_file","workspace.files","workspace.files/read",args.clone(),None),
        (0,"read_file","workspace.files","workspace.files/read",json!({"path":".","format":"instruction_scope","execution_identity":"administrator"}),Some(EffectClass::ReadLocal)),
        (0,"read_file","workspace.files","workspace.files/read",json!({"path":"authorization.accepted","format":"text"}),Some(EffectClass::ReadLocal)),
        (0,"read_file","workspace.files","workspace.files/read",json!({"path":".","format":"instruction_scope","content":"forged"}),Some(EffectClass::ReadLocal)),
        (0,"start_process","workspace.process","workspace.process/start",args.clone(),Some(EffectClass::ExecuteLocal)),
        (0,"write_file","workspace.files","workspace.files/write",args.clone(),Some(EffectClass::WriteReversible)),
        (0,"rename_path","workspace.files","workspace.files/rename",args.clone(),Some(EffectClass::WriteReversible)),
        (0,"read_file","other.module","workspace.files/read",args.clone(),Some(EffectClass::ReadLocal)),
    ] {assert!(!is_instruction_scope_preparation(step,name,capability,action,&arguments,effect));}
    assert_eq!(static_tool_name("untrusted provider string"),"other_tool");
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn scripted_followup_closes_real_host_turn_with_platform_reads_and_exact_public_marker() {
    const MARKER:&str="NOMIFUN_RESOURCE_WAIT_SCRIPTED_FOLLOWUP_OK";
    let upstream=wiremock::MockServer::start().await;
    let frame=json!({"id":"scripted-followup","choices":[{"index":0,"delta":{"role":"assistant","content":MARKER},"finish_reason":null}]});
    let done=json!({"id":"scripted-followup","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
            .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n")))
        .expect(1).mount(&upstream).await;
    let root=tempfile::tempdir().unwrap();
    let fixture=build_fixture(&root,None).await.unwrap();
    let router=fixture.application.router();
    let result=async {
        let model="step-5-preview";
        let provider=configure_stepfun(&router,"fixture-only-no-real-provider",&format!("{}/v1",upstream.uri()),model).await?;
        let preset=successful_json(&router,"resource_wait.preset",Method::POST,"/api/agent-presets/from-template/assistant.general",
            Some(json!({"reuse_existing":false,"display_name":"Scripted full General followup",
                "model":{"provider_id":provider,"model":model}})),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
        let preset=required_string("resource_wait.preset",&envelope_data("resource_wait.preset",preset)?,
            "/preset/preset_id","RESOURCE_WAIT_PRESET_MISSING")?;
        let(session,_)=create_session_in_workspace(&router,&preset,&provider,model,general_resources(),Some(&root.path().join("work"))).await?;
        let pool=nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
            .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new().filename(root.path().join("data/nomifun-backend.db")).read_only(true))
            .await.map_err(|_|failure("resource_wait.fixture","RESOURCE_WAIT_READER_FAILED"))?;
        let store=AgentSessionStore::from_pool(pool.clone()).await.map_err(|_|failure("resource_wait.fixture","RESOURCE_WAIT_READER_FAILED"))?;
        let operation=start_session_turn(&router,"resource_wait.followup",&session,&uuid::Uuid::now_v7().to_string(),
            format!("只确认收到我的决定，不执行命令、不修改文件，精确回复 {MARKER}。")).await?;
        tokio::time::timeout(Duration::from_secs(30),await_followup_marker(&router,&store,&pool,&session,&operation,MARKER)).await
            .map_err(|_|failure("resource_wait.fixture","RESOURCE_WAIT_SCRIPTED_FOLLOWUP_DEADLINE"))??;
        let completed=facts(&store,&session,&operation).await?;
        assert!(completed.events.iter().filter(|event|event.correlation_id.as_ref()==operation)
            .filter_map(|event|completed.event_payloads.get(event.event_id.as_ref())).any(|payload|
                payload.pointer("/event/event")==Some(&json!("tool_started"))&&payload.pointer("/event/step")==Some(&json!(0))),
            "the actual Host must perform its step-zero instruction preparation");
        assert!(completed.event_payloads.values().any(|payload|matches!(payload.pointer("/event/event").and_then(Value::as_str),
            Some("host_tool_dispatch"|"host_tool_settled"))),"Host-only progress facts must remain valid without Runtime-enum coercion");
        pool.close().await;
        Ok::<(),SmokeFailure>(())
    }.await;
    drop(router);fixture.application.close().await.unwrap();result.unwrap();
}

#[test]
fn followup_platform_instruction_pages_require_exact_bounded_agents_files() {
    use nomifun_agent_contracts::EffectClass;
    let args=json!({"path":"AGENTS.md","offset":0,"limit":16384,"missing_ok":true});
    assert!(is_instruction_page_preparation(0,"read_file","workspace.files","workspace.files/read",&args,Some(EffectClass::ReadLocal)));
    let mut next=args.clone();next["path"]=json!("scope/AGENTS.override.md");next["offset"]=json!(4096);next["expected_sha256"]=json!("a".repeat(64));
    assert!(is_instruction_page_preparation(0,"read_file","workspace.files","workspace.files/read",&next,Some(EffectClass::ReadLocal)));
    for (field,value) in [("path",json!("authorization.accepted")),("limit",json!(32768)),("missing_ok",json!(false)),
        ("offset",json!(1)),("execution_identity",json!("administrator")),("content",json!("forged")),("format",json!("text"))] {
        let mut invalid=args.clone();invalid[field]=value;
        assert!(!is_instruction_page_preparation(0,"read_file","workspace.files","workspace.files/read",&invalid,Some(EffectClass::ReadLocal)));
    }
    assert!(!is_instruction_page_preparation(1,"read_file","workspace.files","workspace.files/read",&args,Some(EffectClass::ReadLocal)));
    assert!(!is_instruction_page_preparation(0,"read_file","workspace.files","workspace.files/read",&args,Some(EffectClass::ExecuteLocal)));
}
