//! Real App/API coverage of the AutoWork source and delegated-task boundary.
use super::*;
use nomifun_agent_execution::{
    AgentExecutionAutomationPort, AutomationExecutionRequest, AutomationExecutionSource,
};
use nomifun_common::{
    AgentExecutionActor, AgentExecutionEventKind, AgentStepMode, AgentToolPolicy,
    ExecutionStepKind, ExecutionStepStatus, StepFailurePolicy,
};
use nomifun_db::{
    AppendAgentExecutionStepsFromAttemptParams, CreateAgentExecutionAttemptParams,
    IAgentExecutionRepository, IRequirementRepository, NewAgentExecutionEvent,
    NewAgentExecutionStep, RetryAgentExecutionStep,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct OriginalModel {
    url: String,
    calls: Arc<AtomicUsize>,
    finish: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
}

impl OriginalModel {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let finish = Arc::new(tokio::sync::Semaphore::new(0));
        let observed = calls.clone();
        let release = finish.clone();
        let routes = axum::Router::new()
            .route(
                "/v1/models",
                axum::routing::get(|| async {
                    axum::Json(json!({"object":"list","data":[{"id":"activity-fixture","object":"model"}]}))
                }),
            )
            .route(
                "/v1/chat/completions",
                axum::routing::post(move |axum::Json(_request): axum::Json<Value>| {
                    let calls = observed.clone();
                    let finish = release.clone();
                    async move {
                        let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        finish.acquire().await.unwrap().forget();
                        let content = if call == 1 {
                            "The original AutoWork root delivered its preserved business result.".to_owned()
                        } else {
                            format!("Delegated child {call} delivered a different result.")
                        };
                        let delta = json!({"id":"original-autowork-result","object":"chat.completion.chunk","created":1,
                            "model":"activity-fixture","choices":[{"index":0,"delta":{"role":"assistant",
                            "content":content},"finish_reason":null}]});
                        let done = json!({"id":"original-autowork-result","object":"chat.completion.chunk","created":1,
                            "model":"activity-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                        (
                            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                            format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                        )
                    }
                }),
            );
        let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Self { url, calls, finish, server }
    }

    async fn wait_for_original(&self) {
        self.wait_for_calls(1).await;
    }

    async fn wait_for_calls(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.calls.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the exact expected native Automation model request must be admitted");
        assert_eq!(self.calls.load(Ordering::SeqCst), expected);
    }

    async fn close(self) {
        self.server.abort();
        let _ = self.server.await;
    }
}

fn event(kind: AgentExecutionEventKind) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent {
        event_type: kind,
        step_id: None,
        attempt_id: None,
        actor: AgentExecutionActor::system(),
        payload: "{}".into(),
    }
}

fn delegated_step(participant: &str, root_title: &str) -> NewAgentExecutionStep {
    NewAgentExecutionStep {
        step_id: uuid::Uuid::now_v7().to_string(),
        // Deliberately duplicate the root's historical presentation hints:
        // neither the title nor role establishes AutoWork source ownership.
        title: root_title.into(),
        spec: "Independent delegated work; retain the original source and sibling.".into(),
        role: Some("requirement_owner".into()),
        tool_policy: AgentToolPolicy::ReadOnly,
        kind: ExecutionStepKind::Agent,
        agent_mode: Some(AgentStepMode::Normal),
        profile: None,
        fanout_group: None,
        control_policy: None,
        status: ExecutionStepStatus::Pending,
        assigned_participant_id: Some(participant.into()),
        assignment_score: None,
        assignment_rationale: None,
        assignment_source: None,
        assignment_locked: false,
        failure_policy: StepFailurePolicy::FailExecution,
        preset_prompt: None,
        graph_x: None,
        graph_y: None,
    }
}

async fn response(router: &axum::Router, method: &str, path: &str, body: Value) -> (axum::http::StatusCode, Value) {
    let response = router.clone().oneshot(
        axum::http::Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string())).unwrap(),
    ).await.unwrap();
    let status = response.status();
    let body = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    (status, body)
}

async fn assert_claim_preserved(repository: &nomifun_db::SqliteRequirementRepository, original: &nomifun_db::models::RequirementRow) {
    let current = repository.get_by_requirement_id(&original.requirement_id).await.unwrap().unwrap();
    assert_eq!(current.status, original.status, "local task controls do not resolve the AutoWork claim");
    assert_eq!(current.claim_generation, original.claim_generation);
    assert_eq!(current.attempt_count, original.attempt_count);
    assert_eq!(current.claim_token, original.claim_token);
    assert_eq!(current.owner_conversation_id, original.owner_conversation_id);
}

fn task<'a>(detail: &'a Value, id: &str) -> &'a Value {
    detail["steps"].as_array().unwrap().iter().find(|step| step["step_id"] == id).unwrap()
}

fn activity<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["items"].as_array().unwrap().iter().find(|row| row["target"]["step_id"] == id).unwrap()
}

fn assert_same_workspace(actual:&str,expected:&str) {
    assert!(nomifun_common::paths::paths_equivalent(std::path::Path::new(actual),std::path::Path::new(expected)),
        "the frozen physical workspace must remain the same: {actual:?} vs {expected:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn autowork_local_task_stop_preserves_source_root_and_sibling_and_old_attempt_cannot_retarget() {
    let model = OriginalModel::start().await;
    let root = tempfile::tempdir().unwrap();
    let (services, router, state, lead, principal) = fixture_in(root.path(), &model.url).await;
    let owner = &principal.principal_id;
    let engine = state.session_owner.execution_progress.get().and_then(std::sync::Weak::upgrade).unwrap();
    services.requirement_service.create(nomifun_api_types::CreateRequirementRequest {
        title: "Original source".into(), content: "Keep the original root business result.".into(),
        tag: "activity-autowork-scope".into(), order_key: None, status: None,
        created_by: None, attachments: vec![],
    }).await.unwrap();
    let requirements = nomifun_db::SqliteRequirementRepository::new(services.database.pool().clone());
    let claim = requirements.claim_next_for_runner("activity-autowork-scope", Some(lead.as_ref()), None,
        60_000, nomifun_common::now_ms()).await.unwrap().unwrap().row;
    // The runner's original source key includes the durable opaque claim
    // capability. Use that exact contract so CurrentRunStop reaches this root.
    let source_scope = format!("nomifun-autowork-agent-execution-v1\0{}\0{}\0{}",
        claim.requirement_id, claim.claim_generation, claim.claim_token.as_deref().unwrap());
    let source = AutomationExecutionSource {
        requirement_id: claim.requirement_id.clone(), claim_generation: claim.claim_generation,
        operation_id: format!("autowork:{}", nomifun_auth::token_sha256_hex(&source_scope)),
    };
    let admitted = engine.admit_automation(owner, AutomationExecutionRequest {
        source, lead_session_id: lead.as_ref().to_owned(),
        goal: "Return the original root business result once.".into(), workspace: None,
    }).await.unwrap();
    model.wait_for_original().await;
    let execution = &admitted.execution_id;
    let repository = nomifun_db::SqliteAgentExecutionRepository::new(services.database.pool().clone());
    let original = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    assert_eq!(original.execution.max_parallel, 1);
    assert_eq!(original.attempts.len(), 1);
    let original_attempt = &original.attempts[0];
    assert_eq!(original_attempt.conversation_id.as_deref(), Some(lead.as_ref()));
    let root_id = &original_attempt.attempt.step_id;
    let root_step = original.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    let participant = root_step.assigned_participant_id.as_deref().unwrap();
    let children = vec![delegated_step(participant, &root_step.title), delegated_step(participant, &root_step.title)];
    let child_ids = [children[0].step_id.clone(), children[1].step_id.clone()];
    let appended = repository.append_steps_from_attempt(owner, execution, &AppendAgentExecutionStepsFromAttemptParams {
        operation_id: "activity-scope-original-delegation".into(), caller_conversation_id: lead.as_ref().to_owned(),
        caller_step_id: root_id.clone(), caller_attempt_id: original_attempt.attempt.attempt_id.clone(),
        expected_caller_step_version: root_step.version,
        expected_caller_attempt_version: original_attempt.attempt.version,
        new_steps: children, new_dependencies: vec![],
    }, &NewAgentExecutionEvent {
        actor: AgentExecutionActor::agent(lead.as_ref().to_owned(), Some(original_attempt.attempt.attempt_id.clone())),
        ..event(AgentExecutionEventKind::PlanChanged)
    }).await.unwrap();
    assert_eq!(appended.added_step_ids, child_ids.to_vec());
    assert!(appended.detail.dependencies.is_empty(), "delegation ownership is independent of DAG edges");
    let cursor = state.session_owner.canonical.store().current_cursor(&lead).await.unwrap();
    let inventory = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    let root_row = inventory["items"].as_array().unwrap().iter().find(|row|
        row["target"]["kind"] == "auto_work_run" && row["target"]["requirement_id"] == claim.requirement_id
        && row["delegation_target"]["step_id"] == *root_id).unwrap();
    let original_owner_target = root_row["target"].clone();
    let original_attempt_target = root_row["delegation_target"].clone();
    assert_eq!(original_owner_target["target_id"], lead.as_ref());
    assert_eq!(original_attempt_target["kind"], "execution_task");
    assert_eq!(original_attempt_target["attempt_id"], original_attempt.attempt.attempt_id);
    for id in &child_ids {
        let row = activity(&inventory, id);
        assert_eq!(row["kind"], "delegated_task");
        assert!(matches!(row["target"]["kind"].as_str(), Some("execution_task" | "execution_step")));
        assert!(row.get("delegation_target").is_none_or(Value::is_null));
    }
    assert_eq!(state.session_owner.canonical.store().current_cursor(&lead).await.unwrap(), cursor,
        "Activity inventory and delegation targets are read-only");
    let child_target = activity(&inventory, &child_ids[0])["target"].clone();
    request(&router, "POST", "/api/agent-activities/stop", json!({"target":child_target,
        "scope":"task","idempotency_key":"stop-one-autowork-child"})).await;
    let detail_path = format!("/api/agent-executions/{execution}");
    let after_child = request(&router, "GET", &detail_path, Value::Null).await;
    assert_eq!(task(&after_child, &child_ids[0])["status"], "cancelled");
    assert_eq!(task(&after_child, &child_ids[1])["status"], "pending");
    assert_eq!(task(&after_child, root_id)["status"], "running");
    assert_eq!(after_child["execution"]["status"], "running");
    assert_eq!(after_child["execution"]["max_parallel"], 1);
    assert_eq!(after_child["automation_control"]["root_step_id"], *root_id);
    assert_eq!(after_child["automation_control"]["lead_conversation_id"], lead.as_ref());
    assert_eq!(after_child["automation_control"]["requirement_id"], claim.requirement_id);
    assert_eq!(after_child["automation_control"]["claim_generation"], claim.claim_generation);
    assert_claim_preserved(&requirements, &claim).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    let cancel_root_path = format!("{detail_path}/steps/{root_id}/cancel");
    for scope in ["task", "task_and_delegated"] {
        let (status, body) = response(&router, "POST", &cancel_root_path, json!({
            "expected_execution_version":after_child["execution"]["version"],
            "expected_step_version":task(&after_child, root_id)["version"],"scope":scope,
        })).await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "root business Stop must use its Requirement owner: {body}");
        assert_claim_preserved(&requirements, &claim).await;
    }
    let after_delegated = request(&router, "POST", &cancel_root_path, json!({
        "expected_execution_version":after_child["execution"]["version"],
        "expected_step_version":task(&after_child, root_id)["version"],"scope":"delegated_work",
    })).await;
    assert_eq!(task(&after_delegated, root_id)["status"], "running");
    assert_eq!(task(&after_delegated, &child_ids[1])["status"], "cancelled");
    assert_eq!(after_delegated["execution"]["status"], "running");
    assert_claim_preserved(&requirements, &claim).await;
    assert_eq!(state.session_owner.canonical.store().head(&lead).await.unwrap().active_turn_id.as_deref(),
        Some(format!("turn:user:{owner}:{}:{}:initial-turn", lead.as_ref(), original_attempt.attempt.attempt_id).as_str()));
    model.finish.add_permits(1);
    let completed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let detail = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
            let root = detail.steps.iter().find(|step| &step.step_id == root_id).unwrap();
            let carrier = services.agent_runtime_sessions.observe_runtime_carrier(lead.as_ref());
            if root.status == "completed" && detail.execution.lease_owner.is_none()
                && matches!(detail.execution.status.as_str(), "completed" | "completed_with_failures" | "failed")
                && !carrier.registered && !carrier.teardown_pending {
                break detail;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("delegated-work Stop preserves the actual root model result and native retirement");
    let completed_attempt = completed.attempts.iter().find(|attempt|
        attempt.attempt.attempt_id == original_attempt.attempt.attempt_id).unwrap();
    assert_eq!(completed_attempt.attempt.status, "completed");
    assert!(completed_attempt.attempt.output_summary.as_deref().unwrap_or_default().contains("preserved business result"));
    assert_ne!(completed.execution.status, "cancelled");
    assert_claim_preserved(&requirements, &claim).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);

    // Use the existing internal retry writers to stage a legitimate queued
    // successor without starting a second model call or changing concurrency.
    let completed_root = completed.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    let reset = tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let current=repository.get_execution_detail(owner,execution).await.unwrap().unwrap();
            let root=current.steps.iter().find(|step|&step.step_id==root_id).unwrap();
            assert_eq!(root.status,"completed");
            assert_eq!(root.version,completed_root.version,"cleanup may advance only unrelated execution observations");
            let target=repository.get_step_detail(owner,execution,root_id).await.unwrap().unwrap();
            assert_eq!(target.current_attempt.as_ref().unwrap().attempt.attempt_id,original_attempt.attempt.attempt_id,
                "the fixture retry must retain the exact original root until its own writer commits");
            match repository.reset_steps_for_retry(owner,execution,current.execution.version,
                &[RetryAgentExecutionStep {step_id:root_id.clone(),expected_step_version:root.version}],
                &event(AgentExecutionEventKind::StepChanged)).await {
                Ok(reset)=>break reset,
                Err(nomifun_db::DbError::Conflict(error))=>{
                    let after=repository.get_execution(owner,execution).await.unwrap().unwrap();
                    assert!(after.version>current.execution.version,"a same-version conflict is not cleanup progress: {error}");
                },
                Err(error)=>panic!("staging the exact successor failed: {error}"),
            }
        }
    }).await.expect("the original completed root can stage one queued successor while cleanup finishes");
    let pending = reset.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    let successor = repository.create_attempt(owner, execution, root_id, pending.version, None,
        &CreateAgentExecutionAttemptParams { participant_id: pending.assigned_participant_id.clone(),
            start_immediately: false, trigger_reason: "manual_retry".into(), effective_config: "{}".into(),
            retry_after: None, runtime_state: None }, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let successor_attempt = &successor.current_attempt.as_ref().unwrap().attempt;
    assert_ne!(successor_attempt.attempt_id, original_attempt.attempt.attempt_id);
    let (stale_status, stale_body) = response(&router, "POST", "/api/agent-activities/stop", json!({
        "target":original_attempt_target,"scope":"delegated_work","idempotency_key":"old-autowork-attempt-view",
    })).await;
    assert_eq!(stale_status, axum::http::StatusCode::CONFLICT, "a captured old Attempt must not retarget its successor: {stale_body}");
    let untouched = repository.get_step_detail(owner, execution, root_id).await.unwrap().unwrap();
    assert_eq!(untouched.current_attempt.as_ref().unwrap().attempt.attempt_id, successor_attempt.attempt_id);
    assert_eq!(untouched.current_attempt.as_ref().unwrap().attempt.status, "queued");
    assert_claim_preserved(&requirements, &claim).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);

    // The original primary target remains the exact Requirement CurrentRunStop
    // owner even when this source's root has acquired a later Attempt.
    let stopped = request(&router, "POST", "/api/agent-activities/stop", json!({
        "target":original_owner_target,"idempotency_key":"stop-original-autowork-source",
    })).await;
    assert_eq!(stopped["accepted"], true);
    assert_eq!(stopped["execution_stopped"], true,
        "the source fence alone is not confirmation that its exact Execution cancellation finished: {stopped}");
    let cancelled_claim = requirements.get_by_requirement_id(&claim.requirement_id).await.unwrap().unwrap();
    assert_eq!(cancelled_claim.status, "cancelled");
    assert_eq!(cancelled_claim.claim_generation, claim.claim_generation);
    assert_eq!(cancelled_claim.attempt_count, claim.attempt_count);
    let final_detail = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    assert_eq!(final_detail.execution.status, "cancelled");
    let stopped_successor = final_detail.attempts.iter().find(|attempt|
        attempt.attempt.attempt_id == successor_attempt.attempt_id).unwrap();
    assert_eq!(stopped_successor.attempt.status, "cancelled");
    assert!(stopped_successor.attempt.started_at.is_none(),
        "the durable source Stop cannot invoke the queued successor while its audit CAS catches up");
    assert_eq!(final_detail.attempts.len(), 2);
    assert_eq!(final_detail.attempts.iter().find(|attempt|
        attempt.attempt.attempt_id == original_attempt.attempt.attempt_id).unwrap().attempt.status, "completed");
    let events = repository.list_events(owner, execution, 0, 100).await.unwrap();
    let cancellations = events.iter().filter(|event|
        event.event_type == AgentExecutionEventKind::StatusChanged.as_str()
            && serde_json::from_str::<Value>(&event.payload).unwrap()["status"] == "cancelled"
    ).collect::<Vec<_>>();
    assert_eq!(cancellations.len(), 1, "the original source control must commit one explicit aggregate cancellation");
    assert_eq!(cancellations[0].actor_type, "system");
    assert!(cancellations[0].step_id.is_none() && cancellations[0].attempt_id.is_none());
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert!(state.session_owner.canonical.store().get_live_session(&lead).await.is_ok());
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
    model.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn autowork_running_child_stop_keeps_its_independent_session_root_result_and_original_authority() {
    use nomifun_agent_execution::AgentExecutionSessionPort;

    let model = OriginalModel::start().await;
    let temporary = tempfile::tempdir().unwrap();
    let (services, router, state, initial, principal) = fixture_in(temporary.path(), &model.url).await;
    let owner = &principal.principal_id;
    let engine = state.session_owner.execution_progress.get().and_then(std::sync::Weak::upgrade).unwrap();
    let initial = state.session_owner.get_session(owner, initial.as_ref()).await.unwrap();
    let selected = initial.model.as_ref().unwrap();
    let model_ref = json!({"provider_id":selected.provider_id,"model":selected.model});
    let preset = request(&router, "POST", "/api/agent-presets/from-template/coding.codex",
        json!({"reuse_existing":false,"display_name":"Real AutoWork delegation","model":model_ref})).await;
    let created = request(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":preset["preset"]["preset_id"],"model":model_ref,"title":"Original AutoWork lead",
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
    })).await;
    let lead = AgentSessionId::from(created["agent_session_id"].as_str().unwrap());
    let lead_projection = state.session_owner.get_session(owner, lead.as_ref()).await.unwrap();
    let original_snapshot = lead_projection.agent_snapshot.as_ref().unwrap();
    assert!(!original_snapshot.enabled_capability_actions.is_empty(), "the inherited grants must be real");
    assert!(original_snapshot.canonical_binding.as_ref().unwrap().typed_resource_bindings.iter()
        .any(|binding| binding.resource_kind == "workspace"));
    let workspace = lead_projection.extra["workspace"].as_str().unwrap().to_owned();
    let preserved_file = std::path::PathBuf::from(&workspace).join("original-lead-user-data.txt");
    std::fs::write(&preserved_file, "Retain the original user workspace while children retire.").unwrap();
    services.requirement_service.create(nomifun_api_types::CreateRequirementRequest {
        title: "Real delegated AutoWork".into(), content: "Preserve the root while stopping one real delegate.".into(),
        tag: "activity-autowork-dispatch".into(), order_key: None, status: None,
        created_by: None, attachments: vec![],
    }).await.unwrap();
    let requirements = nomifun_db::SqliteRequirementRepository::new(services.database.pool().clone());
    let claim = requirements.claim_next_for_runner("activity-autowork-dispatch", Some(lead.as_ref()), None,
        60_000, nomifun_common::now_ms()).await.unwrap().unwrap().row;
    let source_scope = format!("nomifun-autowork-agent-execution-v1\0{}\0{}\0{}",
        claim.requirement_id, claim.claim_generation, claim.claim_token.as_deref().unwrap());
    let source = AutomationExecutionSource {
        requirement_id: claim.requirement_id.clone(), claim_generation: claim.claim_generation,
        operation_id: format!("autowork:{}", nomifun_auth::token_sha256_hex(&source_scope)),
    };
    let admitted = engine.admit_automation(owner, AutomationExecutionRequest {
        source, lead_session_id: lead.as_ref().to_owned(),
        goal: "Deliver the original root result and authorize two independent delegates.".into(),
        workspace: Some(workspace.clone()),
    }).await.unwrap();
    model.wait_for_original().await;
    let execution = &admitted.execution_id;
    let repository = nomifun_db::SqliteAgentExecutionRepository::new(services.database.pool().clone());
    let original = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    assert_eq!(original.execution.max_parallel, 1);
    assert_same_workspace(original.execution.work_dir.as_deref().unwrap(),&workspace);
    assert_eq!(original.attempts.len(), 1);
    let original_attempt = &original.attempts[0].attempt;
    let root_id = &original_attempt.step_id;
    let root_step = original.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    assert_eq!(root_step.tool_policy, "full");
    let participant = root_step.assigned_participant_id.as_deref().unwrap();
    let mut children = vec![delegated_step(participant, &root_step.title), delegated_step(participant, &root_step.title)];
    for child in &mut children {
        child.tool_policy = AgentToolPolicy::Full;
    }
    let child_ids = [children[0].step_id.clone(), children[1].step_id.clone()];
    repository.append_steps_from_attempt(owner, execution, &AppendAgentExecutionStepsFromAttemptParams {
        operation_id: "real-autowork-children".into(), caller_conversation_id: lead.as_ref().to_owned(),
        caller_step_id: root_id.clone(), caller_attempt_id: original_attempt.attempt_id.clone(),
        expected_caller_step_version: root_step.version, expected_caller_attempt_version: original_attempt.version,
        new_steps: children, new_dependencies: vec![],
    }, &NewAgentExecutionEvent {
        actor: AgentExecutionActor::agent(lead.as_ref().to_owned(), Some(original_attempt.attempt_id.clone())),
        ..event(AgentExecutionEventKind::PlanChanged)
    }).await.unwrap();
    let root_operation = state.session_owner.canonical.store().head(&lead).await.unwrap().active_turn_id.unwrap();
    model.finish.add_permits(1);
    model.wait_for_calls(2).await;

    let dispatched = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    let completed_root = dispatched.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    assert_eq!(completed_root.status, "completed");
    assert_eq!(dispatched.execution.status, "running");
    assert_eq!(dispatched.execution.max_parallel, 1);
    let running = dispatched.attempts.iter().find(|attempt|
        child_ids.contains(&attempt.attempt.step_id) && attempt.attempt.status == "running").unwrap();
    let child_id = running.attempt.step_id.clone();
    let child_attempt = running.attempt.attempt_id.clone();
    let child_session = AgentSessionId::from(running.conversation_id.as_deref().unwrap());
    let sibling_id = child_ids.iter().find(|id| **id != child_id).unwrap();
    assert_ne!(child_session.as_ref(), lead.as_ref(), "a delegate never borrows the source's root Session");
    assert_eq!(dispatched.steps.iter().find(|step| &step.step_id == sibling_id).unwrap().status, "pending");
    assert!(dispatched.attempts.iter().all(|attempt|
        &attempt.attempt.step_id != sibling_id), "the user's one existing slot admits one child at a time");
    let links = repository.list_conversation_links(owner, execution).await.unwrap();
    let child_link = links.iter().find(|link| link.attempt_id.as_deref() == Some(child_attempt.as_str())).unwrap();
    assert_eq!(child_link.relation, "attempt");
    assert_eq!(child_link.conversation_id, child_session.as_ref());
    let original_link = links.iter().find(|link| link.attempt_id.as_deref() == Some(original_attempt.attempt_id.as_str())).unwrap();
    assert_eq!(original_link.relation, "automation");
    assert_eq!(original_link.conversation_id, lead.as_ref());
    // The root's task entry must survive its real cleanup acknowledgement;
    // an inactive cleanup link is not a permanent control pin.
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let links = repository.list_conversation_links(owner, execution).await.unwrap();
            if links.iter().any(|link| link.attempt_id.as_deref() == Some(original_attempt.attempt_id.as_str())
                && !link.active && link.cleanup_completed_at.is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("wait read-only for the exact root cleanup ACK before discovering its retained control");
    // get_session re-admits the child's saved immutable artifacts, including
    // the real resource definitions; its changed binding IDs are legitimate.
    let child_projection = state.session_owner.get_session(owner, child_session.as_ref()).await.unwrap();
    assert_eq!(child_projection.linked_execution_id.as_deref(), Some(execution.as_str()));
    assert_eq!(child_projection.execution_step_id.as_deref(), Some(child_id.as_str()));
    assert_eq!(child_projection.execution_attempt_id.as_deref(), Some(child_attempt.as_str()));
    let current_lead = state.session_owner.get_session(owner, lead.as_ref()).await.unwrap();
    assert!(current_lead.linked_execution_id.is_none());
    assert!(current_lead.execution_step_id.is_none());
    assert!(current_lead.execution_attempt_id.is_none());
    let mut inherited = child_projection.agent_snapshot.clone().unwrap();
    let inherited_binding = inherited.canonical_binding.take().unwrap();
    let mut source_snapshot = original_snapshot.clone();
    let source_binding = source_snapshot.canonical_binding.take().unwrap();
    inherited.preset_name = original_snapshot.preset_name.clone();
    assert_eq!(inherited, source_snapshot, "child identity, model, instructions, skills and capability grants remain exact");
    assert_eq!(inherited_binding.preset_revision_ref, source_binding.preset_revision_ref);
    assert_eq!(inherited_binding.resolved_snapshot_ref, source_binding.resolved_snapshot_ref);
    assert_eq!(inherited_binding.typed_resource_bindings.len(), source_binding.typed_resource_bindings.len());
    for resource in &inherited_binding.typed_resource_bindings {
        let workspace_bearing = matches!(resource.resource_kind.as_str(), "workspace" | "process_session" | "project_memory");
        let original_resource = source_binding.typed_resource_bindings.iter().find(|original|
            original.resource_kind == resource.resource_kind
                && (workspace_bearing || original.resource_id == resource.resource_id)).unwrap();
        assert_eq!(resource.owner_id, *owner);
        assert_eq!(resource.owner_id, original_resource.owner_id);
        assert_eq!(resource.operations, original_resource.operations, "freezing the shared physical root adds no operations");
        assert_eq!(resource.connection_config_ref, original_resource.connection_config_ref);
        if workspace_bearing {
            let mut expected_parameters = original_resource.typed_parameters.clone();
            expected_parameters.remove(nomifun_agent_contracts::MANAGED_WORKSPACE_LOCATION_PARAMETER);
            let inherited_root=resource.typed_parameters.get("workspace_root").unwrap();
            assert_same_workspace(inherited_root,&workspace);
            expected_parameters.insert("workspace_root".into(), inherited_root.clone());
            assert_eq!(resource.typed_parameters, expected_parameters);
        } else {
            assert_eq!(resource, original_resource);
        }
    }
    assert_same_workspace(child_projection.extra["workspace"].as_str().unwrap(),&workspace);
    let constraints = nomifun_api_types::ExecutionConstraints::from_extra(&child_projection.extra).unwrap();
    assert_eq!(constraints.tool_scope, AgentToolPolicy::Full);
    let effective: Value = serde_json::from_str(&running.attempt.effective_config).unwrap();
    assert_eq!(effective["tool_policy"], "full");
    assert_eq!(running.attempt.participant_id.as_deref(), Some(participant));
    let child_operation = state.session_owner.canonical.store().head(&child_session).await.unwrap().active_turn_id.unwrap();
    assert_eq!(child_operation, format!("turn:user:{owner}:{}:{child_attempt}:initial-turn", child_session.as_ref()));
    let original_result = AgentExecutionSessionPort::read_turn_output(state.session_owner.as_ref(), owner, lead.as_ref(), None)
        .await.unwrap().unwrap();
    assert_eq!(original_result.canonical_operation_id, root_operation);
    assert!(original_result.delivery.completed);
    assert!(original_result.delivery.result_text.as_deref().unwrap().contains("preserved business result"));
    assert_claim_preserved(&requirements, &claim).await;

    let inventory = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    let child_row = activity(&inventory, &child_id);
    assert_eq!(child_row["kind"], "delegated_task");
    assert_eq!(child_row["target"]["kind"], "execution_task");
    assert_eq!(child_row["target"]["attempt_id"], child_attempt);
    assert_eq!(child_row["agent_session_id"], child_session.as_ref());
    let root_row = inventory["items"].as_array().unwrap().iter().find(|row|
        row["target"]["kind"] == "auto_work_run" && row["target"]["requirement_id"] == claim.requirement_id
            && row["delegation_target"]["step_id"] == *root_id).unwrap();
    let root_delegation = root_row["delegation_target"].clone();
    let root_owner = root_row["target"].clone();
    assert_eq!(root_owner["target_id"], lead.as_ref());
    assert_eq!(root_delegation["attempt_id"], original_attempt.attempt_id);
    request(&router, "POST", "/api/agent-activities/stop", json!({"target":child_row["target"],
        "scope":"task","idempotency_key":"stop-the-running-real-autowork-child"})).await;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let receipt = state.session_owner.canonical.store().read_turn_receipt(&child_session, &child_operation.as_str().into()).await.unwrap();
            let carrier = services.agent_runtime_sessions.observe_runtime_carrier(child_session.as_ref());
            if receipt.status == nomifun_agent_session::TurnReceiptStatus::Cancelled
                && !carrier.registered && !carrier.teardown_pending && carrier.active_generation.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("task Stop must fence and stop the captured child's exact native Turn");
    let after_child = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    assert_eq!(after_child.steps.iter().find(|step| step.step_id == child_id).unwrap().status, "cancelled");
    assert_ne!(after_child.steps.iter().find(|step| &step.step_id == sibling_id).unwrap().status, "cancelled");
    assert_eq!(after_child.steps.iter().find(|step| &step.step_id == root_id).unwrap().status, "completed");
    assert_eq!(after_child.attempts.iter().filter(|attempt| attempt.attempt.step_id == child_id).count(), 1);
    assert_eq!(after_child.attempts.iter().find(|attempt| attempt.attempt.attempt_id == child_attempt).unwrap().conversation_id.as_deref(),
        Some(child_session.as_ref()));
    assert_claim_preserved(&requirements, &claim).await;
    assert_eq!(AgentExecutionSessionPort::read_turn_output(state.session_owner.as_ref(), owner, lead.as_ref(), None).await.unwrap().unwrap(),
        original_result, "the original root result reader never selects the last delegated Attempt");
    assert!(state.session_owner.canonical.store().head(&lead).await.unwrap().active_turn_id.is_none());
    assert!(std::fs::read_to_string(&preserved_file).unwrap().contains("Retain the original user workspace"));

    // Root business is already Completed. Its original delegation control may
    // still stop the remaining child, whether that sibling is queued or has
    // legitimately acquired the single slot released by the selected child.
    request(&router, "POST", "/api/agent-activities/stop", json!({"target":root_delegation,
        "scope":"delegated_work","idempotency_key":"completed-root-stops-only-remaining-delegates"})).await;
    let settled = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let detail = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
            if detail.execution.lease_owner.is_none()
                && matches!(detail.execution.status.as_str(), "completed_with_failures" | "completed" | "failed") {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("the original root and cancelled children must settle without an aggregate cancellation");
    assert_ne!(settled.execution.status, "cancelled");
    assert_eq!(settled.steps.iter().find(|step| &step.step_id == root_id).unwrap().status, "completed");
    assert!(settled.steps.iter().filter(|step| child_ids.contains(&step.step_id)).all(|step| step.status == "cancelled"));
    assert!(settled.attempts.iter().all(|attempt| attempt.attempt.attempt_no == 0));
    assert_claim_preserved(&requirements, &claim).await;
    let observed_calls = model.calls.load(Ordering::SeqCst);
    assert!((2..=3).contains(&observed_calls), "only the root and at most one request per original child may run");

    request(&router, "POST", "/api/agent-activities/stop", json!({"target":root_owner,
        "idempotency_key":"cancel-source-after-exact-child-retirement"})).await;
    let cancelled_claim = requirements.get_by_requirement_id(&claim.requirement_id).await.unwrap().unwrap();
    assert_eq!(cancelled_claim.status, "cancelled");
    assert_eq!(cancelled_claim.claim_generation, claim.claim_generation);
    assert_eq!(cancelled_claim.attempt_count, claim.attempt_count);
    let current_root = settled.steps.iter().find(|step| &step.step_id == root_id).unwrap();
    let rejected = repository.reset_steps_for_retry(owner, execution, settled.execution.version,
        &[RetryAgentExecutionStep { step_id: root_id.clone(), expected_step_version: current_root.version }],
        &event(AgentExecutionEventKind::StepChanged)).await.unwrap_err();
    assert!(matches!(rejected, nomifun_db::DbError::Conflict(ref reason) if reason.contains("Requirement claim")),
        "the original source cancellation fence must reject new work: {rejected}");
    engine.recover().await.unwrap();
    let recovered = repository.get_execution_detail(owner, execution).await.unwrap().unwrap();
    assert_eq!(recovered.attempts.len(), settled.attempts.len());
    assert_eq!(model.calls.load(Ordering::SeqCst), observed_calls, "source Stop and recovery never resurrect either child");
    assert_eq!(AgentExecutionSessionPort::read_turn_output(state.session_owner.as_ref(), owner, lead.as_ref(), None).await.unwrap().unwrap(),
        original_result);
    assert!(state.session_owner.canonical.store().get_live_session(&lead).await.is_ok());
    assert!(state.session_owner.canonical.store().get_live_session(&child_session).await.is_ok());
    assert!(preserved_file.exists());
    // A cancelled HTTP client cannot strand the fixture's own server waiter.
    model.finish.add_permits(2);
    services.shutdown_nomi_core_host().await.unwrap();
    assert!(preserved_file.exists(), "app-owned child cleanup preserves the admitted user workspace");
    services.database.close().await;
    model.close().await;
}
