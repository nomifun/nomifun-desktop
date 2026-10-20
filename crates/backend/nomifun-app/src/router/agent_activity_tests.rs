use super::*;
use tower::ServiceExt;

#[path = "agent_activity_autowork_scope_tests.rs"]
mod autowork_scope_tests;

#[path = "source_browser_retirement_tests.rs"]
mod source_browser_retirement_tests;
#[path = "agent_activity_task_details_tests.rs"]
mod task_details_tests;

const TRUST: &str = "activity-lifecycle-regression";

#[path = "agent_activity_browser_fixture.rs"]
mod browser_fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual real-browser fixture; requires a fresh marked temporary root"]
async fn activity_browser_fixture() {
    browser_fixture::run().await;
}

async fn request(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(status.is_success(), "{method} {path}: {status}: {body}");
    body["data"].clone()
}

async fn fixture() -> (
    tempfile::TempDir,
    crate::services::AppServices,
    axum::Router,
    NomiCoreAgentApiState,
    AgentSessionId,
    PrincipalRef,
) {
    let root = tempfile::tempdir().unwrap();
    let (services, router, state, session, principal) =
        fixture_in(root.path(), "http://127.0.0.1:9/v1").await;
    (root, services, router, state, session, principal)
}

async fn fixture_in(
    root: &std::path::Path,
    provider_url: &str,
) -> (
    crate::services::AppServices,
    axum::Router,
    NomiCoreAgentApiState,
    AgentSessionId,
    PrincipalRef,
) {
    // macOS's default temporary directory passes through /var, a symlink.
    // Resolve this fixture-owned directory before the real storage guard;
    // production admission must still reject symbolic storage ancestors.
    let root = root.canonicalize().expect("the isolated activity fixture root exists");
    let config = crate::AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(TRUST.into()),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path())
        .await
        .unwrap();
    let services = crate::services::AppServices::from_config(database, &config)
        .await
        .unwrap();
    let (states, _) = super::super::super::state::try_build_module_states(&services)
        .await
        .unwrap();
    let state = states.nomi_core_agent_api.clone();
    let router = super::super::super::create_router_with_states(&services, states);
    let provider=request(&router,"POST","/api/providers",json!({"platform":"custom","name":"activity fixture","base_url":provider_url,
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"activity-fixture","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default"}]}})).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"activity-fixture"});
    let preset = request(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({"reuse_existing":false,"display_name":"activity fixture","model":model}),
    )
    .await;
    let created = request(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset["preset"]["preset_id"],"model":model,"title":"Lifecycle input Alpha"}),
    )
    .await;
    let session = AgentSessionId::from(created["agent_session_id"].as_str().unwrap());
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: services.authoritative_user_id.as_ref().into(),
    };
    (services, router, state, session, principal)
}

#[tokio::test]
async fn activity_snapshot_includes_unclaimed_input_and_stop_never_invents_a_turn_or_restarts_it() {
    let (_root, services, router, state, session, principal) = fixture().await;
    let accepted = state
        .session_owner
        .canonical
        .accept_session_input(
            &principal,
            &session,
            "saved-input",
            json!({"content":"saved, never sent","files":[],"inject_skills":[],"hidden":false}),
        )
        .await
        .unwrap();
    let input = accepted.record.unwrap().event_id;
    let before = state
        .session_owner
        .canonical
        .store()
        .current_cursor(&session)
        .await
        .unwrap();
    let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    assert!(
        activities["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["target"]["message_id"] == input.as_ref())
    );
    let snapshot = request(
        &router,
        "GET",
        "/api/agent-activities/stop-targets",
        Value::Null,
    )
    .await;
    let target = snapshot["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["message_id"] == input.as_ref())
        .unwrap()
        .clone();
    assert_eq!(target["kind"], "session_input");
    assert!(target.get("operation_id").is_none());
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .current_cursor(&session)
            .await
            .unwrap(),
        before,
        "discovery and snapshot are read-only"
    );
    let stopped = request(
        &router,
        "POST",
        "/api/agent-activities/stop",
        json!({"target":target,"idempotency_key":"cancel-saved-input"}),
    )
    .await;
    assert_eq!(stopped["accepted"], true);
    assert_eq!(stopped["execution_stopped"], true);
    assert!(
        state
            .session_owner
            .canonical
            .store()
            .session_input_claim(&session, &input)
            .await
            .unwrap()
            .is_none()
    );
    let delivery = state
        .session_owner
        .canonical
        .store()
        .session_input_delivery(&session, &input)
        .await
        .unwrap();
    assert_eq!(delivery["reason"], "CANCELLED_BY_OWNER");
    let again = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    assert!(
        !again["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["target"]["message_id"] == input.as_ref())
    );
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn activity_old_exact_turn_stop_cannot_cancel_a_successor() {
    let (_root, services, router, state, session, principal) = fixture().await;
    let original_key = format!("original-{}-控制", "x".repeat(50));
    let old = state
        .session_owner
        .canonical
        .start_turn(
            &principal,
            &session,
            &original_key,
            json!({"content":"original","files":[],"inject_skills":[]}),
        )
        .await
        .unwrap();
    assert!(old.operation_id.as_ref().len() > nomifun_common::MAX_IDEMPOTENCY_KEY_LEN);
    assert!(old.operation_id.as_ref().contains("控制"));
    let snapshot = request(
        &router,
        "GET",
        "/api/agent-activities/stop-targets",
        Value::Null,
    )
    .await;
    let target = snapshot["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["operation_id"] == old.operation_id.as_ref())
        .unwrap()
        .clone();
    request(&router, "POST", "/api/agent-activities/stop",
        json!({"target":target,"idempotency_key":"original-ended"})).await;
    assert_eq!(state.session_owner.canonical.store().read_turn_receipt(&session, &old.operation_id)
        .await.unwrap().status, TurnReceiptStatus::Cancelled,
        "a long original Unicode operation must be directly stoppable through the public host API");
    let next = state
        .session_owner
        .canonical
        .start_turn(
            &principal,
            &session,
            "successor",
            json!({"content":"successor","files":[],"inject_skills":[]}),
        )
        .await
        .unwrap();
    request(
        &router,
        "POST",
        "/api/agent-activities/stop",
        json!({"target":target,"idempotency_key":"stale-snapshot-stop"}),
    )
    .await;
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .head(&session)
            .await
            .unwrap()
            .active_turn_id
            .as_deref(),
        Some(next.operation_id.as_ref())
    );
    state
        .session_owner
        .canonical
        .cancel_exact_turn(&principal, &session, "test-cleanup", &next.operation_id)
        .await
        .unwrap();
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn activity_keeps_an_attached_cron_occurrence_discoverable_after_its_job_was_deleted() {
    let (_root, services, router, _state, session, _principal) = fixture().await;
    let job = uuid::Uuid::now_v7().to_string();
    let run = uuid::Uuid::now_v7().to_string();
    // A fault that loses configuration must not erase an admitted occurrence.
    // Its already-verified attached Session retains the original owner fact.
    nomifun_db::sqlx::query("INSERT INTO cron_run_reservations(cron_job_run_id,cron_job_id,trigger_kind,operation_key,request_fingerprint,status,conversation_id,created_at_ms,updated_at_ms,settled_at_ms) \
        VALUES(?,?,'run_now',?,?,'reserved',?,1,1,NULL)")
        .bind(&run).bind(&job).bind(format!("run-now:{run}")).bind(format!("request:{run}")).bind(session.as_ref())
        .execute(services.database.pool()).await.unwrap();
    let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    assert!(
        activities["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["target"]["kind"] == "cron_run"
                && item["target"]["cron_job_run_id"] == run)
    );
    let snapshot = request(
        &router,
        "GET",
        "/api/agent-activities/stop-targets",
        Value::Null,
    )
    .await;
    assert!(
        snapshot["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["kind"] == "cron_run" && target["cron_job_run_id"] == run)
    );
    nomifun_db::sqlx::query("UPDATE cron_run_reservations SET status='input_required',settled_at_ms=2 WHERE cron_job_run_id=?")
        .bind(&run).execute(services.database.pool()).await.unwrap();
    let ended = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    assert!(
        !ended["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["target"]["cron_job_run_id"] == run),
        "an input handoff is a terminal occurrence, not a stoppable native run"
    );
    let ended_snapshot = request(
        &router,
        "GET",
        "/api/agent-activities/stop-targets",
        Value::Null,
    )
    .await;
    assert!(
        !ended_snapshot["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["cron_job_run_id"] == run)
    );
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn activity_stop_signals_the_exact_unclaimed_preparation_and_keeps_it_visible_until_owner_settles()
 {
    let (_root, services, router, state, session, principal) = fixture().await;
    let accepted = state
        .session_owner
        .canonical
        .accept_session_input(
            &principal,
            &session,
            "cold-input",
            json!({"content":"cold original input","files":[],"inject_skills":[],"hidden":false}),
        )
        .await
        .unwrap();
    let input = accepted.record.unwrap().event_id;
    let original = state
        .session_owner
        .hold_input_preparation_for_test(&session, &input)
        .unwrap();
    let stopped=request(&router,"POST","/api/agent-activities/stop",json!({"target":{"kind":"session_input","agent_session_id":session,"message_id":input},"idempotency_key":"stop-cold-original"})).await;
    assert_eq!(stopped["accepted"], true);
    assert_eq!(
        stopped["execution_stopped"], false,
        "signalling a preparation is not joining its native work"
    );
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .session_input_delivery(&session, &input)
            .await
            .unwrap()["reason"],
        "CANCELLED_BY_OWNER"
    );
    assert_eq!(
        state
            .session_owner
            .input_preparation_observations(&session)
            .unwrap(),
        vec![(input.clone(), true)]
    );
    let cursor = state
        .session_owner
        .canonical
        .store()
        .current_cursor(&session)
        .await
        .unwrap();
    let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    let row = activities["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["target"]["message_id"] == input.as_ref())
        .unwrap();
    assert_eq!(row["state"], "stopping");
    assert_eq!(row["stop_accepted"], true);
    assert_eq!(row["execution_stopped"], false);
    let snapshot = request(
        &router,
        "GET",
        "/api/agent-activities/stop-targets",
        Value::Null,
    )
    .await;
    assert!(
        snapshot["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["message_id"] == input.as_ref()),
        "an already-signalled owner still belongs to the finite Stop snapshot"
    );
    drop(original);
    let finished = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    assert!(
        !finished["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["target"]["message_id"] == input.as_ref())
    );
    assert!(
        state
            .session_owner
            .canonical
            .store()
            .session_input_claim(&session, &input)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .current_cursor(&session)
            .await
            .unwrap(),
        cursor,
        "inventory/refresh never starts a Turn or reruns preparation"
    );
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn activity_discovers_a_plan_without_attempts_and_stops_only_the_explicit_collaboration_target()
 {
    use nomifun_db::IAgentExecutionRepository;
    let (_root, services, router, state, session, principal) = fixture().await;
    let projection = state
        .session_owner
        .get_session(&principal.principal_id, session.as_ref())
        .await
        .unwrap();
    let snapshot = projection.agent_snapshot.as_ref().unwrap();
    let model = snapshot.resolved_model.as_ref().unwrap();
    // Match the existing template participant resolver: an omitted concrete
    // Agent identity denotes the built-in Nomi metadata anchor, whose backend
    // column is not a second snapshot identity to infer with a string query.
    let source_agent_id=snapshot.resolved_agent_id.clone()
        .unwrap_or_else(||nomifun_common::NOMI_AGENT_ID.to_owned());
    let repository =
        nomifun_db::SqliteAgentExecutionRepository::new(services.database.pool().clone());
    let created = repository
        .create_execution_with_participants(
            &principal.principal_id,
            &nomifun_db::CreateAgentExecutionParams {
                goal: "planning without an Attempt".into(),
                status: nomifun_common::AgentExecutionStatus::Planning,
                adaptation_policy: nomifun_common::AdaptationPolicy::Fixed,
                decision_policy: nomifun_common::DecisionPolicy::Automatic,
                delegation_policy: nomifun_common::DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: json!({"mode":"automatic"}).to_string(),
            },
            &[nomifun_db::NewAgentExecutionParticipant {
                participant_id: uuid::Uuid::now_v7().to_string(),
                source_agent_id,
                preset_id: Some(snapshot.preset_id.clone()),
                preset_revision: Some(snapshot.preset_revision),
                agent_snapshot: Some(serde_json::to_string(snapshot).unwrap()),
                provider_id: Some(model.provider_id.to_string()),
                model: Some(model.model.clone()),
                role: Some("builder".into()),
                capability: None,
                constraints: None,
                description: None,
                system_prompt: None,
                enabled_skills: serde_json::to_string(&snapshot.included_skills).unwrap(),
                disabled_builtin_skills: serde_json::to_string(&snapshot.excluded_auto_skills).unwrap(),
                sort_order: 0,
            }],
            &nomifun_db::NewAgentExecutionEvent {
                event_type: nomifun_common::AgentExecutionEventKind::Created,
                step_id: None,
                attempt_id: None,
                actor: nomifun_common::AgentExecutionActor::user(&principal.principal_id),
                payload: "{}".into(),
            },
        )
        .await
        .unwrap();
    let execution = created.execution_id;
    let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    let row = activities["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["target"]["kind"] == "execution_group"
                && item["target"]["execution_id"] == execution
        })
        .unwrap();
    assert_eq!(row["kind"], "collaboration");
    assert_eq!(row["state"], "planning");
    let target = row["target"].clone();
    let stopped = request(
        &router,
        "POST",
        "/api/agent-activities/stop",
        json!({"target":target,"idempotency_key":"stop-original-plan"}),
    )
    .await;
    assert_eq!(stopped["accepted"], true);
    assert_eq!(
        stopped["execution_stopped"], false,
        "aggregate cancellation alone is not native shutdown proof"
    );
    let status: String =
        nomifun_db::sqlx::query_scalar("SELECT status FROM agent_executions WHERE execution_id=?")
            .bind(&execution)
            .fetch_one(services.database.pool())
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    let retry = request(
        &router,
        "POST",
        "/api/agent-activities/stop",
        json!({"target":target,"idempotency_key":"stop-original-plan"}),
    )
    .await;
    assert_eq!(
        retry["accepted"], true,
        "a lost acknowledgement can observe the absorbing original cancellation"
    );
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn activity_ssh_stop_replays_original_canonical_receipts_without_a_live_pool_or_successor_lookup()
 {
    use nomifun_agent_contracts::{
        SessionSshLinkCloseResult, SessionSshLinkCloseV1, SessionSshLinkClosedV1,
    };
    let (_root, services, router, state, session, principal) = fixture().await;
    let store = state.session_owner.canonical.store();
    let mut targets = Vec::new();
    for result in [Some(true), Some(false), None] {
        let close = SessionSshLinkCloseV1 {
            agent_session_id: session.clone(),
            owner_ref: principal.clone(),
            ssh_host_id: uuid::Uuid::now_v7().to_string(),
            pool_instance_id: uuid::Uuid::now_v7().to_string(),
            instance_id: 1,
        };
        store
            .record_session_ssh_link_close_requested(&principal, close.clone())
            .await
            .unwrap();
        if let Some(reaped) = result {
            store
                .record_session_ssh_link_closed(
                    &principal,
                    SessionSshLinkClosedV1 {
                        close: close.clone(),
                        result: if reaped {
                            SessionSshLinkCloseResult::Reaped {
                                detail: "original native close proved exit".into(),
                            }
                        } else {
                            SessionSshLinkCloseResult::Lost {
                                detail: "original connection exit remains unknown".into(),
                            }
                        },
                    },
                )
                .await
                .unwrap();
        }
        let target = json!({"kind":"ssh_link","agent_session_id":session,"ssh_host_id":close.ssh_host_id,"pool_instance_id":close.pool_instance_id,"instance_id":close.instance_id});
        targets.push((target, result == Some(true)));
    }
    let cursor = store.current_cursor(&session).await.unwrap();
    for (index, (target, reaped)) in targets.iter().enumerate() {
        for _ in 0..2 {
            let stopped = request(
                &router,
                "POST",
                "/api/agent-activities/stop",
                json!({"target":target,"idempotency_key":format!("lost-ssh-response-{index}")}),
            )
            .await;
            assert_eq!(stopped["accepted"], true);
            assert_eq!(
                stopped["execution_stopped"], *reaped,
                "only the original Reaped fact proves exit; an absent old pool never does"
            );
        }
    }
    assert!(
        state
            .ssh_pool
            .resource_instances(&principal.principal_id)
            .is_empty(),
        "receipt replay must never dial a replacement connection"
    );
    assert_eq!(
        store.current_cursor(&session).await.unwrap(),
        cursor,
        "read/replay must not append a second close history"
    );
    let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
    let resource = activities["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "resources" && item["agent_session_id"] == session.as_ref())
        .unwrap();
    for (target, reaped) in &targets {
        let row = resource["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|resource| resource["target"] == *target)
            .unwrap();
        assert_eq!(
            row["state"],
            if *reaped { "released" } else { "unconfirmed" }
        );
    }
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn all_stop_process_cutoff_keeps_background_creator_and_excludes_a_later_turn() {
    let (_root, services, _router, state, session, principal) = fixture().await;
    let original = state
        .session_owner
        .canonical
        .start_turn(
            &principal,
            &session,
            "background-creator",
            json!({"content":"original process creator","files":[],"inject_skills":[]}),
        )
        .await
        .unwrap();
    state
        .session_owner
        .canonical
        .cancel_exact_turn(
            &principal,
            &session,
            "creator-ended",
            &original.operation_id,
        )
        .await
        .unwrap();
    let cutoff = state
        .session_owner
        .canonical
        .store()
        .head(&session)
        .await
        .unwrap()
        .last_seq as i64;
    let successor = state
        .session_owner
        .canonical
        .start_turn(
            &principal,
            &session,
            "later-creator",
            json!({"content":"later process creator","files":[],"inject_skills":[]}),
        )
        .await
        .unwrap();
    let observation = |creator: &OperationId, stopped| {
        super::super::super::engine_process_host::ProcessInstanceObservation {
            owner_generation: uuid::Uuid::now_v7().to_string(),
            process_id: uuid::Uuid::now_v7().to_string(),
            creator_turn_id: creator.as_ref().into(),
            creator_call_id: "original-host-call".into(),
            stop_requested: false,
            stop_in_flight: false,
            execution_stopped: stopped,
            cleanup_unconfirmed: false,
            display_name: Some("synthetic owner observation".into()),
        }
    };
    let original_process = observation(&original.operation_id, false);
    let mut tx = state.session_owner.pool.begin().await.unwrap();
    let captured =
        process_target_at_cutoff_tx(&mut tx, session.as_ref(), cutoff, original_process.clone())
            .await
            .unwrap()
            .unwrap();
    assert!(
        matches!(captured,Target::ProcessInstance {owner_generation,process_id,creator_turn_id,creator_call_id,..}
        if owner_generation==original_process.owner_generation&&process_id==original_process.process_id&&creator_turn_id==original.operation_id.as_ref()&&creator_call_id=="original-host-call"),
        "the original exact background capability remains stoppable after its parent Turn ended"
    );
    assert!(
        process_target_at_cutoff_tx(
            &mut tx,
            session.as_ref(),
            cutoff,
            observation(&successor.operation_id, false)
        )
        .await
        .unwrap()
        .is_none(),
        "later accepted work is outside the finite Stop snapshot"
    );
    assert!(
        process_target_at_cutoff_tx(
            &mut tx,
            session.as_ref(),
            cutoff,
            observation(&original.operation_id, true)
        )
        .await
        .unwrap()
        .is_none(),
        "confirmed ended native work needs no Stop command"
    );
    tx.commit().await.unwrap();
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .head(&session)
            .await
            .unwrap()
            .active_turn_id,
        Some(successor.operation_id.as_ref().to_owned()),
        "capture never cancels or resumes work"
    );
    state
        .session_owner
        .canonical
        .cancel_exact_turn(&principal, &session, "test-end", &successor.operation_id)
        .await
        .unwrap();
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn automation_cleanup_stays_discoverable_and_preserves_active_source_and_successor() {
    for original_status in ["cancelled", "running"] {
        let (_root, services, router, state, session, principal) = fixture().await;
        let execution = uuid::Uuid::now_v7().to_string();
        let step = uuid::Uuid::now_v7().to_string();
        let attempt = uuid::Uuid::now_v7().to_string();
        let requirement = uuid::Uuid::now_v7().to_string();
        let original = state
            .session_owner
            .canonical
            .start_turn(
                &principal,
                &session,
                &format!("{attempt}:initial-turn"),
                json!({"content":"old automation task","files":[],"inject_skills":[]}),
            )
            .await
            .unwrap();
        state
            .session_owner
            .canonical
            .cancel_exact_turn(
                &principal,
                &session,
                "old-automation-ended",
                &original.operation_id,
            )
            .await
            .unwrap();
        let successor = state
            .session_owner
            .canonical
            .start_turn(
                &principal,
                &session,
                "healthy-successor",
                json!({"content":"keep the healthy successor","files":[],"inject_skills":[]}),
            )
            .await
            .unwrap();
        let initial = json!({"mode":"automation","source":{"requirement_id":requirement,"claim_generation":1,"operation_id":"autowork:old-cleanup"},"plan":{"steps":[]}});
        nomifun_db::sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,created_at,updated_at) \
        VALUES(?,?,'old automation cleanup',?,'automatic','fixed','automatic','automatic',?,1,1)")
        .bind(&execution).bind(&principal.principal_id).bind(original_status).bind(initial.to_string()).execute(services.database.pool()).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
            .bind(session.as_ref()).bind(&execution).execute(services.database.pool()).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,introduced_in_revision,created_at,updated_at) \
        VALUES(?,?,'old automation task','retain exact cleanup','agent','cancelled',0,1,1)")
        .bind(&step).bind(&execution).execute(services.database.pool()).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,created_at,updated_at) \
        VALUES(?,?,?,0,'cancelled','initial',1,1)").bind(&attempt).bind(&execution).bind(&step).execute(services.database.pool()).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) \
        VALUES(?,?,'automation',?,?,0,1,1)").bind(session.as_ref()).bind(&execution).bind(&step).bind(&attempt).execute(services.database.pool()).await.unwrap();
        let activities = request(&router, "GET", "/api/agent-activities", Value::Null).await;
        let row = activities["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == format!("execution:{attempt}"))
            .unwrap();
        assert_eq!(row["kind"], "auto_work_run");
        assert_eq!(row["can_stop"], false);
        assert_eq!(row["can_retry_cleanup"], true);
        let stopped=request(&router,"POST","/api/agent-activities/stop",json!({"target":row["target"],"idempotency_key":"retry-original-automation-cleanup","retry_cleanup":true})).await;
        assert_eq!(stopped["accepted"], true);
        tokio::time::timeout(std::time::Duration::from_secs(10),async {loop {
        let acknowledged:bool=nomifun_db::sqlx::query_scalar("SELECT cleanup_completed_at IS NOT NULL FROM conversation_execution_links WHERE execution_id=? AND attempt_id=? AND relation='automation'")
            .bind(&execution).bind(&attempt).fetch_one(services.database.pool()).await.unwrap();
        if acknowledged {break;}
        tokio::task::yield_now().await;
    }}).await.expect("only the original exact-source outbox acknowledgement is awaited");
        let status: String = nomifun_db::sqlx::query_scalar(
            "SELECT status FROM agent_executions WHERE execution_id=?",
        )
        .bind(&execution)
        .fetch_one(services.database.pool())
        .await
        .unwrap();
        assert_eq!(
            status, original_status,
            "cleanup-only retry must not cancel a healthy active automation source"
        );
        assert_eq!(
            state
                .session_owner
                .canonical
                .store()
                .head(&session)
                .await
                .unwrap()
                .active_turn_id,
            Some(successor.operation_id.as_ref().to_owned()),
            "retrying inactive AutoWork cleanup never selects the Session successor"
        );
        state
            .session_owner
            .canonical
            .cancel_exact_turn(
                &principal,
                &session,
                "test-cleanup-successor",
                &successor.operation_id,
            )
            .await
            .unwrap();
        services.shutdown_nomi_core_host().await.unwrap();
        services.database.close().await;
    }
}

#[tokio::test]
async fn cancelled_planning_handle_projection_checks_original_owner_and_admitted_version() {
    let (_root, services, _router, state, session, principal) = fixture().await;
    let own = uuid::Uuid::now_v7().to_string();
    let foreign = uuid::Uuid::now_v7().to_string();
    let future = uuid::Uuid::now_v7().to_string();
    for (id, owner) in [
        (&own, principal.principal_id.clone()),
        (&foreign, uuid::Uuid::now_v7().to_string()),
        (&future, principal.principal_id.clone()),
    ] {
        nomifun_db::sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,version,created_at,updated_at) \
            VALUES(?,?,'original planning owner','cancelled','automatic','fixed','automatic','automatic',?,2,1,1)")
            .bind(id).bind(owner).bind(json!({"mode":"automatic"}).to_string()).execute(services.database.pool()).await.unwrap();
    }
    let observations = vec![
        (own.clone(), 1, false),
        (own.clone(), 2, true),
        (foreign, 0, true),
        (future, 3, true),
    ];
    let cursor = state
        .session_owner
        .canonical
        .store()
        .current_cursor(&session)
        .await
        .unwrap();
    let candidates = planning_candidates(
        services.database.pool(),
        &principal.principal_id,
        "",
        50,
        &observations,
    )
    .await
    .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].execution_id, own);
    assert_eq!(
        candidates[0].status, "cancelled",
        "duplicate original owners deduplicate while preserving the durable aggregate Stop"
    );
    assert!(
        planning_candidates(
            services.database.pool(),
            &principal.principal_id,
            "",
            50,
            &[]
        )
        .await
        .unwrap()
        .is_empty(),
        "a terminal aggregate is no permanent pin once the original planning handles leave"
    );
    assert_eq!(
        state
            .session_owner
            .canonical
            .store()
            .current_cursor(&session)
            .await
            .unwrap(),
        cursor,
        "the planning view never starts a replacement request"
    );
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}
