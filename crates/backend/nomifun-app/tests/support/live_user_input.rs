//! Actual model, managed tools, canonical input handoff and subsequent Turn.
use super::*;

async fn public_history(router: &Router, phase: &'static str, session: &str) -> Result<Vec<Value>, SmokeFailure> {
    let response = successful_json(router, phase, Method::GET,
        format!("/api/agent-sessions/{session}/message-history?page_size=100"),
        None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    envelope_data(phase, response)?.get("items").and_then(Value::as_array).cloned()
        .ok_or_else(|| SmokeFailure::new(phase, "INPUT_RENDERER_HISTORY_MISSING", 502))
}

pub(super) async fn run(router: &Router, api_key: &str, model: &str, root: &Path) -> Result<(), SmokeFailure> {
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset, _) = create_agent_preset(router, &provider, model,
        &[("workspace.process", &["workspace.process/start"], "process_session")]).await?;
    let (session, binding) = create_session(router, &preset, &provider, model,
        json!([{"resource_kind":"process_session","resource_id":"managed-process-session"}])).await?;
    let cursor = session_message_cursor(router, "input.cursor", &session).await?;
    // Keep the two real read-only observations within the existing durable
    // output limit: this case verifies the input handoff, not huge CLI help.
    let operation = start_session_turn(router, "input.start", &session, &uuid::Uuid::now_v7().to_string(),
        r#"我需要一个开学自我介绍PPT，但还没有给你我的个人资料。本轮只准备并询问资料：
先完成两个独立的 start_process 调用，严格使用如下两个参数对象：
{"component":"officecli","args":["help","batch"],"wait_ms":30000}
{"component":"officecli","args":["--version"],"wait_ms":30000}
这两次都是只读查询；不要用 shell 合并，不要省略任一调用。
两个调用实际结束后，询问我的姓名和爱好，等我回答后再制作。此时不生成文件，不编造个人资料。
缺少资料是正常等待用户，不能声称PPT已交付。"#.into()).await?;
    let pool = nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new()
            .filename(root.join("data/nomifun-backend.db")).read_only(true)).await
        .map_err(|_| SmokeFailure::new("input.facts", "INPUT_FACT_READER_FAILED", 500))?;
    let store = nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await
        .map_err(|_| SmokeFailure::new("input.facts", "INPUT_FACT_READER_FAILED", 500))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let request;
    loop {
        let (messages, _) = session_messages_after(router, "input.messages", &session, cursor).await?;
        if let Some(code) = first_durable_error_code(&messages) { return Err(SmokeFailure::new("input.messages", code, 422)); }
        let events = session_events(router, &session).await?;
        if events.iter().any(|event| event["kind"] == "turn/failed"
            && event["correlation_id"].as_str() == Some(operation.as_str())) {
            return Err(SmokeFailure::new("input.provider", "INPUT_HANDOFF_TURN_FAILED", 422));
        }
        if let Some(terminal) = events.iter().find(|event| event["kind"] == "turn/completed"
            && event["correlation_id"].as_str() == Some(operation.as_str())) {
            request = terminal.pointer("/payload/value/user_input_request").cloned()
                .ok_or_else(|| SmokeFailure::new("input.terminal", "USER_INPUT_REQUEST_MISSING", 422))?;
            let typed: nomifun_agent_contracts::AgentUserInputRequest = serde_json::from_value(request.clone())
                .map_err(|_| SmokeFailure::new("input.terminal", "USER_INPUT_REQUEST_INVALID", 422))?;
            typed.validate().map_err(|_| SmokeFailure::new("input.terminal", "USER_INPUT_REQUEST_INVALID", 422))?;
            // The public event endpoint returns Stored references for larger
            // tool receipts. Use the existing typed canonical reader to hydrate
            // the exact Turn; absence of inline JSON is not absence of execution.
            let facts = store.turn_output_facts(&session.clone().into(), &operation.clone().into()).await
                .map_err(|_| SmokeFailure::new("input.facts", "INPUT_FACT_READER_FAILED", 500))?;
            let failure = || SmokeFailure::new("input.evidence", "INPUT_HANDOFF_TOOL_OR_COMPLETION_SCOPE_INVALID", 422);
            let turn_start = facts.events.iter().find(|event| event.kind.0 == "turn/started"
                && event.correlation_id.as_ref() == operation.as_str()).ok_or_else(failure)?;
            let starts = facts.events.iter().filter(|event| event.kind.0 == "tool/call-started"
                && event.causation_event_id.as_ref() == Some(&turn_start.event_id)
                && facts.event_payloads[event.event_id.as_ref()]["action_id"] == "workspace.process/start").collect::<Vec<_>>();
            let mut successful = 0;
            let mut observations = Vec::new();
            let mut arguments = Vec::new();
            for call in &starts {
                let payload = &facts.event_payloads[call.event_id.as_ref()];
                let id: nomifun_agent_contracts::chat_model::ToolCallId = payload["call_id"].as_str()
                    .ok_or_else(failure)?.into();
                let proposal = facts.events.iter().find(|event| event.kind.0 == "runtime/progress-recorded"
                    && event.correlation_id.as_ref() == operation.as_str()
                    && facts.event_payloads[event.event_id.as_ref()].pointer("/event/event").and_then(Value::as_str) == Some("tool_call_completed")
                    && facts.event_payloads[event.event_id.as_ref()].pointer("/event/call/call_id").and_then(Value::as_str) == Some(id.as_ref()))
                    .ok_or_else(failure)?;
                arguments.push(facts.event_payloads[proposal.event_id.as_ref()].pointer("/event/call/arguments")
                    .cloned().ok_or_else(failure)?);
                let result = facts.events.iter().find(|event| event.kind.0 == "tool/result-recorded"
                    && event.causation_event_id.as_ref() == Some(&call.event_id)
                    && facts.event_payloads[event.event_id.as_ref()]["operation_id"] == payload["operation_id"])
                    .ok_or_else(failure)?;
                let result_payload = &facts.event_payloads[result.event_id.as_ref()];
                let result: nomifun_agent_runtime::AgentToolResult = serde_json::from_value(result_payload["output"].clone())
                    .map_err(|_| failure())?;
                result.validate_for(&id).map_err(|_| failure())?;
                let command: Value = serde_json::from_str(&result.output_text())
                    .map_err(|_| SmokeFailure::new("input.evidence", "INPUT_COMMAND_RESULT_INVALID", 422))?;
                observations.push(json!({"state":command["state"], "exit_code":command["exit_code"], "cleanup":command["cleanup"], "error":result.is_error, "code":command["code"]}));
                if result_payload["error"].is_null() && !result.is_error && command["state"] == "exited"
                    && command["success"] == true && command["exit_code"] == 0
                    && command.pointer("/cleanup/reaped") == Some(&json!(true)) { successful += 1; }
            }
            let expected = [json!({"component":"officecli","args":["help","batch"],"wait_ms":30000}),
                json!({"component":"officecli","args":["--version"],"wait_ms":30000})];
            if starts.len() != 2 || successful != 2 || !expected.iter().all(|expected| arguments.contains(expected))
                || facts.events.iter().any(|event|
                event.kind.0 == "turn/failed" || facts.event_payloads.get(event.event_id.as_ref())
                    .and_then(|payload| payload.pointer("/event/event")).and_then(Value::as_str) == Some("completion_reported")) {
                eprintln!("NOMIFUN_INPUT_EVIDENCE started={} successful={successful} commands={}", starts.len(), json!(observations));
                return Err(failure());
            }
            let history = public_history(router, "input.public_result", &session).await?;
            let note = history.iter().find(|message|
                message.pointer("/content/turn_outcome/kind").and_then(Value::as_str) == Some("input_required"))
                .ok_or_else(|| SmokeFailure::new("input.public_result", "INPUT_PUBLIC_RESULT_MISSING", 422))?;
            if note.pointer("/content/user_input_request") != Some(&request)
                || note.pointer("/content/turn_outcome/request_id") != request.get("request_id") {
                return Err(SmokeFailure::new("input.public_result", "INPUT_PUBLIC_RESULT_DIFFERS", 422));
            }
            break;
        }
        if tokio::time::Instant::now() >= deadline { return Err(SmokeFailure::new("input.wait", "INPUT_HANDOFF_DEADLINE", 504)); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    let cursor = session_message_cursor(router, "input.followup_cursor", &session).await?;
    let followup = start_session_turn(router, "input.followup", &session, &uuid::Uuid::now_v7().to_string(),
        "我叫小王，爱好羽毛球。本轮只确认收到资料，不生成文件、不调用工具，精确回复 NOMIFUN_INPUT_FOLLOWUP_OK。".into()).await?;
    if followup == operation { return Err(SmokeFailure::new("input.followup", "INPUT_HANDOFF_REOPENED_OLD_TURN", 422)); }
    wait_for_session_marker(router, "input.followup_reply", &session, cursor,
        "NOMIFUN_INPUT_FOLLOWUP_OK", TURN_RESULT_DEADLINE).await?;
    let history = public_history(router, "input.followup_result", &session).await?;
    let answer = history.iter().find(|message|
        message.pointer("/content/turn_outcome/kind").and_then(Value::as_str) == Some("answered"))
        .ok_or_else(|| SmokeFailure::new("input.followup_result", "INPUT_FOLLOWUP_RESULT_MISSING", 422))?;
    let ids = answer.pointer("/content/turn_outcome/message_ids").and_then(Value::as_array)
        .ok_or_else(|| SmokeFailure::new("input.followup_result", "INPUT_FOLLOWUP_RESULT_INVALID", 422))?;
    if ids.is_empty() || !ids.iter().all(|id| history.iter().any(|message|
        message.get("message_id") == Some(id)
            && message.pointer("/content/content").and_then(Value::as_str)
                .is_some_and(|text| text.contains("NOMIFUN_INPUT_FOLLOWUP_OK")))) {
        return Err(SmokeFailure::new("input.followup_result", "INPUT_FOLLOWUP_ANSWER_NOT_REFERENCED", 422));
    }
    assert_session_binding(router, "input.binding", &session, &binding).await?;
    let events = session_events(router, &session).await?;
    if events.iter().filter(|event| event["kind"] == "turn/completed"
        && event["correlation_id"].as_str() == Some(operation.as_str())
        && event.pointer("/payload/value/user_input_request") == Some(&request)).count() != 1 {
        return Err(SmokeFailure::new("input.history", "INPUT_HANDOFF_HISTORY_CHANGED", 422));
    }
    pool.close().await;
    eprintln!("NOMIFUN_USER_INPUT_HANDOFF_LIVE_OK model={model} commands=2 turns=2 public_results=verified");
    Ok(())
}
