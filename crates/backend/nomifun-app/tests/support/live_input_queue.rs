//! Real model, original owned process, canonical input modes and a full Host restart.
//! Every file, provider and Session belongs to this disposable smoke root.
use super::*;
use nomifun_agent_contracts::{EventId, OperationId};
use nomifun_agent_session::{AgentSessionStore, SessionInputClaim, TurnReceiptStatus};

const T_MARKER: &str = "NOMIFUN_INPUT_QUEUE_T_C_OK";
const A_MARKER: &str = "NOMIFUN_INPUT_QUEUE_A_OK";
const B_MARKER: &str = "NOMIFUN_INPUT_QUEUE_B_OK";
const RESUME_MARKER: &str = "NOMIFUN_INPUT_QUEUE_RESTART_OK";

fn failed(phase: &'static str, code: &'static str) -> SmokeFailure {
    SmokeFailure::new(phase, code, 422)
}

#[derive(Clone)]
struct Submitted {
    id: EventId,
    body: Value,
}

struct RestartPlan {
    session: String,
    pending: Submitted,
    cancelled: Submitted,
    turn_count: i64,
    stopped_operation: OperationId,
    stopped_model_steps: usize,
}

async fn reader(root: &Path) -> Result<(nomifun_db::SqlitePool, AgentSessionStore), SmokeFailure> {
    let pool = nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new()
            .filename(root.join("data/nomifun-backend.db")).read_only(true)).await
        .map_err(|_| failed("input_queue.reader", "INPUT_QUEUE_READER_FAILED"))?;
    let store = AgentSessionStore::from_pool(pool.clone()).await
        .map_err(|_| failed("input_queue.reader", "INPUT_QUEUE_READER_FAILED"))?;
    Ok((pool, store))
}

async fn post_input(router: &Router, session: &str, body: Value) -> Result<Submitted, SmokeFailure> {
    let response = successful_json(router, "input_queue.accept", Method::POST,
        format!("/api/agent-sessions/{session}/messages"), Some(body.clone()),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let accepted = envelope_data("input_queue.accept", response)?;
    if accepted["status"] != "accepted" || accepted["agent_session_id"] != session {
        return Err(failed("input_queue.accept", "INPUT_QUEUE_ACCEPTANCE_INVALID"));
    }
    let id = required_string("input_queue.accept", &accepted, "/message_id", "INPUT_QUEUE_MESSAGE_ID_MISSING")?;
    Ok(Submitted { id: id.into(), body })
}

fn body(content: String, delivery: Value) -> Value {
    json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":content},"delivery":delivery})
}

async fn replay(router: &Router, session: &str, submitted: &Submitted) -> Result<(), SmokeFailure> {
    let response = successful_json(router, "input_queue.replay", Method::POST,
        format!("/api/agent-sessions/{session}/messages"), Some(submitted.body.clone()),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let accepted = envelope_data("input_queue.replay", response)?;
    if accepted["message_id"].as_str() != Some(submitted.id.as_ref()) || accepted["replayed"] != true {
        return Err(failed("input_queue.replay", "INPUT_QUEUE_REPLAY_CREATED_INPUT"));
    }
    Ok(())
}

async fn queue(router: &Router, session: &str) -> Result<Value, SmokeFailure> {
    let response = successful_json(router, "input_queue.snapshot", Method::GET,
        format!("/api/agent-sessions/{session}/input-queue"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let snapshot = envelope_data("input_queue.snapshot", response)?;
    if snapshot["version"].as_u64().is_none() || !snapshot["items"].is_array() {
        return Err(failed("input_queue.snapshot", "INPUT_QUEUE_SNAPSHOT_INVALID"));
    }
    Ok(snapshot)
}

async fn control(router: &Router, session: &str, action: Value) -> Result<Value, SmokeFailure> {
    let snapshot = queue(router, session).await?;
    let response = successful_json(router, "input_queue.control", Method::POST,
        format!("/api/agent-sessions/{session}/input-queue"),
        Some(json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),
            "expected_version":snapshot["version"],"action":action})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    envelope_data("input_queue.control", response)
}

async fn claim(store: &AgentSessionStore, session: &str, input: &EventId) -> Result<SessionInputClaim, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + TURN_RESULT_DEADLINE;
    loop {
        if let Some(claim) = store.session_input_claim(&session.into(), input).await
            .map_err(|_| failed("input_queue.claim", "INPUT_QUEUE_CLAIM_INVALID"))? { return Ok(claim); }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed("input_queue.claim", "INPUT_QUEUE_NOT_CLAIMED"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn completed(store: &AgentSessionStore, session: &str, claim: &SessionInputClaim, marker: &str)
    -> Result<u64, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let receipt = store.read_turn_receipt(&session.into(), &claim.operation_id).await
            .map_err(|_| failed("input_queue.complete", "INPUT_QUEUE_RECEIPT_INVALID"))?;
        if matches!(receipt.status, TurnReceiptStatus::Failed | TurnReceiptStatus::Cancelled) {
            return Err(failed("input_queue.complete", "INPUT_QUEUE_TURN_FAILED"));
        }
        if receipt.status == TurnReceiptStatus::Completed {
            let facts = store.turn_output_facts(&session.into(), &claim.operation_id).await
                .map_err(|_| failed("input_queue.complete", "INPUT_QUEUE_FACTS_INVALID"))?;
            let public = nomifun_agent_session::canonical_turn_public_result(&facts, &receipt)
                .map_err(|_| failed("input_queue.complete", "INPUT_QUEUE_OUTCOME_INVALID"))?;
            if public.result_text.as_deref().map(str::trim) != Some(marker) {
                return Err(failed("input_queue.complete", "INPUT_QUEUE_EXACT_REPLY_MISSING"));
            }
            return facts.events.iter().find(|event| event.kind.0 == "turn/completed"
                && event.correlation_id.as_ref() == claim.operation_id.as_ref()).map(|event| event.seq)
                .ok_or_else(|| failed("input_queue.complete", "INPUT_QUEUE_TERMINAL_MISSING"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed("input_queue.complete", "INPUT_QUEUE_COMPLETION_DEADLINE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn counts(pool: &nomifun_db::SqlitePool, session: &str) -> Result<(i64, i64), SmokeFailure> {
    nomifun_db::sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_turns WHERE session_id=?), (SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/user-accepted')")
        .bind(session).bind(session).fetch_one(pool).await
        .map_err(|_| failed("input_queue.counts", "INPUT_QUEUE_COUNTS_INVALID"))
}

async fn model_steps(store:&AgentSessionStore,session:&str,operation:&OperationId)->Result<usize,SmokeFailure> {
    let facts=store.turn_output_facts(&session.into(),operation).await
        .map_err(|_|failed("input_queue.stop","INPUT_QUEUE_FACTS_INVALID"))?;
    Ok(facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        && event.correlation_id.as_ref()==operation.as_ref()
        && facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.pointer("/event/event"))
            .and_then(Value::as_str)==Some("model_step_started")).count())
}

async fn await_gate(root: &Path, name: &str, store: &AgentSessionStore, session: &str, turn: &SessionInputClaim)
    -> Result<(), SmokeFailure> {
    let ready = root.join("work").join(format!("{name}.ready"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    while !ready.is_file() {
        let receipt = store.read_turn_receipt(&session.into(), &turn.operation_id).await
            .map_err(|_| failed("input_queue.gate", "INPUT_QUEUE_RECEIPT_INVALID"))?;
        if receipt.status != TurnReceiptStatus::Running {
            return Err(failed("input_queue.gate", "INPUT_QUEUE_GATE_NOT_STARTED"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed("input_queue.gate", "INPUT_QUEUE_GATE_START_DEADLINE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Ok(())
}

async fn release(root: &Path, name: &str) -> Result<(), SmokeFailure> {
    tokio::fs::write(root.join("work").join(format!("{name}.release")), b"fixture-release").await
        .map_err(|_| failed("input_queue.gate", "INPUT_QUEUE_GATE_RELEASE_FAILED"))
}

fn gate_prompt(root: &Path, name: &str) -> String {
    let args = json!({"command":"node","args":[root.join("work/queue-gate.cjs").to_string_lossy(),name],
        "cwd":root.join("work").to_string_lossy(),"wait_ms":30000});
    format!("本轮是隔离输入队列验证。先且只调用一次 start_process，参数严格为 {args}。调用工具前不要输出说明文字，不要产生中间公开回复。这是测试自有进程，外部测试会释放它；不要修改文件或重新启动它。等该调用实际结束后，本轮默认只回复 NOMIFUN_INPUT_QUEUE_T_INITIAL_OK；若本轮收到新的补充指令，则按补充要求回复。后续独立用户输入各自执行，不合并任务。")
}

async fn assert_held(router: &Router, store: &AgentSessionStore, session: &str, pending: &Submitted)
    -> Result<(), SmokeFailure> {
    let snapshot = queue(router, session).await?;
    if !snapshot["items"].as_array().is_some_and(|items| items.iter().any(|item|
        item["message_id"].as_str() == Some(pending.id.as_ref()) && item["held"] == true)) {
        return Err(failed("input_queue.hold", "INPUT_QUEUE_HOLD_MISSING"));
    }
    if store.session_input_claim(&session.into(), &pending.id).await
        .map_err(|_| failed("input_queue.hold", "INPUT_QUEUE_CLAIM_INVALID"))?.is_some() {
        return Err(failed("input_queue.hold", "INPUT_QUEUE_HELD_INPUT_STARTED"));
    }
    Ok(())
}

async fn first_run(router: &Router, api_key: &str, model: &str, root: &Path) -> Result<RestartPlan, SmokeFailure> {
    official_runtime_build(router).await?;
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset, _) = create_agent_preset(router, &provider, model,
        &[("workspace.process", &["workspace.process/start"], "process_session")]).await?;
    let (session, _) = create_session_in_workspace(router, &preset, &provider, model,
        json!([{"resource_kind":"process_session","resource_id":"managed-process-session"}]),
        Some(&root.join("work"))).await?;
    tokio::fs::write(root.join("work/queue-gate.cjs"), br#"const fs=require('node:fs');
const name=process.argv[2];
if(!['normal','stop'].includes(name))process.exit(3);
fs.appendFileSync(name+'.launches','launch\n');
fs.writeFileSync(name+'.ready','ready');
const check=setInterval(()=>{if(fs.existsSync(name+'.release')){clearInterval(check);console.log('QUEUE_GATE_RELEASED');process.exit(0)}},25);
setTimeout(()=>{console.error('QUEUE_GATE_FIXTURE_EXPIRED');process.exit(2)},120000);
"#).await.map_err(|_| failed("input_queue.gate", "INPUT_QUEUE_GATE_CREATE_FAILED"))?;
    let (pool, store) = reader(root).await?;
    let root_input = post_input(router, &session, body(gate_prompt(root,"normal"),json!({"kind":"queue"}))).await?;
    let root_claim = claim(&store,&session,&root_input.id).await?;
    await_gate(root,"normal",&store,&session,&root_claim).await?;
    // Omit delivery for A to exercise the public default through the real Host.
    let a = post_input(router,&session,json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":format!("本轮不要调用工具，只精确回复 {A_MARKER}。")}})).await?;
    let b = post_input(router,&session,body(format!("本轮不要调用工具，只精确回复 {B_MARKER}。"),json!({"kind":"queue"}))).await?;
    let c = post_input(router,&session,body(format!("这是当前回合的补充：原进程实际结束后，本轮只精确回复 {T_MARKER}，不要执行其他排队任务。"),
        json!({"kind":"prefer_current","turn_id":root_input.id}))).await?;
    let before = counts(&pool,&session).await?;
    for input in [&root_input,&a,&b,&c] {replay(router,&session,input).await?;}
    if counts(&pool,&session).await? != before {
        return Err(failed("input_queue.replay", "INPUT_QUEUE_REPLAY_CHANGED_COUNTS"));
    }
    if store.session_input_claim(&session.clone().into(),&a.id).await.map_err(|_|failed("input_queue.claim","INPUT_QUEUE_CLAIM_INVALID"))?.is_some()
        || store.session_input_claim(&session.clone().into(),&b.id).await.map_err(|_|failed("input_queue.claim","INPUT_QUEUE_CLAIM_INVALID"))?.is_some() {
        return Err(failed("input_queue.claim", "INPUT_QUEUE_QUEUED_INPUT_STOLEN"));
    }
    release(root,"normal").await?;
    let c_claim=claim(&store,&session,&c.id).await?;
    if !c_claim.steering || c_claim.operation_id!=root_claim.operation_id {
        return Err(failed("input_queue.steer","INPUT_QUEUE_STEERING_TARGET_CHANGED"));
    }
    let t_end=completed(&store,&session,&root_claim,T_MARKER).await?;
    if store.session_input_delivery(&session.clone().into(),&c.id).await
        .map_err(|_|failed("input_queue.steer","INPUT_QUEUE_DELIVERY_INVALID"))?["state"]!="claimed" {
        return Err(failed("input_queue.steer","INPUT_QUEUE_NATIVE_DELIVERY_UNCONFIRMED"));
    }
    let original=store.turn_output_facts(&session.clone().into(),&root_claim.operation_id).await
        .map_err(|_|failed("input_queue.steer","INPUT_QUEUE_FACTS_INVALID"))?;
    let native_inputs=original.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        && event.correlation_id.as_ref()==root_claim.operation_id.as_ref())
        .filter_map(|event|original.event_payloads.get(event.event_id.as_ref()))
        .filter(|payload|payload.pointer("/event/event").and_then(Value::as_str)==Some("steering_inputs"))
        .filter_map(|payload|payload.pointer("/event/inputs").and_then(Value::as_array))
        .flatten().filter_map(|input|input.get("message_id").and_then(Value::as_str)).collect::<Vec<_>>();
    if native_inputs!=vec![c.id.as_ref()] {
        return Err(failed("input_queue.steer","INPUT_QUEUE_QUEUED_CONTENT_ENTERED_CURRENT_TURN"));
    }
    let a_claim=claim(&store,&session,&a.id).await?;
    let a_end=completed(&store,&session,&a_claim,A_MARKER).await?;
    let b_claim=claim(&store,&session,&b.id).await?;
    completed(&store,&session,&b_claim,B_MARKER).await?;
    let settled_counts=counts(&pool,&session).await?;
    replay(router,&session,&c).await?;
    if counts(&pool,&session).await?!=settled_counts {
        return Err(failed("input_queue.replay","INPUT_QUEUE_CLOSED_TARGET_REPLAY_RESTARTED"));
    }
    let a_start:u64=nomifun_db::sqlx::query_scalar::<_,i64>("SELECT seq FROM agent_events WHERE session_id=? AND event_id=?")
        .bind(&session).bind(a_claim.claim_event_id.as_ref()).fetch_one(&pool).await
        .map_err(|_|failed("input_queue.order","INPUT_QUEUE_START_MISSING"))? as u64;
    let b_start:u64=nomifun_db::sqlx::query_scalar::<_,i64>("SELECT seq FROM agent_events WHERE session_id=? AND event_id=?")
        .bind(&session).bind(b_claim.claim_event_id.as_ref()).fetch_one(&pool).await
        .map_err(|_|failed("input_queue.order","INPUT_QUEUE_START_MISSING"))? as u64;
    if a_claim.steering || b_claim.steering || a_claim.operation_id==b_claim.operation_id
        || a_claim.operation_id==root_claim.operation_id || a_start<=t_end || b_start<=a_end {
        return Err(failed("input_queue.order","INPUT_QUEUE_TURNS_MERGED_OR_REORDERED"));
    }
    if tokio::fs::read_to_string(root.join("work/normal.launches")).await.ok().as_deref()!=Some("launch\n") {
        return Err(failed("input_queue.process","INPUT_QUEUE_GATE_REPEATED"));
    }
    eprintln!("NOMIFUN_INPUT_QUEUE_PHASE phase=mixed_turns_verified");
    let stop_input=post_input(router,&session,body(gate_prompt(root,"stop"),json!({"kind":"queue"}))).await?;
    let stop_claim=claim(&store,&session,&stop_input.id).await?;
    await_gate(root,"stop",&store,&session,&stop_claim).await?;
    let pending=post_input(router,&session,body(format!("本轮不要调用工具，只精确回复 {RESUME_MARKER}。"),json!({"kind":"queue"}))).await?;
    let cancelled=post_input(router,&session,body("这条测试排队输入将被用户移除，不应执行。".into(),json!({"kind":"queue"}))).await?;
    control(router,&session,json!({"kind":"remove","message_id":cancelled.id})).await?;
    replay(router,&session,&cancelled).await?;
    let stopped_model_steps=model_steps(&store,&session,&stop_claim.operation_id).await?;
    let response=successful_json(router,"input_queue.stop",Method::POST,
        format!("/api/agent-sessions/{session}/turns/cancel"),
        Some(json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),"expected_turn_id":stop_input.id})),
        LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let _=envelope_data("input_queue.stop",response)?;
    release(root,"stop").await?;
    let deadline=tokio::time::Instant::now()+SHUTDOWN_DEADLINE;
    loop {
        let receipt=store.read_turn_receipt(&session.clone().into(),&stop_claim.operation_id).await
            .map_err(|_|failed("input_queue.stop","INPUT_QUEUE_RECEIPT_INVALID"))?;
        if receipt.status==TurnReceiptStatus::Cancelled {break;}
        if tokio::time::Instant::now()>=deadline {return Err(failed("input_queue.stop","INPUT_QUEUE_STOP_DEADLINE"));}
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert_held(router,&store,&session,&pending).await?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert_held(router,&store,&session,&pending).await?;
    if store.session_input_delivery(&session.clone().into(),&cancelled.id).await
        .map_err(|_|failed("input_queue.remove","INPUT_QUEUE_DELIVERY_INVALID"))?["state"]!="cancelled" {
        return Err(failed("input_queue.remove","INPUT_QUEUE_REMOVED_INPUT_REVIVED"));
    }
    let turn_count=counts(&pool,&session).await?.0;
    if turn_count!=4 || model_steps(&store,&session,&stop_claim.operation_id).await?!=stopped_model_steps {
        return Err(failed("input_queue.counts","INPUT_QUEUE_UNEXPECTED_EXECUTION_AFTER_STOP"));
    }
    eprintln!("NOMIFUN_INPUT_QUEUE_PHASE phase=stop_hold_verified");
    pool.close().await;
    Ok(RestartPlan {session,pending,cancelled,turn_count,stopped_operation:stop_claim.operation_id,stopped_model_steps})
}

async fn after_restart(router:&Router,root:&Path,plan:&RestartPlan)->Result<(),SmokeFailure> {
    let (pool,store)=reader(root).await?;
    assert_held(router,&store,&plan.session,&plan.pending).await?;
    replay(router,&plan.session,&plan.pending).await?;
    replay(router,&plan.session,&plan.cancelled).await?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert_held(router,&store,&plan.session,&plan.pending).await?;
    if counts(&pool,&plan.session).await?.0!=plan.turn_count
        || model_steps(&store,&plan.session,&plan.stopped_operation).await?!=plan.stopped_model_steps {
        return Err(failed("input_queue.restart","INPUT_QUEUE_RESTART_IGNORED_HOLD"));
    }
    eprintln!("NOMIFUN_INPUT_QUEUE_PHASE phase=restart_hold_verified");
    control(router,&plan.session,json!({"kind":"resume"})).await?;
    let next=claim(&store,&plan.session,&plan.pending.id).await?;
    if next.steering {return Err(failed("input_queue.resume","INPUT_QUEUE_RESUME_BECAME_STEER"));}
    completed(&store,&plan.session,&next,RESUME_MARKER).await?;
    if counts(&pool,&plan.session).await?.0!=plan.turn_count+1
        || store.session_input_claim(&plan.session.clone().into(),&plan.cancelled.id).await
            .map_err(|_|failed("input_queue.restart","INPUT_QUEUE_CLAIM_INVALID"))?.is_some() {
        return Err(failed("input_queue.restart","INPUT_QUEUE_REPLAY_REVIVED_CANCELLED_INPUT"));
    }
    pool.close().await;
    eprintln!("NOMIFUN_INPUT_QUEUE_PHASE phase=resume_verified");
    Ok(())
}

async fn close(fixture:LiveFixture)->Result<(),SmokeFailure> {
    let LiveFixture {_environment,application}=fixture;
    let result=hard_deadline("input_queue.shutdown","INPUT_QUEUE_SHUTDOWN_DEADLINE",SHUTDOWN_DEADLINE,async {
        application.close().await.map_err(|_|failed("input_queue.shutdown","INPUT_QUEUE_SHUTDOWN_FAILED"))
    }).await;
    drop(_environment);
    result
}

pub(super) async fn run()->Result<(),SmokeFailure> {
    let model=live_model()?;
    if model!="step-5-preview" {return Err(failed("input_queue.model","INPUT_QUEUE_MODEL_INVALID"));}
    let api_key=required_secret_from_stdin()?;
    let root=tempfile::tempdir().map_err(|_|failed("input_queue.bootstrap","INPUT_QUEUE_ROOT_CREATE_FAILED"))?;
    let first=build_fixture(&root,None).await?;
    let router=first.application.router();
    let result=AssertUnwindSafe(first_run(&router,api_key.as_str(),&model,root.path()).boxed_local()).catch_unwind().await
        .unwrap_or_else(|_|Err(failed("input_queue.first","INPUT_QUEUE_FIXTURE_PANICKED")));
    drop(router);
    let closed=close(first).await;
    if result.is_err() {
        emit_live_runtime_progress_trace(root.path()).await;
        emit_live_turn_failure_trace(root.path()).await;
    }
    ensure_secret_not_persisted(root.path(),api_key.as_bytes()).await?;
    let plan=result?;
    closed?;
    let second=build_fixture(&root,None).await?;
    let router=second.application.router();
    let result=AssertUnwindSafe(after_restart(&router,root.path(),&plan).boxed_local()).catch_unwind().await
        .unwrap_or_else(|_|Err(failed("input_queue.restart","INPUT_QUEUE_FIXTURE_PANICKED")));
    drop(router);
    let closed=close(second).await;
    if result.is_err() {
        emit_live_runtime_progress_trace(root.path()).await;
        emit_live_turn_failure_trace(root.path()).await;
    }
    ensure_secret_not_persisted(root.path(),api_key.as_bytes()).await?;
    result?;
    closed?;
    eprintln!("NOMIFUN_INPUT_QUEUE_LIVE_OK model={model} queue_turns=2 exact_steer=true stop_hold=true host_restarts=1 resumed_turns=1 idempotency=true cancelled_replay_fenced=true");
    Ok(())
}
