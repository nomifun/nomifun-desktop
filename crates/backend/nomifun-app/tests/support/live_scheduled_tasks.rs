//! Isolated production Cron -> canonical Session -> real model/tool acceptance.
//! Credentials are owned by the parent smoke fixture and never enter output.
use super::*;

const FILE_MARKER: &str = "NOMIFUN_CRON_WORKBENCH_FILE_OK";
const REPLY_MARKER: &str = "NOMIFUN_CRON_WORKBENCH_REPLY_OK";
const PROCESS_CHECK: &str = "test -f cron-workbench-proof.txt && printf CRON_PROCESS_OK";

fn failed(phase: &'static str, code: &'static str) -> SmokeFailure {
    SmokeFailure::new(phase, code, 422)
}

async fn run_now(router: &Router, job: &str, key: &str) -> Result<String, SmokeFailure> {
    let (status, value)=dispatch_json_with_headers(router,"cron.run_now",Method::POST,
        format!("/api/cron/jobs/{job}/run"),Some(json!({})),TURN_COMMAND_DEADLINE,
        &[("idempotency-key",key)]).await?;
    if status!=StatusCode::OK {return Err(SmokeFailure::http("cron.run_now",status,&value));}
    let value=envelope_data("cron.run_now",value)?;
    required_string("cron.run_now",&value,"/conversation_id","CRON_SESSION_MISSING")
}

async fn completed_job(router: &Router, id: &str, count: u64) -> Result<Value, SmokeFailure> {
    let deadline=tokio::time::Instant::now()+TURN_RESULT_DEADLINE;
    loop {
        let value=successful_json(router,"cron.settlement",Method::GET,
            format!("/api/cron/jobs/{id}"),None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
        let job=envelope_data("cron.settlement",value)?;
        let actual=job.pointer("/state/run_count").and_then(Value::as_u64).unwrap_or(0);
        if actual>=count {
            if job.pointer("/state/last_status").and_then(Value::as_str)!=Some("ok") {
                return Err(failed("cron.settlement","CRON_RUN_NOT_SUCCESSFUL"));
            }
            return Ok(job);
        }
        if tokio::time::Instant::now()>=deadline {
            return Err(failed("cron.settlement","CRON_RUN_DID_NOT_SETTLE"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn tool_succeeded(events: &[Value], action: &str, work_dir: &Path) -> bool {
    let Some(turn)=events.iter().find(|event|event["kind"]=="turn/started") else{return false;};
    if !events.iter().any(|event|event["kind"]=="turn/completed" && event["correlation_id"]==turn["correlation_id"]) {return false;}
    events.iter().filter(|event|event["kind"]=="tool/call-started"
        && event["causation_event_id"]==turn["event_id"]
        && event.pointer("/payload/value/action_id").and_then(Value::as_str)==Some(action))
        .any(|start| {
            let call=start.pointer("/payload/value/call_id").and_then(Value::as_str);
            let Some(proposal)=events.iter().find(|event|event["kind"]=="runtime/progress-recorded"
                && event["correlation_id"]==turn["correlation_id"]
                && event.pointer("/payload/value/event/event").and_then(Value::as_str)==Some("tool_call_completed")
                && event.pointer("/payload/value/event/call/call_id").and_then(Value::as_str)==call) else{return false;};
            let Some(arguments)=proposal.pointer("/payload/value/event/call/arguments") else{return false;};
            match action {
                "workspace.files/write" => {
                    let Some(path)=arguments["path"].as_str() else{return false;};
                    let path=Path::new(path);
                    let resolved=if path.is_absolute(){path.to_owned()}else{work_dir.join(path)};
                    if std::fs::canonicalize(resolved).ok()!=std::fs::canonicalize(work_dir.join("cron-workbench-proof.txt")).ok()
                        || arguments["content"]!=FILE_MARKER {return false;}
                }
                "workspace.process/start" => {
                    if arguments["command"]!="/bin/sh" || arguments["args"]!=json!(["-c",PROCESS_CHECK])
                        || arguments["wait_ms"]!=30000 {return false;}
                }
                _ => return false,
            }
            events.iter().any(|result| {
            if result["kind"]!="tool/result-recorded"
                || result["causation_event_id"]!=start["event_id"]
                || result.pointer("/payload/value/operation_id")!=start.pointer("/payload/value/operation_id")
                || !result.pointer("/payload/value/error").is_none_or(Value::is_null) {return false;}
            let Some(output)=result.pointer("/payload/value/output") else{return false;};
            let Ok(output)=serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(output.clone()) else{return false;};
            if output.is_error || Some(output.call_id.as_ref())!=start.pointer("/payload/value/call_id").and_then(Value::as_str) {return false;}
            if action=="workspace.process/start" {
                let Ok(observation)=serde_json::from_str::<Value>(&output.output_text()) else{return false;};
                return observation["success"]==true && observation["exit_code"]==0
                    && observation.pointer("/cleanup/reaped")==Some(&json!(true));
            }
            true
            })
        })
}

async fn resolved_events(router: &Router, store: &nomifun_agent_session::AgentSessionStore, session: &str) -> Result<Vec<Value>,SmokeFailure> {
    let mut events=session_events(router,session).await?;
    let ids=events.iter().map(|event|event["event_id"].as_str().map(|id|id.to_owned().into()))
        .collect::<Option<Vec<nomifun_agent_contracts::EventId>>>()
        .ok_or_else(||failed("cron.effects","CRON_EVENT_ID_MISSING"))?;
    let payloads=store.resolved_event_payloads(&session.to_owned().into(),&ids).await
        .map_err(|_|failed("cron.effects","CRON_FACT_READER_FAILED"))?;
    for event in &mut events {
        let payload=payloads.get(event["event_id"].as_str().unwrap())
            .ok_or_else(||failed("cron.effects","CRON_EVENT_PAYLOAD_MISSING"))?;
        event["payload"]["value"]=payload.clone();
    }
    Ok(events)
}

pub(super) async fn run(router: &Router, api_key: &str, model: &str, root: &Path, work_dir: &Path) -> Result<(),SmokeFailure> {
    if model!="step-5-preview" {return Err(failed("cron.model","CRON_LIVE_MODEL_INVALID"));}
    let provider=configure_stepfun(router,api_key,STEPFUN_PLAN_BASE_URL,model).await?;
    let before=successful_json(router,"cron.library_before",Method::GET,
        "/api/agent-preset-templates?source=official",None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let before=envelope_data("cron.library_before",before)?;
    let original=before.pointer("/official_presets/assistant.general/current_stable_revision").cloned();
    let proof_path=work_dir.join("cron-workbench-proof.txt");
    let file_arguments=json!({"path":"cron-workbench-proof.txt","content":FILE_MARKER});
    let process_arguments=json!({"command":"/bin/sh","args":["-c",PROCESS_CHECK],"wait_ms":30000});
    let prompt=format!(
        "Use write_file with exactly these JSON arguments: {file_arguments}. Do not add any prefix, suffix or newline to the file content. Do not write the file using a shell. After write_file succeeds, use start_process with exactly these JSON arguments: {process_arguments}, and wait for successful exit. Do not delegate. After both operations are confirmed successful reply with {REPLY_MARKER}."
    );
    let created=successful_json(router,"cron.create",Method::POST,"/api/cron/jobs",
        Some(json!({"name":"Live scheduled workbench acceptance","created_by":"user",
            "schedule":{"kind":"every","every_ms":600000},"message":prompt,
            "execution_mode":"new_conversation","model":{"provider_id":provider,"model":model},
            "workspace":work_dir.to_string_lossy()})),LOCAL_API_DEADLINE,&[StatusCode::CREATED]).await?;
    let job=envelope_data("cron.create",created)?;
    if job.pointer("/metadata/agent_selection/template_key").and_then(Value::as_str)!=Some("assistant.general")
        || job.pointer("/metadata/model/model").and_then(Value::as_str)!=Some(model)
        || job.pointer("/metadata/model/provider_id").and_then(Value::as_str)!=Some(&provider) {
        return Err(failed("cron.create","CRON_DEFAULT_OR_MODEL_MISMATCH"));
    }
    let binding=job.pointer("/metadata/agent_binding").ok_or_else(||failed("cron.create","CRON_BINDING_MISSING"))?;
    let resources=binding["typed_resource_bindings"].as_array().ok_or_else(||failed("cron.create","CRON_RESOURCES_MISSING"))?;
    for kind in ["workspace","process_session"] {
        if !resources.iter().any(|resource|resource["resource_kind"]==kind
            && resource["operations"].as_array().is_some_and(|operations|!operations.is_empty())) {
            return Err(failed("cron.create","CRON_RESOURCE_GRANT_MISSING"));
        }
    }
    let id=required_string("cron.create",&job,"/cron_job_id","CRON_ID_MISSING")?;
    let key=uuid::Uuid::now_v7().to_string();
    let session=run_now(router,&id,&key).await?;
    completed_job(router,&id,1).await?;
    let proof=tokio::fs::read_to_string(&proof_path).await.map_err(|_|failed("cron.file","CRON_FILE_MISSING"))?;
    if proof!=FILE_MARKER {
        eprintln!("NOMIFUN_LIVE_CRON_FILE_SHAPE bytes={} marker_without_lf={} lf_suffix={} literal_escape_suffix={}",
            proof.len(),proof==FILE_MARKER,proof.bytes().rev().take_while(|byte|*byte==b'\n').count(),proof.ends_with("\\n"));
        return Err(failed("cron.file","CRON_FILE_CONTENT_MISMATCH"));
    }
    // Read the same canonical facts as runtime consumers, including large
    // tool outputs stored by reference. Public /events preserves raw storage.
    let pool=nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new()
            .filename(root.join("data/nomifun-backend.db")).read_only(true)).await
        .map_err(|_|failed("cron.effects","CRON_FACT_READER_FAILED"))?;
    let store=nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await
        .map_err(|_|failed("cron.effects","CRON_FACT_READER_FAILED"))?;
    let events=resolved_events(router,&store,&session).await?;
    if !tool_succeeded(&events,"workspace.files/write",work_dir) || !tool_succeeded(&events,"workspace.process/start",work_dir)
        || events.iter().any(|event|event["kind"]=="turn/failed") {
        return Err(failed("cron.effects","CRON_CANONICAL_TOOL_PROOF_MISSING"));
    }
    let model_starts=events.iter().filter(|event|event.pointer("/payload/value/event/event")
        .and_then(Value::as_str)==Some("model_step_started")).count();
    if model_starts==0 {return Err(failed("cron.effects","CRON_MODEL_REQUEST_NOT_PROVEN"));}
    if run_now(router,&id,&key).await?!=session {
        return Err(failed("cron.replay","CRON_REPLAY_CHANGED_SESSION"));
    }
    let replay=completed_job(router,&id,1).await?;
    let after_replay=resolved_events(router,&store,&session).await?;
    if replay.pointer("/state/run_count").and_then(Value::as_u64)!=Some(1)
        || after_replay.iter().filter(|event|event.pointer("/payload/value/event/event")
            .and_then(Value::as_str)==Some("model_step_started")).count()!=model_starts {
        return Err(failed("cron.replay","CRON_REPLAY_REDISPATCHED"));
    }
    let projection=successful_json(router,"cron.model_projection",Method::GET,
        format!("/api/agent-sessions/{session}/projection"),None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let projection=envelope_data("cron.model_projection",projection)?;
    if projection.pointer("/model/model").and_then(Value::as_str)!=Some(model)
        || projection.pointer("/model/provider_id").and_then(Value::as_str)!=Some(&provider) {
        return Err(failed("cron.model_projection","CRON_EXECUTED_MODEL_MISMATCH"));
    }

    let continuing=successful_json(router,"cron.continuing_create",Method::POST,"/api/cron/jobs",
        Some(json!({"name":"Live continuing scheduled acceptance","created_by":"user",
            "schedule":{"kind":"at","at_ms":nomifun_common::now_ms()+2000},"execution_mode":"existing",
            "model":{"provider_id":provider,"model":model},
            "message":"Do not call tools or delegate. Reply with NOMIFUN_CRON_CONTINUING_OK."})),
        LOCAL_API_DEADLINE,&[StatusCode::CREATED]).await?;
    let continuing=envelope_data("cron.continuing_create",continuing)?;
    let continuing_id=required_string("cron.continuing_create",&continuing,"/cron_job_id","CRON_ID_MISSING")?;
    let once=completed_job(router,&continuing_id,1).await?;
    let first=required_string("cron.continuing",&once,"/metadata/conversation_id","CRON_CONTINUING_ANCHOR_MISSING")?;
    let trigger:String=nomifun_db::sqlx::query_scalar("SELECT trigger_kind FROM cron_run_reservations WHERE cron_job_id=? ORDER BY id LIMIT 1")
        .bind(&continuing_id).fetch_one(&pool).await.map_err(|_|failed("cron.timer","CRON_TRIGGER_READER_FAILED"))?;
    if trigger!="scheduled" || once["enabled"]!=false
        || once.pointer("/state/next_run_at_ms").is_some_and(|value|!value.is_null()) {
        return Err(failed("cron.timer","CRON_ONESHOT_TIMER_NOT_PROVEN"));
    }
    let second=run_now(router,&continuing_id,&uuid::Uuid::now_v7().to_string()).await?;
    completed_job(router,&continuing_id,2).await?;
    if first!=second {return Err(failed("cron.continuing","CRON_CONTINUING_SESSION_CHANGED"));}
    let after=successful_json(router,"cron.library_after",Method::GET,
        "/api/agent-preset-templates?source=official",None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let after=envelope_data("cron.library_after",after)?;
    if after.pointer("/official_presets/assistant.general/current_stable_revision").cloned()!=original {
        return Err(failed("cron.library_after","CRON_CHANGED_AUTHOR_DEFAULT"));
    }
    drop(store);
    pool.close().await;
    eprintln!("NOMIFUN_LIVE_CRON_EVIDENCE {{\"model\":\"step-5-preview\",\"default_general\":true,\"file_effect\":true,\"process_effect\":true,\"idempotent_replay\":true,\"scheduled_timer_triggered\":true,\"continuing_runs\":2,\"continuing_same_session\":true,\"author_default_unchanged\":true}}");
    Ok(())
}
