//! Formal Stop re-arms the exact old cleanup outbox, never its business work.
use super::*;
use super::scoped_stop::Fixture;
use nomifun_common::{AgentExecutionActor, AgentExecutionCancellationScope as StopScope, ExecutionAttemptStatus};
use nomifun_db::{AgentExecutionAttemptRow, ConversationCleanupTurnTarget, PendingConversationCleanup,
    RetryAgentExecutionStep, SettleAgentExecutionAttemptParams};

async fn fail_with_backoff(f: &Fixture, title: &str) -> nomifun_db::AgentExecutionStepDetailRow {
    fail(f, title, true).await
}

async fn fail(f: &Fixture, title: &str, will_retry: bool) -> nomifun_db::AgentExecutionStepDetailRow {
    let task = f.task(title).await;
    let attempt = &task.current_attempt.as_ref().unwrap().attempt;
    f.repository.settle_attempt(&f.owner, &f.execution_id, &task.step.step_id, task.step.version,
        &attempt.attempt_id, attempt.version, None, &SettleAgentExecutionAttemptParams {
            expected_active_session_turn: None, expected_terminal_turn: None,
            attempt_status: ExecutionAttemptStatus::Failed,
            step_status: if will_retry { ExecutionStepStatus::Pending } else { ExecutionStepStatus::Failed },
            execution_status: None, question: Some(None), error: Some(Some("original retryable provider failure".into())),
            output_summary: Some(None), output_files: None, tokens: Some(Some(7)),
            retry_after: Some(will_retry.then(|| nomifun_common::now_ms() + 60_000)), runtime_state: Some(None),
            started_at: None, finished_at: None, loop_repeat_reset: None,
        }, &NewAgentExecutionEvent {
            step_id: Some(task.step.step_id.clone()), attempt_id: Some(attempt.attempt_id.clone()),
            payload: serde_json::json!({"attempt_status":"failed","step_status":if will_retry {"pending"} else {"failed"},
                "turn_operation_key":format!("{}:initial-turn",attempt.attempt_id)}).to_string(),
            ..event(AgentExecutionEventKind::AttemptChanged)
        }).await.unwrap()
}

async fn acknowledge(f: &Fixture, when: i64) {
    for cleanup in f.repository.list_pending_conversation_cleanups(Some(&f.execution_id), None, 100).await.unwrap().cleanups {
        assert!(f.repository.validate_conversation_cleanup(&cleanup).await.unwrap());
        assert!(f.repository.mark_conversation_cleanup_completed_exact(&cleanup, when).await.unwrap());
    }
    assert!(f.repository.list_pending_conversation_cleanups(Some(&f.execution_id), None, 100).await.unwrap().cleanups.is_empty());
}

async fn audit(f: &Fixture, attempt: &str) -> serde_json::Value {
    let row = nomifun_db::sqlx::query_as::<_, AgentExecutionAttemptRow>(
        "SELECT * FROM agent_execution_attempts WHERE execution_id=? AND attempt_id=?")
        .bind(&f.execution_id).bind(attempt).fetch_one(f.db.pool()).await.unwrap();
    serde_json::to_value(row).unwrap()
}

async fn pending(f: &Fixture) -> Vec<PendingConversationCleanup> {
    f.repository.list_pending_conversation_cleanups(Some(&f.execution_id), None, 100).await.unwrap().cleanups
}

fn stop_event() -> NewAgentExecutionEvent {
    NewAgentExecutionEvent { actor: AgentExecutionActor::user(OWNER_ID), ..event(AgentExecutionEventKind::StepChanged) }
}

#[tokio::test]
async fn stopped_backoff_task_requeues_only_its_original_owner_and_exact_replay_cannot_rearm_it_twice() {
    let f = Fixture::memory(&["selected", "legal-retry", "finished"], &[]).await;
    f.start("selected").await;
    let selected = fail_with_backoff(&f, "selected").await;
    f.start("legal-retry").await;
    let sibling = fail_with_backoff(&f, "legal-retry").await;
    f.start("finished").await; f.complete("finished").await;
    acknowledge(&f, 700).await;
    let attempt = &selected.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let sibling_id = &sibling.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let original = audit(&f, attempt).await;
    let sibling_audit = audit(&f, sibling_id).await;
    let captured = f.detail().await;
    assert!(f.repository.cancel_execution(&f.owner, &f.execution_id, captured.execution.version,
        &NewAgentExecutionEvent { actor: AgentExecutionActor::user(FOREIGN_CANONICAL_LEAD_ID),
            ..event(AgentExecutionEventKind::StatusChanged) }).await.is_err());
    assert!(pending(&f).await.is_empty(), "a rejected writer rolls back re-armed acknowledgements with the Stop");
    assert_eq!(f.task("selected").await.step.status, "pending");
    let cancelled = f.repository.cancel_step(&f.owner, &f.execution_id, captured.execution.version,
        &selected.step.step_id, selected.step.version, StopScope::Task, &stop_event()).await.unwrap();
    assert_eq!(f.task("selected").await.step.status, "cancelled");
    assert_eq!(audit(&f, attempt).await, original, "Stop preserves the failed result, version, time and retry audit");
    assert_eq!(audit(&f, sibling_id).await, sibling_audit);
    assert_eq!(f.task("legal-retry").await.step.status, "pending");
    assert_eq!(f.task("finished").await.step.status, "completed");
    let cleanups = pending(&f).await;
    assert_eq!(cleanups.len(), 1);
    assert_eq!(&cleanups[0].attempt_id, attempt);
    let session = selected.current_attempt.as_ref().unwrap().conversation_id.as_ref().unwrap();
    assert_eq!(cleanups[0].target, ConversationCleanupTurnTarget::Exact {
        operation_id: format!("turn:user:{}:{session}:{attempt}:initial-turn", f.owner),
    });
    let facts = f.repository.read_task_retirement_facts(&f.owner, session, &f.execution_id, &selected.step.step_id, attempt)
        .await.unwrap().unwrap();
    assert_eq!(facts.detail.step.status, "cancelled");
    assert_eq!(facts.original_attempt.attempt.retry_after, original["retry_after"].as_i64());
    assert!(facts.events.iter().any(|event| event.event_type == "step_changed"));
    let links = f.repository.list_conversation_links(&f.owner, &f.execution_id).await.unwrap();
    assert!(links.iter().filter(|link| link.attempt_id.as_deref() != Some(attempt)).all(|link| link.cleanup_completed_at == Some(700)));
    acknowledge(&f, 900).await;
    let replay = f.repository.cancel_step(&f.owner, &f.execution_id, captured.execution.version,
        &selected.step.step_id, selected.step.version, StopScope::Task, &stop_event()).await.unwrap();
    assert_eq!(replay.execution.version, cancelled.execution.version);
    assert!(pending(&f).await.is_empty(), "an exact Stop replay does not create another cleanup generation");
    assert!(f.repository.create_attempt(&f.owner, &f.execution_id, &selected.step.step_id,
        f.task("selected").await.step.version, None, &CreateAgentExecutionAttemptParams {
            participant_id: selected.step.assigned_participant_id, start_immediately: false,
            trigger_reason: "forbidden stopped retry".into(), effective_config: "{}".into(), retry_after: None, runtime_state: None,
        }, &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
}

#[tokio::test]
async fn stopped_backoff_rearms_every_original_terminal_owner_without_retargeting_a_successor() {
    let f = Fixture::memory(&["selected"], &[]).await;
    f.start("selected").await;
    let first = fail_with_backoff(&f, "selected").await;
    acknowledge(&f, 700).await;
    // A legitimate retry acquires a different Attempt and Session. While the
    // Step remains pending, it must not re-arm the older receipt by itself.
    f.start("selected").await;
    let current = fail_with_backoff(&f, "selected").await;
    acknowledge(&f, 800).await;
    let old_id = &first.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let current_id = &current.current_attempt.as_ref().unwrap().attempt.attempt_id;
    assert_ne!(old_id, current_id);
    let before = f.repository.list_conversation_links(&f.owner, &f.execution_id).await.unwrap();
    assert!(before.iter().filter(|link| link.attempt_id.as_deref() == Some(old_id)).all(|link| link.cleanup_completed_at == Some(700)));
    let old_audit = audit(&f, old_id).await;
    let current_audit = audit(&f, current_id).await;
    let detail = f.detail().await;
    f.repository.cancel_step(&f.owner, &f.execution_id, detail.execution.version,
        &current.step.step_id, current.step.version, StopScope::Task, &stop_event()).await.unwrap();
    let cleanups = pending(&f).await;
    assert_eq!(cleanups.len(), 2);
    for source in [&first, &current] {
        let original = source.current_attempt.as_ref().unwrap();
        let cleanup = cleanups.iter().find(|cleanup| cleanup.attempt_id == original.attempt.attempt_id).unwrap();
        let session = original.conversation_id.as_ref().unwrap();
        assert_eq!(&cleanup.conversation_id, session);
        assert_eq!(cleanup.target, ConversationCleanupTurnTarget::Exact {
            operation_id: format!("turn:user:{}:{session}:{}:initial-turn", f.owner, original.attempt.attempt_id),
        });
    }
    let links = f.repository.list_conversation_links(&f.owner, &f.execution_id).await.unwrap();
    assert!(links.iter().all(|link| !link.active && link.cleanup_completed_at.is_none()));
    let facts = f.repository.read_task_retirement_facts(&f.owner, first.current_attempt.as_ref().unwrap().conversation_id.as_deref().unwrap(),
        &f.execution_id, &first.step.step_id, old_id).await.unwrap().unwrap();
    assert_eq!(&facts.original_attempt.attempt.attempt_id, old_id);
    assert_eq!(&facts.detail.current_attempt.unwrap().attempt.attempt_id, current_id);
    assert_eq!(audit(&f, old_id).await, old_audit);
    assert_eq!(audit(&f, current_id).await, current_audit);
}

#[tokio::test]
async fn stopped_backoff_delegation_requeues_the_selected_child_without_stopping_its_running_parent() {
    let f = Fixture::memory(&["parent", "unrelated"], &[]).await;
    let parent = f.start("parent").await;
    f.append("parent", "child").await;
    f.start("child").await;
    let child = fail_with_backoff(&f, "child").await;
    acknowledge(&f, 700).await;
    let parent_id = &parent.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let parent_audit = audit(&f, parent_id).await;
    let child_id = &child.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let child_audit = audit(&f, child_id).await;
    let detail = f.detail().await;
    f.repository.cancel_step(&f.owner, &f.execution_id, detail.execution.version,
        &parent.step.step_id, parent.step.version, StopScope::DelegatedWork, &stop_event()).await.unwrap();
    let cleanups = pending(&f).await;
    assert_eq!(cleanups.len(), 1); assert_eq!(&cleanups[0].attempt_id, child_id);
    assert_eq!(f.task("parent").await.step.status, "running");
    assert_eq!(audit(&f, parent_id).await, parent_audit);
    assert_eq!(audit(&f, child_id).await, child_audit);
    assert_eq!(f.task("child").await.step.status, "cancelled");
    assert_eq!(f.task("unrelated").await.step.status, "pending");
}

#[tokio::test]
async fn stopped_backoff_whole_cancel_delete_and_fatal_failure_rearm_only_unfinished_current_owners() {
    for mode in ["whole", "delete", "fatal"] {
        let f = Fixture::memory(&["backoff", "finished"], &[]).await;
        f.start("backoff").await;
        let selected = fail_with_backoff(&f, "backoff").await;
        f.start("finished").await; f.complete("finished").await;
        acknowledge(&f, 700).await;
        let id = &selected.current_attempt.as_ref().unwrap().attempt.attempt_id;
        let original = audit(&f, id).await;
        let finished = f.task("finished").await.current_attempt.unwrap().attempt.attempt_id;
        let finished_audit = audit(&f, &finished).await;
        let captured = f.detail().await;
        let lease = AgentExecutionLeaseToken::new("original-fatal-owner".into());
        let command = match mode {
            "delete" => NewAgentExecutionEvent { payload: serde_json::json!({"deleted":true}).to_string(), ..event(AgentExecutionEventKind::Deleted) },
            "fatal" => NewAgentExecutionEvent { payload: serde_json::json!({"status":"failed","lead_report_operation_id":null,"reason":"original scheduler failure"}).to_string(), ..event(AgentExecutionEventKind::StatusChanged) },
            _ => NewAgentExecutionEvent { payload: serde_json::json!({"status":"cancelled","reason":"cancelled_by_caller"}).to_string(), ..event(AgentExecutionEventKind::StatusChanged) },
        };
        match mode {
            "delete" => { assert!(f.repository.delete_execution(&f.owner, &f.execution_id, captured.execution.version, &command).await.unwrap()); }
            "fatal" => {
                f.repository.try_acquire_lease(&f.execution_id, captured.execution.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
                f.repository.fail_active_execution(&f.owner, &f.execution_id, captured.execution.version, &lease,
                    "original scheduler failure", &command).await.unwrap();
            }
            _ => { f.repository.cancel_execution(&f.owner, &f.execution_id, captured.execution.version, &command).await.unwrap(); }
        }
        assert_eq!(audit(&f, id).await, original, "{mode} preserves the original failed Attempt");
        assert_eq!(audit(&f, &finished).await, finished_audit);
        let cleanups = pending(&f).await;
        assert_eq!(cleanups.len(), 1, "{mode}"); assert_eq!(&cleanups[0].attempt_id, id);
        let session = selected.current_attempt.as_ref().unwrap().conversation_id.as_ref().unwrap();
        let facts = f.repository.read_task_retirement_facts(&f.owner, session, &f.execution_id, &selected.step.step_id, id)
            .await.unwrap().unwrap();
        assert_eq!(facts.detail.step.status, "cancelled");
        assert_eq!(facts.original_attempt.attempt.status, "failed");
        assert!(!facts.events.is_empty());
        if mode == "delete" { assert!(facts.execution.deleted_at.is_some()); }
        let links: Vec<(String, Option<i64>)> = nomifun_db::sqlx::query_as(
            "SELECT attempt_id,cleanup_completed_at FROM conversation_execution_links WHERE execution_id=? AND attempt_id=?")
            .bind(&f.execution_id).bind(&finished).fetch_all(f.db.pool()).await.unwrap();
        assert!(!links.is_empty() && links.iter().all(|link| link.1 == Some(700)), "finished result resources are not requeued by {mode}");
        acknowledge(&f, 900).await;
        match mode {
            "delete" => assert!(!f.repository.delete_execution(&f.owner, &f.execution_id, captured.execution.version, &command).await.unwrap()),
            "fatal" => assert!(f.repository.fail_active_execution(&f.owner, &f.execution_id, captured.execution.version, &lease,
                "original scheduler failure", &command).await.is_err()),
            _ => assert!(f.repository.cancel_execution(&f.owner, &f.execution_id, captured.execution.version, &command).await.is_err()),
        }
        assert!(pending(&f).await.is_empty(), "a rejected/replayed {mode} command cannot rearm the settled cleanup");
    }
}

#[tokio::test]
async fn stopped_backoff_preserves_a_completed_attempt_when_its_explicit_retry_is_stopped_before_invocation() {
    let f = Fixture::memory(&["selected"], &[]).await;
    f.start("selected").await; f.complete("selected").await;
    acknowledge(&f, 700).await;
    let completed = f.task("selected").await;
    let attempt = &completed.current_attempt.as_ref().unwrap().attempt.attempt_id;
    let original = audit(&f, attempt).await;
    let detail = f.detail().await;
    f.repository.reset_steps_for_retry(&f.owner, &f.execution_id, detail.execution.version,
        &[RetryAgentExecutionStep { step_id: completed.step.step_id.clone(), expected_step_version: completed.step.version }],
        &event(AgentExecutionEventKind::StepChanged)).await.unwrap();
    assert!(pending(&f).await.is_empty(), "ordinary explicit retry alone does not rearm old cleanup");
    let detail = f.detail().await; let current = f.task("selected").await;
    f.repository.cancel_step(&f.owner, &f.execution_id, detail.execution.version, &current.step.step_id,
        current.step.version, StopScope::Task, &stop_event()).await.unwrap();
    let cleanups = pending(&f).await;
    assert_eq!(cleanups.len(), 1); assert_eq!(&cleanups[0].attempt_id, attempt);
    assert_eq!(audit(&f, attempt).await, original, "the preserved completed business result is not rewritten as cancelled");
}

#[tokio::test]
async fn stopped_backoff_original_owner_is_revisited_when_a_later_attempt_finishes_or_its_queued_retry_is_stopped() {
    for outcome in ["completed", "failed", "stopped_queued"] {
        let f = Fixture::memory(&["selected"], &[]).await;
        f.start("selected").await;
        let original = fail_with_backoff(&f, "selected").await;
        acknowledge(&f, 700).await;
        let old = original.current_attempt.as_ref().unwrap();
        let old_audit = audit(&f, &old.attempt.attempt_id).await;
        let current_id = if outcome == "stopped_queued" {
            let current = f.task("selected").await;
            let queued = f.repository.create_attempt(&f.owner, &f.execution_id, &current.step.step_id, current.step.version, None,
                &CreateAgentExecutionAttemptParams { participant_id: current.step.assigned_participant_id, start_immediately: false,
                    trigger_reason: "explicit retry".into(), effective_config: "{}".into(), retry_after: None, runtime_state: None },
                &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
            let id = queued.current_attempt.as_ref().unwrap().attempt.attempt_id.clone();
            assert!(pending(&f).await.is_empty(), "queuing another Attempt does not end the Step's existing purpose");
            let detail = f.detail().await;
            f.repository.cancel_step(&f.owner, &f.execution_id, detail.execution.version, &queued.step.step_id,
                queued.step.version, StopScope::Task, &stop_event()).await.unwrap();
            id
        } else {
            let running = f.start("selected").await;
            assert!(pending(&f).await.is_empty(), "a live retry must preserve the old cleanup receipt");
            if outcome == "completed" { f.complete("selected").await; }
            else { fail(&f, "selected", false).await; }
            running.current_attempt.unwrap().attempt.attempt_id
        };
        assert_ne!(old.attempt.attempt_id, current_id);
        let cleanups = pending(&f).await;
        assert_eq!(cleanups.len(), if outcome == "stopped_queued" { 1 } else { 2 }, "{outcome}");
        let original_cleanup = cleanups.iter().find(|cleanup| cleanup.attempt_id == old.attempt.attempt_id).unwrap();
        assert_eq!(original_cleanup.conversation_id, old.conversation_id.as_ref().unwrap().as_str());
        assert_eq!(original_cleanup.target, ConversationCleanupTurnTarget::Exact {
            operation_id: format!("turn:user:{}:{}:{}:initial-turn", f.owner, old.conversation_id.as_ref().unwrap(), old.attempt.attempt_id),
        });
        let facts = f.repository.read_task_retirement_facts(&f.owner, &original_cleanup.conversation_id, &f.execution_id,
            &original.step.step_id, &old.attempt.attempt_id).await.unwrap().unwrap();
        assert_eq!(facts.original_attempt.attempt.attempt_id, old.attempt.attempt_id);
        assert_eq!(facts.detail.current_attempt.unwrap().attempt.attempt_id, current_id);
        assert!(matches!(facts.detail.step.status.as_str(), "completed" | "failed" | "cancelled"));
        assert_eq!(audit(&f, &old.attempt.attempt_id).await, old_audit);
        acknowledge(&f, 900).await;
        assert!(pending(&f).await.is_empty(), "reading and ACK cannot themselves re-arm {outcome} cleanup");
    }
}

#[tokio::test]
async fn stopped_backoff_replan_dependency_skip_and_verified_adoption_rearm_the_original_closed_scope() {
    for mode in ["replan", "skip", "adopt"] {
        let f = Fixture::memory(&["original", "retained"], &[]).await;
        f.start("original").await;
        let original = fail_with_backoff(&f, "original").await;
        f.start("retained").await;
        if mode == "skip" { fail(&f, "retained", false).await; }
        else { f.complete("retained").await; }
        acknowledge(&f, 700).await;
        let old = original.current_attempt.as_ref().unwrap();
        let old_audit = audit(&f, &old.attempt.attempt_id).await;
        let retained = f.task("retained").await;
        let retained_id = &retained.current_attempt.as_ref().unwrap().attempt.attempt_id;
        let retained_audit = audit(&f, retained_id).await;
        let before = f.detail().await;
        if mode == "adopt" {
            // This fixture tests the original trusted domain writer only;
            // canonical public-delivery validation belongs to the Engine's
            // existing adoption consumer, not this cleanup observation.
            f.repository.adopt_step_output(&f.owner, &f.execution_id, before.execution.version,
                &original.step.step_id, original.step.version, &nomifun_db::AdoptAgentExecutionStepOutputParams {
                    output_summary: "already verified public output".into(), output_files: "[]".into(), tokens: Some(7), runtime_state: None,
                }, &NewAgentExecutionEvent { payload: serde_json::json!({"change":"output_adopted"}).to_string(),
                    ..event(AgentExecutionEventKind::AttemptChanged) }).await.unwrap();
        } else {
            let replanned = f.repository.reconcile_plan(&f.owner, &f.execution_id, before.execution.version,
                &ReconcileAgentExecutionPlanParams { goal: None, adaptation_policy: None, decision_policy: None, delegation_policy: None,
                    keep_step_ids: if mode == "replan" { vec![retained.step.step_id.clone()] }
                        else { vec![original.step.step_id.clone(), retained.step.step_id.clone()] },
                    new_steps: vec![], new_participants: vec![], retire_participant_ids: vec![], execution_status: AgentExecutionStatus::Running,
                    new_dependencies: if mode == "skip" { vec![NewAgentExecutionStepDependency {
                        blocker_step_id: retained.step.step_id.clone(), blocked_step_id: original.step.step_id.clone() }] } else { vec![] },
                }, &NewAgentExecutionEvent { actor: AgentExecutionActor::user(&f.owner),
                    payload: serde_json::json!({"change":"replanned","plan_revision":9999,
                        "superseded_step_ids":[retained.step.step_id]}).to_string(),
                    ..event(AgentExecutionEventKind::PlanChanged) }).await.unwrap();
            let events = f.repository.list_events(&f.owner, &f.execution_id, 0, 100).await.unwrap();
            let payload: serde_json::Value = serde_json::from_str(&events.last().unwrap().payload).unwrap();
            assert_eq!(payload["plan_revision"], replanned.execution.plan_revision,
                "the writer authors the true revision, never caller-supplied cleanup authority");
            assert_eq!(payload["superseded_step_ids"], if mode == "replan" {
                serde_json::json!([original.step.step_id]) } else { serde_json::json!([]) });
            if mode == "skip" {
                assert!(pending(&f).await.is_empty(), "retaining a pending Step in a new plan does not end its purpose");
                let pending_step = f.task("original").await;
                f.repository.transition_step_status(&f.owner, &f.execution_id, &pending_step.step.step_id,
                    replanned.execution.version, pending_step.step.version, None, ExecutionStepStatus::Skipped,
                    &NewAgentExecutionEvent { payload: serde_json::json!({"status":"skipped","reason":"dependency_failed"}).to_string(),
                        ..event(AgentExecutionEventKind::StepChanged) }).await.unwrap();
            }
        }
        let cleanups = pending(&f).await;
        assert_eq!(cleanups.len(), if mode == "adopt" { 2 } else { 1 }, "{mode}");
        assert!(cleanups.iter().any(|cleanup| cleanup.attempt_id == old.attempt.attempt_id));
        assert!(!cleanups.iter().any(|cleanup| &cleanup.attempt_id == retained_id));
        assert_eq!(audit(&f, &old.attempt.attempt_id).await, old_audit);
        assert_eq!(audit(&f, retained_id).await, retained_audit);
        let facts = f.repository.read_task_retirement_facts(&f.owner, old.conversation_id.as_deref().unwrap(), &f.execution_id,
            &original.step.step_id, &old.attempt.attempt_id).await.unwrap().unwrap();
        if mode == "replan" {
            assert!(f.repository.get_step_detail(&f.owner, &f.execution_id, &original.step.step_id).await.unwrap().is_none());
            assert!(facts.detail.step.superseded_in_revision.is_some());
            assert!(facts.events.iter().any(|event| event.event_type == "plan_changed"));
        } else if mode == "skip" {
            assert_eq!(facts.detail.step.status, "skipped");
            assert!(facts.events.iter().any(|event| event.event_type == "step_changed"));
        } else {
            assert_eq!(facts.detail.step.status, "completed");
            assert_ne!(facts.detail.current_attempt.unwrap().attempt.attempt_id, old.attempt.attempt_id);
            assert!(facts.events.iter().any(|event| serde_json::from_str::<serde_json::Value>(&event.payload).unwrap()["change"] == "output_adopted"));
        }
        acknowledge(&f, 900).await;
        assert!(pending(&f).await.is_empty());
    }
}
