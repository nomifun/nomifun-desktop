//! Independent eight-child acceptance. The two-child case and its proof stay
//! separate; this uses only the ordinary isolated live-provider fixture.
use super::*;
use nomifun_agent_contracts::{AgentSessionId, OperationId};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::{AgentSessionStore, ChatCausalityFacts};
use nomifun_db::sqlx;
use std::collections::BTreeSet;
use std::path::PathBuf;

const MARKER: &str = "NOMIFUN_EIGHT_SUBAGENT_INTEGRATION_OK";
const DIRECTORY: &str = "验收/八子模块";
const INTEGRATION: &str = "integrated-eight.txt";
const COUNT: usize = 8;

const LATEST_TURN_SQL: &str = "SELECT operation_id,state FROM agent_turns WHERE session_id=? ORDER BY rowid DESC LIMIT 1";
const EXECUTIONS_SQL: &str = "SELECT execution_id,status,max_parallel FROM agent_executions WHERE json_extract(initial_plan_input,'$.source.agent_session_id')=?";
const STEPS_SQL: &str = "SELECT step_id,title,spec FROM agent_execution_steps WHERE execution_id=? AND superseded_in_revision IS NULL";
const ATTEMPTS_SQL: &str = "SELECT step.tool_policy,attempt.status,link.conversation_id,attempt.attempt_no,attempt.started_at,attempt.finished_at \
    FROM agent_execution_attempts attempt \
    JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
    LEFT JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id AND link.step_id=attempt.step_id \
        AND link.attempt_id=attempt.attempt_id AND link.relation='attempt' \
    WHERE attempt.execution_id=?";
const DELEGATIONS_SQL: &str = "SELECT COUNT(*) FROM agent_effects WHERE action_id='agent/delegate'";
const TURN_COUNT_SQL: &str = "SELECT COUNT(*) FROM agent_turns WHERE session_id=?";
const PROJECTIONS_SQL: &str = "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/assistant-projected'";
const CHILD_LINKS_SQL: &str = "SELECT link.conversation_id,step.title FROM conversation_execution_links link JOIN agent_execution_steps step ON step.execution_id=link.execution_id AND step.step_id=link.step_id WHERE link.execution_id=? AND link.relation='attempt'";
const CHILD_TURNS_SQL: &str = "SELECT operation_id,state FROM agent_turns WHERE session_id=?";
const PROCESS_COUNT_SQL: &str = "SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.process/start' AND state IN ('pending','unknown','returned')";
const WRITE_SESSIONS_SQL: &str = "SELECT session_id FROM agent_effects WHERE action_id='workspace.files/write' AND state='returned'";

type SettledAttempt = (String, String, String, i64, i64, i64);

async fn read_attempts(pool: &sqlx::SqlitePool, execution: &str) -> Result<Vec<SettledAttempt>, SmokeFailure> {
    // Session identity belongs to the exact existing link, never an Attempt
    // mirror. Queued/interrupted rows legitimately have nullable timestamps;
    // this completed-work acceptance requires their actual interval instead.
    let rows: Vec<(String, String, Option<String>, i64, Option<i64>, Option<i64>)> = sqlx::query_as(ATTEMPTS_SQL)
        .bind(execution).fetch_all(pool).await.map_err(|error| {
            let kind = match error {
                sqlx::Error::Database(_) => "database",
                sqlx::Error::ColumnDecode { .. } => "column_decode",
                sqlx::Error::ColumnNotFound(_) => "column_not_found",
                sqlx::Error::PoolTimedOut => "pool_timeout",
                sqlx::Error::RowNotFound => "row_not_found",
                _ => "other",
            };
            eprintln!("NOMIFUN_SUBAGENT_EIGHT_QUERY_FAILURE query=attempts kind={kind}");
            failed("EIGHT_ATTEMPT_READ_FAILED")
        })?;
    let attempts = rows.into_iter().map(|(policy,state,session,number,start,end)| {
        let session = session.filter(|session| !session.is_empty()).ok_or_else(|| failed("EIGHT_CHILD_SESSION_BINDING_MISSING"))?;
        let (Some(start),Some(end)) = (start,end) else { return Err(failed("EIGHT_ATTEMPT_INTERVAL_MISSING")); };
        Ok((policy,state,session,number,start,end))
    }).collect::<Result<Vec<_>, SmokeFailure>>()?;
    if attempts.len() != COUNT || attempts.iter().any(|(policy,state,_,number,start,end)| policy != "full" || state != "completed" || *number != 0 || end <= start)
        || attempts.iter().map(|(_,_,session,_,_,_)| session).collect::<BTreeSet<_>>().len() != COUNT {
        return Err(failed("EIGHT_CHILD_OR_DUPLICATE_ATTEMPT_INVALID"));
    }
    Ok(attempts)
}

fn failed(code: &'static str) -> SmokeFailure { SmokeFailure::new("subagent.eight", code, 422) }

struct Module { title: String, file: String, content: String, role: String }

fn modules() -> Vec<Module> {
    (1..=COUNT).map(|index| Module {
        title: format!("child-{index:02}"), file: format!("{DIRECTORY}/module-{index:02}.txt"),
        content: format!("MODULE_{index:02}={}", index * 11),
        role: format!("你是 SubAgent-{index:02}，负责设计、实现和运行第{index:02}个独立模块。"),
    }).collect()
}

fn integrated(modules: &[Module]) -> String {
    format!("{};SUM=396", modules.iter().map(|module| module.content.as_str()).collect::<Vec<_>>().join(";"))
}

fn verification_command(expected: &str) -> (String, Value) {
    if cfg!(windows) {
        (PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe")
            .to_string_lossy().into_owned(), json!(["-NoProfile","-NonInteractive","-Command",
                format!("if ((Get-Content -LiteralPath '{INTEGRATION}' -Raw) -ceq '{expected}') {{ exit 0 }} else {{ exit 1 }}")]))
    } else {
        ("/bin/sh".into(), json!(["-c",format!("printf %s '{expected}' | cmp -s - '{INTEGRATION}'")]))
    }
}

fn same_path(work: &Path, supplied: &Value, expected: &str) -> bool {
    let Some(supplied) = supplied.as_str() else { return false; };
    let supplied = if Path::new(supplied).is_absolute() { PathBuf::from(supplied) } else { work.join(supplied) };
    match (nomifun_common::paths::canonicalize_simplified(&supplied),
        nomifun_common::paths::canonicalize_simplified(&work.join(expected))) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    }
}

fn runtime_events(facts: &ChatCausalityFacts, operation: &OperationId) -> Vec<AgentEngineEvent> {
    facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded"
        && event.correlation_id.as_ref() == operation.as_ref())
        .filter_map(|event| facts.event_payloads.get(event.event_id.as_ref())?.get("event"))
        .filter_map(|value| serde_json::from_value(value.clone()).ok()).collect()
}

fn overlap(intervals: &[(i64, i64)], threshold: usize) -> (usize, i64) {
    let mut points = intervals.iter().flat_map(|(start,end)| [(*start,1_i32),(*end,-1_i32)]).collect::<Vec<_>>();
    points.sort_unstable(); // End before start: touching intervals do not overlap.
    let (mut active,mut peak,mut duration,mut previous) = (0_i32,0_usize,0_i64,None);
    for (time,delta) in points {
        if active as usize >= threshold && let Some(previous) = previous { duration += time - previous; }
        active += delta;
        peak = peak.max(active as usize);
        previous = Some(time);
    }
    (peak,duration)
}

fn native_interval(facts: &ChatCausalityFacts, operation: &OperationId) -> Result<(i64,i64), SmokeFailure> {
    let started = facts.events.iter().find(|event| event.kind.0 == "turn/started"
        && event.correlation_id.as_ref() == operation.as_ref()).ok_or_else(|| failed("EIGHT_CHILD_START_MISSING"))?;
    let id = uuid::Uuid::parse_str(started.event_id.as_ref()).map_err(|_| failed("EIGHT_CHILD_START_ID_INVALID"))?;
    let (seconds,nanos) = id.get_timestamp().ok_or_else(|| failed("EIGHT_CHILD_START_TIME_MISSING"))?.to_unix();
    let started = i64::try_from(seconds * 1000 + u64::from(nanos / 1_000_000)).map_err(|_| failed("EIGHT_CHILD_START_TIME_INVALID"))?;
    // agent_turns.finished_at is a canonical sequence index, not a wall clock.
    // Use the exact original terminal Payload, as the Session reader does.
    let terminal = facts.events.iter().find(|event| event.kind.0 == "turn/completed"
        && event.correlation_id.as_ref() == operation.as_ref()).ok_or_else(|| failed("EIGHT_CHILD_TERMINAL_MISSING"))?;
    let finished = facts.event_payloads.get(terminal.event_id.as_ref())
        .and_then(|payload| payload.get("finished_at_ms")).and_then(Value::as_i64)
        .ok_or_else(|| failed("EIGHT_CHILD_FINISH_TIME_MISSING"))?;
    if finished <= started { return Err(failed("EIGHT_CHILD_NATIVE_INTERVAL_INVALID")); }
    Ok((started,finished))
}

pub(super) async fn run(router: &Router, key: &str, model: &str, root: &Path, work: &Path) -> Result<(), SmokeFailure> {
    if model != "step-5-preview" { return Err(failed("EIGHT_MODEL_INVALID")); }
    let modules = modules();
    let expected = integrated(&modules);
    // Fixture setup creates directories only, never any child or parent artifact.
    tokio::fs::create_dir_all(work.join(DIRECTORY)).await.map_err(|_| failed("EIGHT_FIXTURE_DIRECTORY_FAILED"))?;
    let provider = configure_stepfun(router,key,STEPFUN_PLAN_BASE_URL,model).await?;
    let (preset,_) = create_agent_preset(router,&provider,model,&[
        ("agent.collaboration",&["agent/delegate"],""),
        ("workspace.files",&["workspace.files/read","workspace.files/write"],"workspace"),
        ("workspace.process",&["workspace.process/start","workspace.process/poll","workspace.process/cancel"],"process_session"),
    ]).await?;
    let (session,_) = create_session_in_workspace(router,&preset,&provider,model,json!([
        {"resource_kind":"workspace","resource_id":"default-workspace"},
        {"resource_kind":"process_session","resource_id":"managed-process-session"}
    ]),Some(work)).await?;
    let tasks = modules.iter().map(|module| format!(
        "Task name {}: its prompt must begin with the exact role label {} Use write_file to save {} with exactly {} and no trailing newline; read that exact file back and report its saved path and exact contents. Modify only this own file, use native file tools for this task, do not start a process or shell, and do not delegate.",
        module.title,module.role,json!(module.file),json!(module.content))).collect::<Vec<_>>().join("\n");
    let (verify_program,verify_arguments) = verification_command(&expected);
    let prompt = format!("Complete this eight-module development acceptance within this SAME parent Turn. Use exactly ONE agent/delegate strategy=parallel call with all eight independent tasks, all tool_policy=full, no dependencies, and leave the configured parallel limit unchanged (do not pass max_parallel or any concurrency override).\n{tasks}\nYou must not write or modify any child file yourself. After acceptance, use agent_wait mode=all, then agent_status to consume the eight exact canonical results including every result page if present. Do not create another delegation, retry an Attempt, ask for user input, or finish on the acceptance receipt. Read ALL eight original module files yourself, compute the sum of their numbers, and write {INTEGRATION} with the eight exact module strings in child-01 through child-08 order joined by semicolons followed by ;SUM=<the computed sum>, with no newline. Read back that integrated file. Run the exact verification using start_process command={}, args={}, wait_ms=30000. If it is still running, poll its exact process_id until exit_code=0 and cleanup.reaped=true. Only after all eight deliveries, original artifact reads, integration and the real process check may you complete normally. End your final answer with {MARKER}.",json!(verify_program),verify_arguments);
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(root.join("data/nomifun-backend.db")).read_only(true))
        .await.map_err(|_| failed("EIGHT_FACT_READER_FAILED"))?;
    start_session_turn(router,"subagent.eight.turn",&session,&uuid::Uuid::now_v7().to_string(),prompt).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(900);
    let operation = loop {
        let turn: Option<(String,String)> = sqlx::query_as(LATEST_TURN_SQL)
            .bind(&session).fetch_optional(&pool).await.map_err(|_| failed("EIGHT_PARENT_READ_FAILED"))?;
        if let Some((operation,state)) = turn {
            if state == "completed" { break OperationId::from(operation); }
            if matches!(state.as_str(),"failed"|"cancelled"|"interrupted") { return Err(failed("EIGHT_PARENT_DID_NOT_COMPLETE")); }
        }
        if tokio::time::Instant::now() >= deadline { return Err(failed("EIGHT_PARENT_DEADLINE")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    };
    let store = AgentSessionStore::from_pool(pool.clone()).await.map_err(|_| failed("EIGHT_FACT_READER_FAILED"))?;
    let sid = AgentSessionId::from(session.as_str());
    let facts = store.chat_causality_facts(&sid,&operation).await.map_err(|_| failed("EIGHT_PARENT_FACTS_INVALID"))?;
    let receipt = store.read_turn_receipt(&sid,&operation).await.map_err(|_| failed("EIGHT_PARENT_RESULT_INVALID"))?;
    let result = nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).map_err(|_| failed("EIGHT_PARENT_RESULT_INVALID"))?;
    let (final_nonempty,final_marked)=live_subagent::final_result_evidence(COUNT,result.result_text.as_deref(),MARKER);
    let executions: Vec<(String,String,i64)> = sqlx::query_as(EXECUTIONS_SQL)
        .bind(&session).fetch_all(&pool).await.map_err(|_| failed("EIGHT_EXECUTION_READ_FAILED"))?;
    let [(execution,status,max_parallel)] = executions.as_slice() else { return Err(failed("EIGHT_DUPLICATED_EXECUTION")); };
    if status != "completed" || *max_parallel != 4 { return Err(failed("EIGHT_EXECUTION_OR_DEFAULT_CAPACITY_INVALID")); }
    let steps: Vec<(String,String,String)> = sqlx::query_as(STEPS_SQL)
        .bind(execution).fetch_all(&pool).await.map_err(|_| failed("EIGHT_STEP_READ_FAILED"))?;
    if steps.len() != COUNT || steps.iter().map(|(_,title,_)| title.as_str()).collect::<BTreeSet<_>>() != modules.iter().map(|module| module.title.as_str()).collect::<BTreeSet<_>>()
        || steps.iter().any(|(_,title,spec)| modules.iter().find(|module| module.title == *title)
        .is_none_or(|module| !spec.contains(&module.role))) { return Err(failed("EIGHT_STEP_OR_ROLE_INVALID")); }
    let attempts = read_attempts(&pool,execution).await?;
    let events = runtime_events(&facts,&operation);
    let wait = live_subagent::wait_evidence(&events)?;
    let (wait_calls,status_calls)=(wait.wait_calls,wait.status_calls);
    let progress = nomifun_agent_runtime::collaboration_progress_from_events(&events).map_err(|_| failed("EIGHT_CONSUMPTION_REPLAY_INVALID"))?;
    let consumed = progress.results.get(execution).ok_or_else(|| failed("EIGHT_RESULT_CONSUMPTION_MISSING"))?;
    let expected_steps = steps.iter().map(|(id,_,_)| id.clone()).collect::<BTreeSet<_>>();
    if !progress.pending_child_executions.is_empty() || !consumed.all_terminal_observed
        || consumed.source_digests.keys().cloned().collect::<BTreeSet<_>>() != expected_steps
        || consumed.result_pages.values().any(|page| page.contiguous_bytes != page.total_bytes) {
        return Err(failed("EIGHT_RESULTS_NOT_FULLY_CONSUMED"));
    }
    let delegation_count: i64 = sqlx::query_scalar(DELEGATIONS_SQL)
        .fetch_one(&pool).await.map_err(|_| failed("EIGHT_DELEGATION_READ_FAILED"))?;
    if delegation_count != 1 { return Err(failed("EIGHT_REPEATED_DELEGATION")); }
    let parent_turns: i64 = sqlx::query_scalar(TURN_COUNT_SQL)
        .bind(&session).fetch_one(&pool).await.map_err(|_| failed("EIGHT_PARENT_READ_FAILED"))?;
    let projections: i64 = sqlx::query_scalar(PROJECTIONS_SQL)
        .bind(&session).fetch_one(&pool).await.map_err(|_| failed("EIGHT_PARENT_READ_FAILED"))?;
    if parent_turns != 1 || projections != 0 { return Err(failed("EIGHT_PARENT_CONTINUATION_INVALID")); }
    let children: Vec<(String,String)> = sqlx::query_as(CHILD_LINKS_SQL)
        .bind(execution).fetch_all(&pool).await.map_err(|_| failed("EIGHT_CHILD_LINK_READ_FAILED"))?;
    if children.len() != COUNT || children.iter().map(|(id,_)| id).collect::<BTreeSet<_>>().len() != COUNT
        || attempts.iter().map(|(_,_,session,_,_,_)| session).collect::<BTreeSet<_>>() != children.iter().map(|(session,_)| session).collect::<BTreeSet<_>>() {
        return Err(failed("EIGHT_CHILD_SESSION_OWNERS_INVALID"));
    }
    let mut native_intervals = Vec::new();
    for (child,title) in &children {
        let module = modules.iter().find(|module| module.title == *title).ok_or_else(|| failed("EIGHT_CHILD_TITLE_INVALID"))?;
        let turns: Vec<(String,String)> = sqlx::query_as(CHILD_TURNS_SQL)
            .bind(child).fetch_all(&pool).await.map_err(|_| failed("EIGHT_CHILD_TURN_READ_FAILED"))?;
        let [(child_operation,state)] = turns.as_slice() else { return Err(failed("EIGHT_CHILD_TURN_COUNT_INVALID")); };
        if state != "completed" { return Err(failed("EIGHT_CHILD_NATIVE_NOT_COMPLETED")); }
        let child_operation = OperationId::from(child_operation.as_str());
        let child_facts = store.chat_causality_facts(&child.as_str().into(),&child_operation).await.map_err(|_| failed("EIGHT_CHILD_FACTS_INVALID"))?;
        native_intervals.push(native_interval(&child_facts,&child_operation)?);
        if child_facts.event_payloads.values().any(|payload| payload.pointer("/event/rejection/kind").and_then(Value::as_str) == Some("required_delegation")) {
            return Err(failed("EIGHT_ROLE_FORCED_REDELEGATION"));
        }
        let processes: i64 = sqlx::query_scalar(PROCESS_COUNT_SQL)
            .bind(child).fetch_one(&pool).await.map_err(|_| failed("EIGHT_CHILD_PROCESS_READ_FAILED"))?;
        if processes != 0 { return Err(failed("EIGHT_CHILD_USED_OPAQUE_PROCESS")); }
        let writes = live_subagent::successful_calls(&child_facts,&child_operation,&["workspace.files/write"]);
        if writes.is_empty() || writes.iter().any(|(arguments,output)| !same_path(work,&arguments["path"],&module.file) || output["written"] != true)
            || writes.last().is_none_or(|(arguments,_)| arguments["content"] != module.content) {
            return Err(failed("EIGHT_CHILD_DID_NOT_AUTHOR_OWN_MODULE"));
        }
        let reads = live_subagent::successful_calls(&child_facts,&child_operation,&["workspace.files/read"]);
        if !reads.iter().any(|(arguments,output)| same_path(work,&arguments["path"],&module.file) && output["content"] == module.content) {
            return Err(failed("EIGHT_CHILD_DID_NOT_READ_OWN_MODULE"));
        }
        if tokio::fs::read(work.join(&module.file)).await.ok().as_deref() != Some(module.content.as_bytes()) {
            return Err(failed("EIGHT_CHILD_ARTIFACT_BYTES_INVALID"));
        }
    }
    let writes: Vec<String> = sqlx::query_scalar(WRITE_SESSIONS_SQL)
        .fetch_all(&pool).await.map_err(|_| failed("EIGHT_NATIVE_WRITES_READ_FAILED"))?;
    let expected_sessions = children.iter().map(|(id,_)| id.as_str()).chain(std::iter::once(session.as_str())).collect::<BTreeSet<_>>();
    if expected_sessions.len() != COUNT+1 || writes.iter().map(String::as_str).collect::<BTreeSet<_>>() != expected_sessions {
        return Err(failed("EIGHT_NATIVE_WRITE_SOURCE_INVALID"));
    }
    let parent_reads = live_subagent::successful_calls(&facts,&operation,&["workspace.files/read"]);
    if modules.iter().any(|module| !parent_reads.iter().any(|(arguments,output)| same_path(work,&arguments["path"],&module.file) && output["content"] == module.content)) {
        return Err(failed("EIGHT_PARENT_DID_NOT_READ_ALL_ORIGINALS"));
    }
    if !parent_reads.iter().any(|(arguments,output)| same_path(work,&arguments["path"],INTEGRATION) && output["content"] == expected) {
        return Err(failed("EIGHT_PARENT_DID_NOT_READ_OWN_INTEGRATION"));
    }
    let parent_writes = live_subagent::successful_calls(&facts,&operation,&["workspace.files/write"]);
    if parent_writes.is_empty() || parent_writes.iter().any(|(arguments,_)| !same_path(work,&arguments["path"],INTEGRATION))
        || parent_writes.last().is_none_or(|(arguments,_)| arguments["content"] != expected)
        || tokio::fs::read(work.join(INTEGRATION)).await.ok().as_deref() != Some(expected.as_bytes()) {
        return Err(failed("EIGHT_PARENT_REPLACED_CHILD_OR_INTEGRATION_INVALID"));
    }
    let verification_processes = live_subagent::successful_calls(&facts,&operation,&["workspace.process/start"]).into_iter()
        .filter(|(arguments,_)| arguments["command"] == verify_program && arguments["args"] == verify_arguments)
        .filter_map(|(_,output)| output["process_id"].as_str().map(str::to_owned)).collect::<BTreeSet<_>>();
    let parent_processes: i64 = sqlx::query_scalar(PROCESS_COUNT_SQL)
        .bind(&session).fetch_one(&pool).await.map_err(|_| failed("EIGHT_PARENT_PROCESS_READ_FAILED"))?;
    if parent_processes == 0 || parent_processes as usize != verification_processes.len() {
        return Err(failed("EIGHT_PARENT_USED_AN_UNVERIFIED_PROCESS"));
    }
    let verified = live_subagent::successful_calls(&facts,&operation,&["workspace.process/start","workspace.process/poll"]).iter()
        .any(|(_,output)| output["process_id"].as_str().is_some_and(|id| verification_processes.contains(id))
            && output["state"] == "exited" && output["exit_code"] == 0 && output.pointer("/cleanup/reaped") == Some(&json!(true)));
    if !verified { return Err(failed("EIGHT_VERIFICATION_EXIT_OR_REAP_INVALID")); }
    let (native_peak,native_overlap) = overlap(&native_intervals,4);
    let (domain_peak,_) = overlap(&attempts.iter().map(|(_,_,_,_,start,end)| (*start,*end)).collect::<Vec<_>>(),4);
    if native_peak < 4 || native_overlap <= 0 || domain_peak < 4 { return Err(failed("EIGHT_FOUR_NATIVE_CHILDREN_DID_NOT_OVERLAP")); }
    let build = events.iter().find_map(|event| match event { AgentEngineEvent::TurnStarted {binding,..} => Some(binding.build_digest().as_ref()), _ => None })
        .filter(|digest: &&str| digest.len()==64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| failed("EIGHT_NATIVE_BUILD_MISSING"))?;
    let proof = json!({"schema_version":1,"model":model,"build_digest":build,"full_children":COUNT,
        "child_completed":COUNT,"attempts":attempts.len(),"delegations":delegation_count,"configured_max_parallel":max_parallel,
        "native_peak":native_peak,"domain_peak":domain_peak,"four_way_overlap_ms":native_overlap,
        "parent_turns":parent_turns,"native_write_sessions":expected_sessions.len(),"final_projection":projections,
        "exact_results_consumed":consumed.source_digests.len(),"wait_calls":wait_calls,"status_calls":status_calls,
        "waits":wait.waits,"wakes":wait.wakes,"parent_model_steps":wait.parent_model_steps,"wait_model_calls":0,
        "parent_read_originals":COUNT,"parent_authored_child_files":0,"integrated":true,"verification_process":true,"verification_reaped":true});
    pool.close().await;
    // This diagnostic proves the individual workload checks even if the
    // independent final-answer marker assertion below fails. It is not PASS.
    eprintln!("NOMIFUN_SUBAGENT_EIGHT_FACTS {proof}");
    if !final_nonempty {return Err(failed("EIGHT_PARENT_FINAL_EMPTY"));}
    if !final_marked {return Err(failed("EIGHT_PARENT_FINAL_MARKER_MISSING"));}
    eprintln!("NOMIFUN_SUBAGENT_EIGHT_LIVE_OK {proof}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;

    #[tokio::test]
    async fn eight_attempt_evidence_uses_exact_links_and_nullable_intervals_from_the_formal_schema() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let pool = database.pool();
        // These are the actual shared queries used by this live acceptance,
        // prepared against all current official migrations, not replica DDL.
        for (name,query) in [
            ("latest_turn",LATEST_TURN_SQL),("executions",EXECUTIONS_SQL),("steps",STEPS_SQL),
            ("attempts",ATTEMPTS_SQL),("delegations",DELEGATIONS_SQL),("turn_count",TURN_COUNT_SQL),
            ("projections",PROJECTIONS_SQL),("child_links",CHILD_LINKS_SQL),("child_turns",CHILD_TURNS_SQL),
            ("process_count",PROCESS_COUNT_SQL),("write_sessions",WRITE_SESSIONS_SQL),
        ] { assert!(pool.prepare(query).await.is_ok(), "the official schema must prepare the live {name} query"); }
        let old = sqlx::query("SELECT attempt.conversation_id FROM agent_execution_attempts attempt").fetch_all(pool).await;
        assert!(matches!(old,Err(sqlx::Error::Database(_))), "a Session mirror is absent from the official Attempt schema");

        let execution = uuid::Uuid::now_v7().to_string();
        let mut identities = Vec::new();
        for index in 0..COUNT {
            let step = uuid::Uuid::now_v7().to_string();
            let attempt = uuid::Uuid::now_v7().to_string();
            let session = uuid::Uuid::now_v7().to_string();
            sqlx::query("INSERT INTO agent_execution_steps (step_id,execution_id,title,spec,tool_policy,kind,status,introduced_in_revision,created_at,updated_at) VALUES (?, ?, ?, 'fixture', 'full', 'agent', 'completed', 0, 1, 1)")
                .bind(&step).bind(&execution).bind(format!("child-{index:02}")).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO agent_execution_attempts (attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,started_at,finished_at,created_at,updated_at) VALUES (?, ?, ?, 0, 'completed', 'fixture', 1000, 2000, 1, 1)")
                .bind(&attempt).bind(&execution).bind(&step).execute(pool).await.unwrap();
            sqlx::query("INSERT INTO conversation_execution_links (conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES (?, ?, 'attempt', ?, ?, 0, 1, 1)")
                .bind(&session).bind(&execution).bind(&step).bind(&attempt).execute(pool).await.unwrap();
            identities.push((step,attempt,session));
        }
        let rows = read_attempts(pool,&execution).await.unwrap();
        assert_eq!(rows.len(),COUNT);
        assert!(rows.iter().all(|(policy,state,_,number,start,end)| policy == "full" && state == "completed" && *number == 0 && *start == 1000 && *end == 2000));
        assert_eq!(rows.iter().map(|(_,_,session,_,_,_)| session).collect::<BTreeSet<_>>(),
            identities.iter().map(|(_,_,session)| session).collect::<BTreeSet<_>>());

        let (step,attempt,session) = &identities[0];
        // The official nullable fields must fail this completed-work check,
        // never decode as zero or disappear through an inner join/filter.
        for (column,value) in [("started_at",1000_i64),("finished_at",2000_i64)] {
            sqlx::query(&format!("UPDATE agent_execution_attempts SET {column}=NULL WHERE attempt_id=?"))
                .bind(attempt).execute(pool).await.unwrap();
            assert_eq!(read_attempts(pool,&execution).await.unwrap_err().code,"EIGHT_ATTEMPT_INTERVAL_MISSING");
            sqlx::query(&format!("UPDATE agent_execution_attempts SET {column}=? WHERE attempt_id=?"))
                .bind(value).bind(attempt).execute(pool).await.unwrap();
        }
        for (column,original) in [("execution_id",&execution),("step_id",step),("attempt_id",attempt)] {
            sqlx::query(&format!("UPDATE conversation_execution_links SET {column}=? WHERE conversation_id=?"))
                .bind(uuid::Uuid::now_v7().to_string()).bind(session).execute(pool).await.unwrap();
            assert_eq!(read_attempts(pool,&execution).await.unwrap_err().code,"EIGHT_CHILD_SESSION_BINDING_MISSING");
            sqlx::query(&format!("UPDATE conversation_execution_links SET {column}=? WHERE conversation_id=?"))
                .bind(original).bind(session).execute(pool).await.unwrap();
        }
        for (column,invalid,original) in [("attempt_no","1","0"),("status","'failed'","'completed'"),("finished_at","1000","2000")] {
            sqlx::query(&format!("UPDATE agent_execution_attempts SET {column}={invalid} WHERE attempt_id=?"))
                .bind(attempt).execute(pool).await.unwrap();
            assert_eq!(read_attempts(pool,&execution).await.unwrap_err().code,"EIGHT_CHILD_OR_DUPLICATE_ATTEMPT_INVALID");
            sqlx::query(&format!("UPDATE agent_execution_attempts SET {column}={original} WHERE attempt_id=?"))
                .bind(attempt).execute(pool).await.unwrap();
        }
        sqlx::query("UPDATE agent_execution_steps SET tool_policy='read_only' WHERE step_id=?").bind(step).execute(pool).await.unwrap();
        assert_eq!(read_attempts(pool,&execution).await.unwrap_err().code,"EIGHT_CHILD_OR_DUPLICATE_ATTEMPT_INVALID");
        sqlx::query("UPDATE agent_execution_steps SET tool_policy='full' WHERE step_id=?").bind(step).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO conversation_execution_links (conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) SELECT conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at FROM conversation_execution_links WHERE conversation_id=?")
            .bind(session).execute(pool).await.unwrap();
        assert_eq!(read_attempts(pool,&execution).await.unwrap_err().code,"EIGHT_CHILD_OR_DUPLICATE_ATTEMPT_INVALID");
        pool.close().await;
    }

    #[test]
    fn native_concurrency_requires_positive_overlap_and_does_not_count_touching_turns() {
        assert_eq!(overlap(&[(0,10),(10,20),(20,30),(30,40)],4),(1,0));
        assert_eq!(overlap(&[(0,10),(1,11),(2,12),(3,13),(10,20)],4),(4,8));
    }
}
