//! Real model and real owned process, with a fixture-controlled external authorization.
//! No Feishu account, browser credential, production data or human approval is fabricated.
use super::*;
use nomifun_agent_contracts::{AgentSessionId, AgentTurnOutcome, AgentWaitReason, OperationId};
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

fn assert_one_launch(facts: &nomifun_agent_session::ChatCausalityFacts, operation: &str) -> Result<(), SmokeFailure> {
    let start = facts.events.iter().find(|event| event.kind.0 == "turn/started"
        && event.correlation_id.as_ref() == operation)
        .ok_or_else(|| failure("resource_wait.evidence", "RESOURCE_WAIT_START_MISSING"))?;
    let launches = facts.events.iter().filter(|event| event.kind.0 == "tool/call-started"
        && event.causation_event_id.as_ref() == Some(&start.event_id)
        && facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload| payload.get("action_id"))
            .and_then(Value::as_str) == Some("workspace.process/start")).count();
    if launches != 1 || count_event(facts, "turn/failed", operation) != 0 {
        return Err(failure("resource_wait.evidence", "RESOURCE_WAIT_FAILED_OR_REPLAYED"));
    }
    Ok(())
}

async fn await_waiting(store: &AgentSessionStore, session: &str, operation: &str)
    -> Result<String, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let outcome = store.turn_outcome(&session.into(), &operation.into()).await
            .map_err(|_| failure("resource_wait.wait", "RESOURCE_WAIT_OUTCOME_INVALID"))?;
        match outcome {
            Some(AgentTurnOutcome::Waiting { request_id: Some(request), reason: AgentWaitReason::UserInput, .. }) => return Ok(request),
            Some(AgentTurnOutcome::Failed { .. } | AgentTurnOutcome::Cancelled { .. }
                | AgentTurnOutcome::Answered { .. } | AgentTurnOutcome::InputRequired { .. }) =>
                return Err(failure("resource_wait.wait", "RESOURCE_WAIT_CLOSED_BEFORE_AUTHORIZATION")),
            _ => {},
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failure("resource_wait.wait", "RESOURCE_WAIT_DECLARATION_DEADLINE"));
        }
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
            Some(AgentTurnOutcome::Paused { .. })) { return Ok("paused"); }
        if tokio::time::Instant::now() >= deadline {
            return Err(failure("resource_wait.settle", "RESOURCE_WAIT_SETTLEMENT_DEADLINE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn emit_report_rejections(facts: &nomifun_agent_session::ChatCausalityFacts, api_key: &str) {
    let calls = facts.event_payloads.values().filter_map(|payload| payload.pointer("/event/call"))
        .filter(|call| call["name"] == "report_completion").filter_map(|call| call["call_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let messages = facts.event_payloads.values().filter_map(|payload| payload.pointer("/event/result"))
        .filter(|result| result["is_error"] == true && result["call_id"].as_str().is_some_and(|id| calls.contains(id)))
        .filter_map(|result| serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(result.clone()).ok())
        .map(|result| result.output_text().replace(api_key, "[REDACTED]").chars().take(4096).collect::<String>())
        .collect::<Vec<_>>();
    if !messages.is_empty() { eprintln!("NOMIFUN_RESOURCE_WAIT_REPORT_REJECTIONS {}", json!(messages)); }
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

pub(super) async fn run(router: &Router, api_key: &str, model: &str, root: &Path) -> Result<(), SmokeFailure> {
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset, _) = create_agent_preset(router, &provider, model,
        &[("workspace.process", &["workspace.process/start", "workspace.process/poll", "workspace.process/cancel"], "process_session")]).await?;
    let pool = nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new()
            .filename(root.join("data/nomifun-backend.db")).read_only(true)).await
        .map_err(|_| failure("resource_wait.bootstrap", "RESOURCE_WAIT_READER_FAILED"))?;
    let store = AgentSessionStore::from_pool(pool.clone()).await
        .map_err(|_| failure("resource_wait.bootstrap", "RESOURCE_WAIT_READER_FAILED"))?;
    for cancelled in [false, true] {
        let (session, binding) = create_session(router, &preset, &provider, model,
            json!([{"resource_kind":"process_session","resource_id":"managed-process-session"}])).await?;
        let projection = successful_json(router, "resource_wait.workspace", Method::GET,
            format!("/api/agent-sessions/{session}/projection"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let projection = envelope_data("resource_wait.workspace", projection)?;
        let workspace = projection.pointer("/extra/workspace").and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .ok_or_else(|| failure("resource_wait.workspace", "RESOURCE_WAIT_WORKSPACE_MISSING"))?;
        let helper = "const fs=require('node:fs');fs.appendFileSync('authorization-launches.txt','launch\\n');console.log('AUTHORIZATION_READY');const timer=setInterval(()=>{if(fs.existsSync('authorization.accepted')){clearInterval(timer);console.log('AUTHORIZATION_CONFIRMED');}},100);";
        let launch = json!({"component":"node","args":["-e",helper],"wait_ms":1000,"timeout_ms":300000});
        let prompt = format!("这是外部授权等待的真实进程验收。先用 start_process 严格执行以下参数对象且仅启动一次：{launch}\n\
            进程输出 AUTHORIZATION_READY 后，用 update_plan 记录两步：启动授权助手已完成、等待人工授权并核对真实结果进行中。随后立即单独调用 request_user_input，请求我在外部完成授权，remaining_work 保留等待原进程确认。\n\
            不要自行创建 authorization.accepted，不要修改启动记录，不要取消进程，不要重启或重复启动。\n\
            正常等待不是任务完成，也不是失败。不要通过模型反复轮询等待人工；用正式输入等待控制。\n\
            我操作后只检查原进程真实结果，收到 AUTHORIZATION_CONFIRMED 再更新计划并报告完成，最后回复 NOMIFUN_RESOURCE_WAIT_OK。\n\
            若助手真实超时，保留已完成步骤并请求人工处理，不声称授权成功。");
        let operation = start_session_turn(router, "resource_wait.start", &session,
            &uuid::Uuid::now_v7().to_string(), prompt).await?;
        let request = await_waiting(&store, &session, &operation).await?;
        let before = facts(&store, &session, &operation).await?;
        assert_one_launch(&before, &operation)?;
        let before_steps = model_steps(&before, &operation);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let parked = facts(&store, &session, &operation).await?;
        if model_steps(&parked, &operation) != before_steps || count_event(&parked, "turn/completed", &operation) != 0 {
            return Err(failure("resource_wait.park", "RESOURCE_WAIT_MODEL_SPUN_OR_CLOSED"));
        }
        let plan = successful_json(router, "resource_wait.plan", Method::GET,
            format!("/api/agent-sessions/{session}/task-plan"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let plan = envelope_data("resource_wait.plan", plan)?;
        if plan.pointer("/turn_outcome/kind").and_then(Value::as_str) != Some("waiting")
            || plan.pointer("/turn_outcome/request_id").and_then(Value::as_str) != Some(request.as_str()) {
            return Err(failure("resource_wait.plan", "RESOURCE_WAIT_PLAN_PROJECTION_INVALID"));
        }
        if cancelled {
            successful_json(router, "resource_wait.cancel", Method::POST,
                format!("/api/agent-sessions/{session}/turns/cancel"),
                Some(json!({"idempotency_key":uuid::Uuid::now_v7().to_string(),"expected_turn_id":plan["turn_id"]})),
                LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        } else {
            successful_json(router, "resource_wait.reply", Method::POST,
                format!("/api/agent-sessions/{session}/turns/steer"),
                Some(json!({"input":{"content":"我正在完成外部授权，实际结果马上由原进程返回。请只核对原进程结果后继续，不要重新启动助手。"},
                    "expected_wait":{"turn_id":plan["turn_id"],"wait_seq":plan["turn_outcome"]["wait_seq"]},
                    "idempotency_key":uuid::Uuid::now_v7().to_string()})),
                LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
            tokio::fs::write(workspace.join("authorization.accepted"), b"fixture-user-action").await
                .map_err(|_| failure("resource_wait.authorize", "RESOURCE_WAIT_AUTHORIZATION_WRITE_FAILED"))?;
        }
        let outcome = match await_settled(&store, &session, &operation, cancelled).await {
            Ok(outcome) => outcome,
            Err(error) => { emit_report_rejections(&facts(&store, &session, &operation).await?, api_key); return Err(error); }
        };
        let settled = facts(&store, &session, &operation).await?;
        assert_one_launch(&settled, &operation)?;
        if !cancelled && !owner_confirmed_authorization(&settled) {
            return Err(failure("resource_wait.evidence", "RESOURCE_WAIT_AUTHORIZATION_NOT_CONFIRMED"));
        }
        if outcome == "paused" { emit_report_rejections(&settled, api_key); }
        let launches = tokio::fs::read_to_string(workspace.join("authorization-launches.txt")).await
            .map_err(|_| failure("resource_wait.evidence", "RESOURCE_WAIT_LAUNCH_LOG_MISSING"))?;
        if launches != "launch\n" {
            return Err(failure("resource_wait.evidence", "RESOURCE_WAIT_HELPER_REPEATED"));
        }
        assert_session_binding(router, "resource_wait.binding", &session, &binding).await?;
        eprintln!("NOMIFUN_RESOURCE_WAIT_CASE_OK model={model} case={} outcome={outcome} launches=1 model_parked=true waiting_projection=true",
            if cancelled { "cancel" } else { "authorize" });
    }
    pool.close().await;
    eprintln!("NOMIFUN_RESOURCE_WAIT_LIVE_OK model={model} cases=2 credentials_audited_by_runner=true");
    Ok(())
}
