use super::*;
use nomifun_common::{AgentExecutionActor, AgentExecutionCancellationScope as StopScope, ExecutionAttemptStatus};
use nomifun_db::{AdoptAgentExecutionStepOutputParams, AppendAgentExecutionStepsFromAttemptParams, RetryAgentExecutionStep,
    SettleAgentExecutionAttemptParams};

pub(super) struct Fixture {
    pub(super) db: nomifun_db::Database,
    pub(super) repository: SqliteAgentExecutionRepository,
    pub(super) owner: String,
    pub(super) execution_id: String,
}

impl Fixture {
    pub(super) async fn memory(titles: &[&str], dependencies: &[(&str, &str)]) -> Self {
        Self::new(database().await, OWNER_ID.to_owned(), titles, dependencies).await
    }

    async fn new(db: nomifun_db::Database, owner: String, titles: &[&str], dependencies: &[(&str, &str)]) -> Self {
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let participant_id=nomifun_common::generate_id();
        let mut participant=participant(participant_id.clone());
        participant.constraints=Some(r#"{"max_concurrency":3}"#.into());
        let created=repository.create_execution_with_participants(&owner,&execution_params(),&[participant],
            &event(AgentExecutionEventKind::Created)).await.unwrap();
        let steps: Vec<_> = titles.iter().map(|title| {
            let mut task = step(nomifun_common::generate_id(), Some(participant_id.clone()), title);
            task.tool_policy = AgentToolPolicy::ReadOnly;
            task
        }).collect();
        let dependency_rows = dependencies.iter().map(|(blocker, blocked)| NewAgentExecutionStepDependency {
            blocker_step_id: steps.iter().find(|step| step.title == *blocker).unwrap().step_id.clone(),
            blocked_step_id: steps.iter().find(|step| step.title == *blocked).unwrap().step_id.clone(),
        }).collect();
        repository.reconcile_plan(&owner, &created.execution_id, created.version,
            &ReconcileAgentExecutionPlanParams {
                goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
                keep_step_ids: Vec::new(), new_participants: Vec::new(), retire_participant_ids: Vec::new(),
                new_steps: steps, new_dependencies: dependency_rows, execution_status: AgentExecutionStatus::Running,
            }, &event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
        Self { db, repository, owner, execution_id: created.execution_id }
    }

    pub(super) async fn detail(&self) -> nomifun_db::AgentExecutionDetailRows {
        self.repository.get_execution_detail(&self.owner, &self.execution_id).await.unwrap().unwrap()
    }

    pub(super) async fn task(&self, title: &str) -> nomifun_db::AgentExecutionStepDetailRow {
        let detail = self.detail().await;
        let id = &detail.steps.iter().find(|step| step.title == title).unwrap().step_id;
        self.repository.get_step_detail(&self.owner, &self.execution_id, id).await.unwrap().unwrap()
    }

    pub(super) async fn start(&self, title: &str) -> nomifun_db::AgentExecutionStepDetailRow {
        let task = self.task(title).await;
        let queued = self.repository.create_attempt(&self.owner, &self.execution_id, &task.step.step_id,
            task.step.version, None, &CreateAgentExecutionAttemptParams {
                participant_id: task.step.assigned_participant_id.clone(), start_immediately: false,
                trigger_reason: "initial".into(), effective_config: "{}".into(), retry_after: None, runtime_state: None,
            }, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        let attempt = queued.current_attempt.as_ref().unwrap();
        let session = nomifun_common::generate_id();
        insert_canonical_session(&self.db, &session, &self.owner, "live").await;
        self.repository.start_attempt(&self.owner, &self.execution_id, &queued.step.step_id, queued.step.version,
            &attempt.attempt.attempt_id, attempt.attempt.version, &session,
            AgentExecutionAttemptSessionKind::ChildAttempt, None, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap()
    }

    fn stop_event(&self) -> NewAgentExecutionEvent {
        NewAgentExecutionEvent { actor: AgentExecutionActor::user(&self.owner), ..event(AgentExecutionEventKind::StepChanged) }
    }

    async fn stop(&self, title: &str, scope: StopScope) -> nomifun_db::AgentExecutionDetailRows {
        let detail = self.detail().await;
        let root = detail.steps.iter().find(|step| step.title == title).unwrap();
        self.repository.cancel_step(&self.owner, &self.execution_id, detail.execution.version,
            &root.step_id, root.version, scope, &self.stop_event()).await.unwrap()
    }

    async fn append_params(&self, title: &str, child: &str) -> (AppendAgentExecutionStepsFromAttemptParams, NewAgentExecutionEvent) {
        let root = self.task(title).await;
        let attempt = root.current_attempt.as_ref().unwrap();
        let session = attempt.conversation_id.clone().unwrap();
        let params = AppendAgentExecutionStepsFromAttemptParams {
            operation_id: nomifun_common::generate_id(), caller_conversation_id: session.clone(),
            caller_step_id: root.step.step_id, caller_attempt_id: attempt.attempt.attempt_id.clone(),
            expected_caller_step_version: root.step.version, expected_caller_attempt_version: attempt.attempt.version,
            new_steps: vec![{
                let mut child = step(nomifun_common::generate_id(), root.step.assigned_participant_id, child);
                child.tool_policy = AgentToolPolicy::ReadOnly;
                child
            }],
            new_dependencies: Vec::new(),
        };
        (params, NewAgentExecutionEvent { actor: AgentExecutionActor::agent(session, Some(attempt.attempt.attempt_id.clone())),
            ..event(AgentExecutionEventKind::PlanChanged) })
    }

    pub(super) async fn append(&self, title: &str, child: &str) -> String {
        let (params, event) = self.append_params(title, child).await;
        self.repository.append_steps_from_attempt(&self.owner, &self.execution_id, &params, &event)
            .await.unwrap().added_step_ids[0].clone()
    }

    pub(super) async fn complete(&self, title: &str) {
        let root = self.task(title).await;
        let attempt = root.current_attempt.as_ref().unwrap();
        self.repository.settle_attempt(&self.owner, &self.execution_id, &root.step.step_id, root.step.version,
            &attempt.attempt.attempt_id, attempt.attempt.version, None, &completed_params(),
            &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    }
}

fn completed_params() -> SettleAgentExecutionAttemptParams {
    SettleAgentExecutionAttemptParams {
        expected_active_session_turn: None, expected_terminal_turn: None,
        attempt_status: ExecutionAttemptStatus::Completed, step_status: ExecutionStepStatus::Completed,
        execution_status: None, question: Some(None), error: Some(None), output_summary: Some(Some("delivered".into())),
        output_files: None, tokens: None, retry_after: Some(None), runtime_state: Some(None),
        started_at: None, finished_at: None, loop_repeat_reset: None,
    }
}

#[tokio::test]
async fn scoped_stop_preserves_running_sibling_and_aggregate_and_fences_late_settlement() {
    let fixture = Fixture::memory(&["selected", "sibling"], &[]).await;
    let selected = fixture.start("selected").await;
    let sibling = fixture.start("sibling").await;
    let stopped = fixture.stop("selected", StopScope::Task).await;
    assert_eq!(stopped.execution.status, "running");
    assert_eq!(fixture.task("selected").await.step.status, "cancelled");
    assert_eq!(fixture.task("sibling").await.step.status, "running");
    let selected_attempt = selected.current_attempt.unwrap();
    assert!(fixture.repository.settle_attempt(&fixture.owner, &fixture.execution_id, &selected.step.step_id,
        selected.step.version, &selected_attempt.attempt.attempt_id, selected_attempt.attempt.version, None,
        &completed_params(), &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    fixture.complete("sibling").await;
    assert_eq!(fixture.task("sibling").await.current_attempt.unwrap().attempt.attempt_id,
        sibling.current_attempt.unwrap().attempt.attempt_id);
    assert_eq!(fixture.task("sibling").await.step.status, "completed");
    let links = fixture.repository.list_conversation_links(&fixture.owner, &fixture.execution_id).await.unwrap();
    assert!(links.iter().all(|link| !link.active));
    assert!(links.iter().all(|link| link.cleanup_completed_at.is_none()), "domain acceptance cannot fabricate native cleanup");
}

#[tokio::test]
async fn scoped_stop_completed_parent_uses_delegation_facts_and_preserves_shared_dag_join() {
    let fixture = Fixture::memory(&["parent", "independent", "shared-join"],
        &[("parent", "shared-join"), ("independent", "shared-join")]).await;
    nomifun_db::sqlx::query("UPDATE agent_executions SET max_parallel=3 WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    fixture.start("parent").await;
    fixture.start("independent").await;
    fixture.append("parent", "owned-child").await;
    fixture.start("owned-child").await;
    fixture.append("owned-child", "owned-grandchild").await;
    fixture.complete("parent").await;
    let stopped = fixture.stop("parent", StopScope::TaskAndDelegated).await;
    assert_eq!(stopped.execution.status, "running");
    for (title, status) in [("parent", "completed"), ("owned-child", "cancelled"),
        ("owned-grandchild", "cancelled"), ("independent", "running"), ("shared-join", "pending")] {
        assert_eq!(fixture.task(title).await.step.status, status, "{title}");
    }
    let join = fixture.task("shared-join").await.step;
    assert!(stopped.dependencies.iter().any(|edge| edge.blocked_step_id == join.step_id), "missing inputs remain real dependencies");
    let parent = fixture.task("parent").await.step;
    assert!(fixture.repository.reset_steps_for_retry(&fixture.owner, &fixture.execution_id, stopped.execution.version,
        &[RetryAgentExecutionStep { step_id: parent.step_id, expected_step_version: parent.version }],
        &event(AgentExecutionEventKind::StepChanged)).await.is_err(), "a settled ownership root cannot revive its cancelled range");
}

#[tokio::test]
async fn scoped_stop_can_expand_a_cancelled_parent_to_its_remaining_owned_work() {
    let fixture = Fixture::memory(&["parent", "sibling"], &[]).await;
    fixture.start("parent").await;
    fixture.append("parent", "child").await;
    fixture.stop("parent", StopScope::Task).await;
    assert_eq!(fixture.task("child").await.step.status, "pending");
    fixture.stop("parent", StopScope::TaskAndDelegated).await;
    assert_eq!(fixture.task("child").await.step.status, "cancelled");
    assert_eq!(fixture.task("sibling").await.step.status, "pending");
}

#[tokio::test]
async fn scoped_stop_exact_replay_is_idempotent_and_stale_buttons_cannot_select_new_work() {
    let fixture = Fixture::memory(&["selected", "sibling"], &[]).await;
    let before = fixture.detail().await;
    let selected = &before.steps[0];
    let command = fixture.stop_event();
    let first = fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, before.execution.version,
        &selected.step_id, selected.version, StopScope::Task, &command).await.unwrap();
    let repeated = fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, before.execution.version,
        &selected.step_id, selected.version, StopScope::Task, &command).await.unwrap();
    assert_eq!(first.execution.version, repeated.execution.version);
    assert_eq!(first.execution.event_sequence, repeated.execution.event_sequence);
    let sibling = before.steps.iter().find(|step| step.step_id != selected.step_id).unwrap();
    assert!(fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, first.execution.version + 1,
        &sibling.step_id, sibling.version, StopScope::Task, &command).await.is_err(), "future aggregate versions are not captured authority");
    assert!(fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, first.execution.version,
        &sibling.step_id, sibling.version + 1, StopScope::Task, &command).await.is_err());
    assert!(fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, -1,
        &sibling.step_id, sibling.version, StopScope::Task, &command).await.is_err());
    assert_eq!(fixture.detail().await.execution.version, first.execution.version);
    assert_eq!(fixture.task(&sibling.title).await.step.status, "pending");
    let sibling_stopped=fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,before.execution.version,
        &sibling.step_id,sibling.version,StopScope::Task,&command).await.unwrap();
    assert_eq!(fixture.task(&sibling.title).await.step.status,"cancelled", "an unrelated aggregate version does not change the exact task target");
    assert!(sibling_stopped.execution.version>first.execution.version);
}

#[tokio::test]
async fn scoped_stop_blocks_dispatch_retry_and_inflight_delegation_after_acceptance() {
    let fixture = Fixture::memory(&["parent", "sibling"], &[]).await;
    fixture.start("parent").await;
    let (append, append_event) = fixture.append_params("parent", "late-child").await;
    let stopped = fixture.stop("parent", StopScope::TaskAndDelegated).await;
    assert!(fixture.repository.append_steps_from_attempt(&fixture.owner, &fixture.execution_id, &append, &append_event).await.is_err());
    let root = fixture.task("parent").await;
    assert!(fixture.repository.create_attempt(&fixture.owner, &fixture.execution_id, &root.step.step_id, root.step.version,
        None, &CreateAgentExecutionAttemptParams { participant_id: root.step.assigned_participant_id.clone(),
            start_immediately: false, trigger_reason: "retry".into(), effective_config: "{}".into(), retry_after: None, runtime_state: None },
        &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    assert!(fixture.repository.reset_steps_for_retry(&fixture.owner, &fixture.execution_id, stopped.execution.version,
        &[RetryAgentExecutionStep { step_id: root.step.step_id, expected_step_version: root.step.version }],
        &event(AgentExecutionEventKind::StepChanged)).await.is_err());
    assert_eq!(fixture.detail().await.steps.len(), 2);
    assert_eq!(fixture.task("sibling").await.step.status, "pending");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_stop_and_concurrent_append_share_the_writer_fence() {
    let directory = tempfile::tempdir().unwrap();
    let (db, owner) = file_database(&directory.path().join("stop-append.db")).await;
    let fixture = Fixture::new(db, owner, &["parent", "sibling"], &[]).await;
    fixture.start("parent").await;
    let before = fixture.detail().await;
    let parent = fixture.task("parent").await;
    let (append, append_event) = fixture.append_params("parent", "racing-child").await;
    let stop_event = fixture.stop_event();
    let (stop_result, append_result) = tokio::join!(
        fixture.repository.cancel_step(&fixture.owner, &fixture.execution_id, before.execution.version,
            &parent.step.step_id, parent.step.version, StopScope::TaskAndDelegated, &stop_event),
        fixture.repository.append_steps_from_attempt(&fixture.owner, &fixture.execution_id, &append, &append_event),
    );
    assert!(stop_result.is_ok(), "an append that wins first cannot invalidate the unchanged exact Stop target");
    if append_result.is_ok() {
        assert_eq!(fixture.task("racing-child").await.step.status, "cancelled");
    } else {
        assert!(!fixture.detail().await.steps.iter().any(|step| step.title == "racing-child"));
    }
    assert_eq!(fixture.task("parent").await.step.status, "cancelled");
    assert_eq!(fixture.task("sibling").await.step.status, "pending");
    assert_eq!(fixture.detail().await.execution.status, "running");
}

#[tokio::test]
async fn scoped_stop_preserves_explicit_whole_execution_cancellation() {
    let fixture = Fixture::memory(&["one", "two"], &[]).await;
    fixture.start("one").await;
    fixture.start("two").await;
    let detail = fixture.detail().await;
    let stopped = fixture.repository.cancel_execution(&fixture.owner, &fixture.execution_id, detail.execution.version,
        &fixture.stop_event()).await.unwrap();
    assert_eq!(stopped.execution.status, "cancelled");
    assert!(stopped.steps.iter().all(|step| step.status == "cancelled"));
    assert!(stopped.attempts.iter().all(|attempt| attempt.attempt.status == "cancelled"));
    assert!(fixture.repository.list_conversation_links(&fixture.owner, &fixture.execution_id).await.unwrap()
        .iter().all(|link| !link.active && link.cleanup_completed_at.is_none()));
}

#[tokio::test]
async fn delegated_only_stop_preserves_the_running_parent_and_shared_dag_join() {
    let fixture=Fixture::memory(&["parent","independent","shared-join"],
        &[("parent","shared-join"),("independent","shared-join")]).await;
    nomifun_db::sqlx::query("UPDATE agent_executions SET max_parallel=3 WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    let parent=fixture.start("parent").await;
    fixture.start("independent").await;
    fixture.append("parent","child").await;
    fixture.start("child").await;
    fixture.append("child","grandchild").await;
    let stopped=fixture.stop("parent",StopScope::DelegatedWork).await;
    let still_running=fixture.task("parent").await;
    assert_eq!(still_running.step.status,"running");
    assert_eq!(still_running.step.version,parent.step.version);
    assert_eq!(still_running.current_attempt.as_ref().unwrap().attempt.attempt_id,parent.current_attempt.as_ref().unwrap().attempt.attempt_id);
    assert_eq!(still_running.current_attempt.as_ref().unwrap().attempt.version,parent.current_attempt.as_ref().unwrap().attempt.version);
    assert_eq!(fixture.task("child").await.step.status,"cancelled");
    assert_eq!(fixture.task("grandchild").await.step.status,"cancelled");
    assert_eq!(fixture.task("independent").await.step.status,"running");
    assert_eq!(fixture.task("shared-join").await.step.status,"pending");
    assert_eq!(stopped.execution.status,"running");
    let links=fixture.repository.list_conversation_links(&fixture.owner,&fixture.execution_id).await.unwrap();
    assert!(links.iter().any(|link|link.step_id.as_deref()==Some(parent.step.step_id.as_str()) && link.active));
    let events=fixture.repository.list_events(&fixture.owner,&fixture.execution_id,0,100).await.unwrap();
    let stop=events.iter().rev().find(|event|event.event_type=="step_changed").unwrap();
    let payload:serde_json::Value=serde_json::from_str(&stop.payload).unwrap();
    assert_eq!(payload["scope"],"delegated_work");
    assert!(!payload["cancelled_step_ids"].as_array().unwrap().iter().any(|id|id==&parent.step.step_id));
    fixture.complete("parent").await;
    assert_eq!(fixture.task("parent").await.step.status,"completed", "delegation fencing cannot cancel parent business settlement");
    let completed=fixture.detail().await;
    let replay=fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,
        payload["expected_execution_version"].as_i64().unwrap(),&parent.step.step_id,parent.step.version,
        StopScope::DelegatedWork,&fixture.stop_event()).await.unwrap();
    assert_eq!(replay.execution.event_sequence,completed.execution.event_sequence,
        "an exact scope replay remains idempotent after legitimate parent settlement");
}

#[tokio::test]
async fn delegated_only_stop_fences_parent_retry_delegation_and_replan_without_stopping_parent_work() {
    let fixture=Fixture::memory(&["parent","independent"],&[]).await;
    fixture.start("parent").await;
    let captured=fixture.detail().await;
    let root=fixture.task("parent").await.step;
    // An append accepted before Stop is included even if it advanced only the
    // aggregate plan revision after the user's view was captured.
    fixture.append("parent","appended-before-stop").await;
    fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,captured.execution.version,
        &root.step_id,root.version,StopScope::DelegatedWork,&fixture.stop_event()).await.unwrap();
    assert_eq!(fixture.task("appended-before-stop").await.step.status,"cancelled");
    let (append,delegation_event)=fixture.append_params("parent","after-stop").await;
    assert!(fixture.repository.append_steps_from_attempt(&fixture.owner,&fixture.execution_id,&append,&delegation_event).await.is_err());
    fixture.complete("parent").await;
    let before_retry=fixture.detail().await;
    let root=fixture.task("parent").await.step;
    fixture.repository.reset_steps_for_retry(&fixture.owner,&fixture.execution_id,before_retry.execution.version,
        &[RetryAgentExecutionStep {step_id:root.step_id,expected_step_version:root.version}],&event(AgentExecutionEventKind::StepChanged)).await.unwrap();
    fixture.start("parent").await;
    let (append,mut delegation_event)=fixture.append_params("parent","retry-bypass").await;
    assert!(fixture.repository.append_steps_from_attempt(&fixture.owner,&fixture.execution_id,&append,&delegation_event).await.is_err());
    let before=fixture.detail().await;
    delegation_event.step_id=Some(append.caller_step_id.clone());
    delegation_event.attempt_id=Some(append.caller_attempt_id.clone());
    assert!(fixture.repository.reconcile_plan(&fixture.owner,&fixture.execution_id,before.execution.version,
        &ReconcileAgentExecutionPlanParams {goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,
            keep_step_ids:before.steps.iter().map(|step|step.step_id.clone()).collect(),new_participants:Vec::new(),retire_participant_ids:Vec::new(),
            new_steps:append.new_steps,new_dependencies:Vec::new(),execution_status:AgentExecutionStatus::Running},&delegation_event).await.is_err());
    assert_eq!(fixture.detail().await.execution.version,before.execution.version);
    assert_eq!(fixture.task("parent").await.step.status,"running");
}

#[tokio::test]
async fn exact_task_stop_accepts_after_unrelated_sibling_settlement_but_whole_stop_keeps_its_cas() {
    let fixture=Fixture::memory(&["selected","sibling"],&[]).await;
    fixture.start("selected").await;
    fixture.start("sibling").await;
    let captured=fixture.detail().await;
    let selected=fixture.task("selected").await.step;
    fixture.complete("sibling").await;
    assert!(fixture.repository.cancel_execution(&fixture.owner,&fixture.execution_id,captured.execution.version,
        &fixture.stop_event()).await.is_err(), "whole Execution control retains its aggregate CAS");
    let stopped=fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,captured.execution.version,
        &selected.step_id,selected.version,StopScope::Task,&fixture.stop_event()).await.unwrap();
    assert_eq!(stopped.execution.status,"running");
    assert_eq!(fixture.task("selected").await.step.status,"cancelled");
    assert_eq!(fixture.task("sibling").await.step.status,"completed");
}

#[tokio::test]
async fn scoped_stop_of_a_completed_loop_body_cannot_be_erased_by_iteration_reset() {
    let fixture = Fixture::memory(&["body", "loop", "independent"], &[("body", "loop")]).await;
    let controller = fixture.task("loop").await.step;
    nomifun_db::sqlx::query(
        "UPDATE agent_execution_steps SET kind='loop',agent_mode=NULL,assigned_participant_id=NULL WHERE step_id=?",
    ).bind(&controller.step_id).execute(fixture.db.pool()).await.unwrap();
    fixture.start("body").await;
    fixture.complete("body").await;
    fixture.stop("body", StopScope::TaskAndDelegated).await;
    let body = fixture.task("body").await.step;
    assert_eq!(body.status, "completed", "scope fencing preserves the already delivered result");
    let started = fixture.repository.create_attempt(&fixture.owner, &fixture.execution_id, &controller.step_id,
        controller.version, None, &CreateAgentExecutionAttemptParams {
            participant_id: None, start_immediately: true, trigger_reason: "loop".into(),
            effective_config: "{}".into(), retry_after: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt = started.current_attempt.as_ref().unwrap();
    let mut repeat = completed_params();
    repeat.step_status = ExecutionStepStatus::Pending;
    repeat.loop_repeat_reset = Some(nomifun_db::LoopRepeatResetParams {
        body_step_id: body.step_id.clone(),
        expected_steps: vec![RetryAgentExecutionStep { step_id: body.step_id, expected_step_version: body.version }],
    });
    assert!(fixture.repository.settle_attempt(&fixture.owner, &fixture.execution_id, &controller.step_id,
        started.step.version, &attempt.attempt.attempt_id, attempt.attempt.version, None, &repeat,
        &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    assert_eq!(fixture.task("body").await.step.status, "completed");
    assert_eq!(fixture.task("loop").await.step.status, "running", "a rejected reset cannot partially settle its controller");
    assert_eq!(fixture.task("independent").await.step.status, "pending");
}

#[tokio::test]
async fn scoped_stop_of_a_completed_root_cannot_be_reopened_by_output_adoption() {
    let fixture = Fixture::memory(&["completed-root", "independent"], &[]).await;
    fixture.start("completed-root").await;
    fixture.complete("completed-root").await;
    let stopped = fixture.stop("completed-root", StopScope::TaskAndDelegated).await;
    let root = fixture.task("completed-root").await;
    assert!(fixture.repository.adopt_step_output(&fixture.owner, &fixture.execution_id, stopped.execution.version,
        &root.step.step_id, root.step.version, &AdoptAgentExecutionStepOutputParams {
            output_summary: "replacement delivery".into(), output_files: "[]".into(), tokens: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    let after = fixture.detail().await;
    assert_eq!(after.execution.version, stopped.execution.version);
    assert_eq!(after.attempts.len(), stopped.attempts.len(), "a stopped scope cannot acquire an adopted successor Attempt");
    assert_eq!(fixture.task("completed-root").await.step.status, "completed");
    assert_eq!(fixture.task("independent").await.step.status, "pending");
}

#[tokio::test]
async fn adoption_repairs_an_unstopped_source_without_reopening_its_stopped_downstream() {
    let fixture = Fixture::memory(&["source", "skipped-dependent", "independent"], &[("source", "skipped-dependent")]).await;
    let source = fixture.start("source").await;
    let attempt = source.current_attempt.as_ref().unwrap();
    let mut failed = completed_params();
    failed.attempt_status = ExecutionAttemptStatus::Failed;
    failed.step_status = ExecutionStepStatus::Failed;
    failed.error = Some(Some("original failure".into()));
    failed.output_summary = Some(None);
    fixture.repository.settle_attempt(&fixture.owner, &fixture.execution_id, &source.step.step_id, source.step.version,
        &attempt.attempt.attempt_id, attempt.attempt.version, None, &failed,
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_execution_steps SET status='skipped',version=version+1 WHERE execution_id=? AND title='skipped-dependent'")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    let stopped = fixture.stop("skipped-dependent", StopScope::TaskAndDelegated).await;
    let stopped_dependent = fixture.task("skipped-dependent").await.step;
    let source = fixture.task("source").await;
    fixture.repository.adopt_step_output(&fixture.owner, &fixture.execution_id, stopped.execution.version,
        &source.step.step_id, source.step.version, &AdoptAgentExecutionStepOutputParams {
            output_summary: "verified source output".into(), output_files: "[]".into(), tokens: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(fixture.task("source").await.step.status, "completed", "independent output repair remains usable");
    let dependent = fixture.task("skipped-dependent").await.step;
    assert_eq!(dependent.status, "skipped");
    assert_eq!(dependent.version, stopped_dependent.version, "a DAG repair cannot reset an independently stopped task");
    assert_eq!(fixture.task("independent").await.step.status, "pending");
}

#[tokio::test]
async fn decision_answer_queues_until_original_participant_capacity_is_reacquired() {
    let fixture = Fixture::memory(&["waiting", "peer"], &[]).await;
    nomifun_db::sqlx::query("UPDATE agent_executions SET max_parallel=1 WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    let running = fixture.start("waiting").await;
    let attempt = running.current_attempt.as_ref().unwrap();
    let mut waiting = completed_params();
    waiting.attempt_status = ExecutionAttemptStatus::WaitingInput;
    waiting.step_status = ExecutionStepStatus::WaitingInput;
    waiting.execution_status = Some(AgentExecutionStatus::WaitingInput);
    waiting.question = Some(Some("continue?".into()));
    waiting.output_summary = None;
    // WaitingInput alone is not capacity-release proof. Seed the same exact
    // typed terminal that the production Session commits before settlement.
    let session=attempt.conversation_id.as_deref().unwrap();
    let operation=format!("turn:user:{}:{session}:{}:initial-turn",fixture.owner,attempt.attempt.attempt_id);
    let started=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();
    for (seq,id,kind,value) in [(1,&started,"turn/started",serde_json::json!({})),
        (2,&terminal,"turn/completed",serde_json::json!({"user_input_request":{
            "request_id":"original-question","question":"continue?","remaining_work":"Continue the original task."}}))] {
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'capacity-fixture',?,?,1,?,?)")
            .bind(session).bind(seq).bind(id).bind(id).bind(kind).bind(&operation).bind(value.to_string())
            .execute(fixture.db.pool()).await.unwrap();
    }
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,1,1,2)")
        .bind(session).bind(&operation).bind(&operation).bind(&operation).bind(&started).bind(&terminal).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,last_seq,active_set_generation,unread_count) VALUES(?,'ready',2,0,0)")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    fixture.repository.settle_attempt(&fixture.owner, &fixture.execution_id, &running.step.step_id, running.step.version,
        &attempt.attempt.attempt_id, attempt.attempt.version, None, &waiting, &event(AgentExecutionEventKind::DecisionRequested)).await.unwrap();
    fixture.start("peer").await;
    let waiting = fixture.task("waiting").await;
    let attempt = waiting.current_attempt.as_ref().unwrap();
    let before = fixture.detail().await;
    let mut effects = nomifun_db::AttemptConversationEffects::default();
    effects.push_decision("queued-original-answer".into(), "continue from the original progress".into()).unwrap();
    let queued = fixture.repository.resume_waiting_attempt(&fixture.owner, &fixture.execution_id, before.execution.version,
        &waiting.step.step_id, waiting.step.version, &attempt.attempt.attempt_id, attempt.attempt.version,
        &nomifun_db::AttemptConversationEffectParams { runtime_state:Some(effects.encode().unwrap()) },
        &event(AgentExecutionEventKind::DecisionAnswered)).await.unwrap();
    assert_eq!(queued.detail.step.status, "waiting_input");
    assert_eq!(queued.detail.current_attempt.as_ref().unwrap().attempt.attempt_id, attempt.attempt.attempt_id);
    let lease = AgentExecutionLeaseToken::new("queued-decision-admission".into());
    let current = fixture.detail().await;
    fixture.repository.try_acquire_lease(&fixture.execution_id, current.execution.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    assert!(fixture.repository.admit_waiting_attempt_input(&fixture.owner, &fixture.execution_id,
        &waiting.step.step_id, &attempt.attempt.attempt_id, &lease, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap().is_none());
    fixture.complete("peer").await;
    let admitted = fixture.repository.admit_waiting_attempt_input(&fixture.owner, &fixture.execution_id,
        &waiting.step.step_id, &attempt.attempt.attempt_id, &lease, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap().unwrap();
    assert_eq!(admitted.step.status, "running");
    let after_attempt = admitted.current_attempt.unwrap().attempt;
    assert_eq!(after_attempt.attempt_id, attempt.attempt.attempt_id);
    assert_eq!(after_attempt.runtime_state, Some(effects.encode().unwrap()));
    let final_detail = fixture.detail().await;
    assert_eq!(final_detail.attempts.len(), 2, "queueing/resuming adds no replacement Attempt");
    assert_eq!(final_detail.execution.max_parallel, 1);
}

#[tokio::test]
async fn native_pause_yields_capacity_only_after_real_owner_handoff_and_resume_rechecks_it() {
    use nomifun_db::{execution_capacity_available_tx, execution_reservations_tx,
        native_execution_claim_admitted_tx, native_resume_capacity_available_tx};
    let fixture = Fixture::memory(&["parked", "candidate"], &[]).await;
    nomifun_db::sqlx::query("UPDATE agent_execution_participants SET constraints=? WHERE execution_id=?")
        .bind(r#"{"max_concurrency":1}"#).bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    let parked = fixture.start("parked").await;
    let original = parked.current_attempt.as_ref().unwrap();
    let session = original.conversation_id.as_ref().unwrap();
    let operation = format!("turn:user:{}:{session}:{}:initial-turn", fixture.owner, original.attempt.attempt_id);
    let mut pause = nomifun_agent_contracts::NativePauseState { revision:1,reason:"USER_REQUESTED".into(),
        checkpoint_revision:3,checkpoint_digest:Some("c".repeat(64).into()),execution_fence:9,cleanup_proven:false,paused_at_ms:1 };
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'paused',?,0,1,0)")
        .bind(session).bind(&operation).execute(fixture.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at,started_at,native_pause_json,native_pause_revision,native_checkpoint_revision,native_checkpoint_digest,execution_fence) VALUES(?,?,?,?,'running',1,1,?,1,3,?,9)")
        .bind(session).bind(&operation).bind(&operation).bind(&operation).bind(serde_json::to_string(&pause).unwrap())
        .bind("c".repeat(64)).execute(fixture.db.pool()).await.unwrap();
    let candidate = fixture.task("candidate").await.step;
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap()[0].capacity_held);
    assert!(!execution_capacity_available_tx(&mut tx, &fixture.execution_id, &candidate.step_id, None).await.unwrap(),
        "checkpoint/pause state alone is not cleanup evidence");
    assert!(native_execution_claim_admitted_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "an original reservation that was never yielded remains owned");
    tx.commit().await.unwrap();
    pause.cleanup_proven = true;
    nomifun_db::sqlx::query("UPDATE agent_turns SET native_pause_json=? WHERE session_id=? AND operation_id=?")
        .bind(serde_json::to_string(&pause).unwrap()).bind(session).bind(&operation).execute(fixture.db.pool()).await.unwrap();
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(!execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap()[0].capacity_held);
    assert!(!native_execution_claim_admitted_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "a native claim cannot steal back a yielded reservation before resume authorization");
    assert!(execution_capacity_available_tx(&mut tx, &fixture.execution_id, &candidate.step_id, None).await.unwrap());
    assert!(native_resume_capacity_available_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap());
    tx.commit().await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_execution_steps SET tool_policy='full' WHERE step_id=?")
        .bind(&parked.step.step_id).execute(fixture.db.pool()).await.unwrap();
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(!execution_capacity_available_tx(&mut tx, &fixture.execution_id, &candidate.step_id, None).await.unwrap(),
        "yielding model capacity cannot release a possible retained shell writer");
    tx.commit().await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_execution_steps SET tool_policy='read_only' WHERE step_id=?")
        .bind(&parked.step.step_id).execute(fixture.db.pool()).await.unwrap();
    fixture.start("candidate").await;
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(!native_resume_capacity_available_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "native resume must queue behind the same Participant reservation");
    tx.commit().await.unwrap();
    fixture.stop("parked", StopScope::Task).await;
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(native_resume_capacity_available_tx(&mut tx, &fixture.owner, session, &operation).await.is_err(),
        "late resume cannot bypass an exact durable user stop");
    assert!(native_execution_claim_admitted_tx(&mut tx, &fixture.owner, session, &operation).await.is_err());
    tx.commit().await.unwrap();
}
