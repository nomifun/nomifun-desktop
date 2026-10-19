//! Owner-scoped activity discovery over the original Session/domain owners.
//! No row here is persisted, and reading never acquires or resumes a Runtime.
use super::*;
use nomifun_agent_contracts::EventId;
use nomifun_agent_session::TurnReceiptStatus;
#[cfg(test)]
#[path = "agent_activity_tests.rs"]
mod tests;
use nomifun_api_types::{
    AgentActivitiesResponse, AgentActivity, AgentActivityContinuation as Continuation,
    AgentActivityKind as Kind, AgentActivityResource as Resource,
    AgentActivityResourceKind as ResourceKind, AgentActivityResourceState as ResourceState,
    AgentActivityState as ActivityState, AgentActivityTarget as Target,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivityQuery {
    after_cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(sqlx::FromRow)]
struct SessionCandidate {
    session_id: String,
    state: String,
    head_status: Option<String>,
    active_turn_id: Option<String>,
    head_seq: i64,
    process_hint: bool,
    remote_opening: bool,
}

#[derive(sqlx::FromRow)]
struct AttemptCandidate {
    execution_id: String,
    step_id: String,
    attempt_id: String,
    title: String,
    attempt_status: String,
    execution_status: String,
    execution_version: i64,
    step_version: i64,
    runtime_state: Option<String>,
    conversation_id: Option<String>,
    cleanup_pending: bool,
    updated_at: i64,
    initial_plan_input: String,
}

#[derive(sqlx::FromRow)]
struct CronCandidate {
    cron_job_id: String,
    cron_job_run_id: String,
    title: String,
    status: String,
    conversation_id: Option<String>,
    updated_at_ms: i64,
    stop_accepted: bool,
}

#[derive(sqlx::FromRow)]
struct AutoWorkCandidate {
    requirement_id: String,
    title: String,
    claim_generation: i64,
    target_id: String,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct QueuedStepCandidate {
    execution_id: String,
    step_id: String,
    title: String,
    execution_version: i64,
    step_version: i64,
    updated_at: i64,
    initial_plan_input:String,
}

async fn automation_activity_control(state:&NomiCoreAgentApiState,owner:&str,execution:&str,input:&str,
    controls:&mut HashMap<String,nomifun_api_types::AutomationExecutionControl>)
    ->Result<Option<nomifun_api_types::AutomationExecutionControl>,AppError> {
    let initial:Value=serde_json::from_str(input).map_err(|error|AppError::Internal(error.to_string()))?;
    if initial.get("mode").and_then(Value::as_str)!=Some("automation") {return Ok(None);}
    if let Some(control)=controls.get(execution) {return Ok(Some(control.clone()));}
    // The same pure projection as ordinary Execution detail. Original audit
    // rows remain readable for retained cleanup after soft deletion, without
    // acquiring a live Engine/Runtime or selecting the lead Session's latest
    // Turn. These minimal rows never authorize a mutation.
    let row:nomifun_db::models::AgentExecutionRow=sqlx::query_as("SELECT * FROM agent_executions WHERE execution_id=? AND user_id=?")
        .bind(execution).bind(owner).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?
        .ok_or_else(||AppError::NotFound("original automation audit owner is unavailable".into()))?;
    let lead=sqlx::query_scalar("SELECT conversation_id FROM conversation_execution_links WHERE execution_id=? AND relation='lead' ORDER BY created_at DESC,id DESC LIMIT 1")
        .bind(execution).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let roots=sqlx::query_as("SELECT * FROM agent_execution_steps WHERE execution_id=? AND introduced_in_revision=(SELECT MIN(introduced_in_revision) FROM agent_execution_steps WHERE execution_id=?)")
        .bind(execution).bind(execution).fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let facts=nomifun_db::models::AgentExecutionDetailRows {execution:row,lead_conversation_id:lead,steps:roots,
        participants:vec![],dependencies:vec![],attempts:vec![]};
    let control=nomifun_agent_execution::automation_control_from_rows(&facts)?
        .ok_or_else(||AppError::Internal("automation control source is unavailable".into()))?;
    controls.insert(execution.to_owned(),control.clone());Ok(Some(control))
}

#[derive(sqlx::FromRow)]
struct PlanningCandidate {
    execution_id: String,
    goal: String,
    status: String,
    version: i64,
    updated_at: i64,
}

async fn planning_candidates(
    pool: &sqlx::SqlitePool,
    owner: &str,
    after: &str,
    limit: usize,
    observations: &[(String, i64, bool)],
) -> Result<Vec<PlanningCandidate>, AppError> {
    let observations = serde_json::to_string(observations)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let candidates:Vec<PlanningCandidate>=sqlx::query_as("SELECT execution_id,goal,status,version,updated_at FROM agent_executions WHERE user_id=? \
        AND (status='planning' OR EXISTS(SELECT 1 FROM json_each(?) observed \
          WHERE json_extract(observed.value,'$[0]')=execution_id AND json_extract(observed.value,'$[1]') BETWEEN 0 AND version)) \
        AND COALESCE(json_extract(initial_plan_input,'$.mode'),'')<>'automation' \
        AND 'planning:'||execution_id>? ORDER BY execution_id LIMIT ?")
        .bind(owner).bind(observations).bind(after).bind((limit+1)as i64).fetch_all(pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    Ok(candidates)
}

fn unknown_resources() -> Vec<Resource> {
    [
        ResourceKind::Runtime,
        ResourceKind::Processes,
        ResourceKind::Browser,
        ResourceKind::Ssh,
        ResourceKind::KnowledgeWriteback,
        ResourceKind::Effects,
    ]
    .into_iter()
    .map(|kind| Resource {
        kind,
        state: ResourceState::Unconfirmed,
        detail: None,
        display_name: None,
        target: None,
    })
    .collect()
}

fn activity_state(status: &str) -> ActivityState {
    match status {
        "queued" | "pending" | "accepted" => ActivityState::Queued,
        "planning" => ActivityState::Planning,
        "running" => ActivityState::Running,
        "waiting_input" | "waiting" => ActivityState::Waiting,
        "paused" => ActivityState::Paused,
        "reconciliation" | "interrupted" => ActivityState::Recovering,
        "completed" | "done" => ActivityState::Completed,
        "failed" => ActivityState::Failed,
        "cancelled" => ActivityState::Cancelled,
        _ => ActivityState::Unconfirmed,
    }
}

fn terminal_turn(status: TurnReceiptStatus) -> bool {
    matches!(
        status,
        TurnReceiptStatus::Completed | TurnReceiptStatus::Failed | TurnReceiptStatus::Cancelled
    )
}

async fn operation_stopped(
    owner: &NomiCoreSessionOwner,
    session: &AgentSessionId,
    operation: &OperationId,
) -> Result<bool, AppError> {
    let progress = owner
        .canonical
        .store()
        .read_turn_progress(session, operation)
        .await
        .map_err(agent_session_store_error)?;
    let carrier = owner
        .runtime_sessions
        .observe_runtime_carrier(session.as_ref());
    Ok(terminal_turn(progress.status)
        && carrier.supported
        && carrier.active_generation.is_none()
        && carrier.constructing_generation.is_none()
        && !carrier.uncertain_acquisition
        && !carrier.teardown_pending)
}

async fn queued_inputs(
    state: &NomiCoreAgentApiState,
    owner: &PrincipalRef,
    session: &AgentSessionId,
    after: &str,
    limit: usize,
) -> Result<Vec<nomifun_agent_session::PendingSessionInput>, AppError> {
    let namespace = nomifun_agent_session::session_input_namespace(owner, session);
    // Discovery selects IDs only. The original Store validates each accepted
    // fact and derives its current delivery, including repairable preparation
    // blocks; no projection or text can authorize a command.
    let ids:Vec<String>=sqlx::query_scalar(
        "SELECT input.event_id FROM agent_events input WHERE input.session_id=? AND input.kind='message/user-accepted' \
         AND input.kind_version=1 AND input.producer_id='session_api' AND substr(input.idempotency_key,1,length(?))=? \
         AND 'session:'||input.session_id||':input:'||printf('%020d',input.seq)>? \
         AND input.seq>COALESCE((SELECT MAX(clear.seq) FROM agent_events clear WHERE clear.session_id=input.session_id AND clear.kind='context/cleared'),0) \
         AND NOT EXISTS(SELECT 1 FROM agent_turns turn WHERE turn.session_id=input.session_id AND turn.source_message_id=input.event_id) \
         AND NOT EXISTS(SELECT 1 FROM agent_events blocked WHERE blocked.session_id=input.session_id \
            AND blocked.event_id='session-input-blocked:'||input.event_id||':cancelled_by_owner') \
         AND NOT EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads payload ON payload.session_id=claim.session_id AND payload.payload_id=claim.payload_id \
            WHERE claim.session_id=input.session_id AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
              AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(payload.body AS TEXT),'$.value.input_event_id'))=input.event_id) \
         ORDER BY input.seq LIMIT ?",
    ).bind(session.as_ref()).bind(&namespace).bind(&namespace).bind(after).bind((limit+1)as i64)
        .fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let mut pending = Vec::new();
    for id in ids {
        let id = EventId::from(id);
        let delivery = state
            .session_owner
            .canonical
            .store()
            .session_input_delivery(session, &id)
            .await
            .map_err(agent_session_store_error)?;
        if matches!(
            delivery.get("state").and_then(Value::as_str),
            Some("pending" | "blocked")
        ) {
            pending.push(
                state
                    .session_owner
                    .canonical
                    .store()
                    .session_input(session, &id)
                    .await
                    .map_err(agent_session_store_error)?,
            );
        }
    }
    Ok(pending)
}

async fn observe_resources(
    state: &NomiCoreAgentApiState,
    owner: &str,
    session: &AgentSessionId,
    head_seq: u64,
    operation: Option<&OperationId>,
    live: bool,
) -> Result<Vec<Resource>, AppError> {
    if let Some(operation) = operation.filter(|_| live) {
        let latest:Option<String>=sqlx::query_scalar("SELECT turn.operation_id FROM agent_turns turn JOIN agent_events started \
            ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id AND started.kind='turn/started' \
            WHERE turn.session_id=? ORDER BY started.seq DESC LIMIT 1")
            .bind(session.as_ref()).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if latest.as_deref() != Some(operation.as_ref()) {
            return Ok(unknown_resources());
        }
    }
    let mut resources = unknown_resources();
    let carrier = state
        .session_owner
        .runtime_sessions
        .observe_runtime_carrier(session.as_ref());
    resources[0].state = if !carrier.supported {
        ResourceState::Unavailable
    } else if carrier.teardown_pending || carrier.stop_requested {
        ResourceState::Releasing
    } else if carrier.uncertain_acquisition {
        ResourceState::Unconfirmed
    } else if carrier.registered {
        ResourceState::Active
    } else {
        ResourceState::Released
    };
    if let Some(instance_id) = carrier
        .instance_id
        .filter(|_| live && carrier.active_generation.is_none())
    {
        resources[0].target = Some(Target::RuntimeCarrier {
            agent_session_id: session.as_ref().into(),
            instance_id,
            expected_generation: carrier
                .active_generation
                .or(carrier.constructing_generation),
            expected_head_seq: head_seq,
        });
    }
    if let Some(operation) = operation.filter(|_| live) {
        let progress = state
            .session_owner
            .canonical
            .store()
            .read_turn_progress(session, operation)
            .await
            .map_err(agent_session_store_error)?;
        if progress.status == TurnReceiptStatus::Running {
            resources[0].target = None;
        }
    }
    if let Some(engines) = state
        .session_owner
        .native_engines
        .get()
        .and_then(std::sync::Weak::upgrade)
    {
        if let Some(processes) = engines.observe_process_resources(owner, session)? {
            resources[1].state = if processes.busy || processes.cleanup_unconfirmed {
                ResourceState::Unconfirmed
            } else if processes.active {
                ResourceState::Active
            } else if processes.reaped {
                ResourceState::Released
            } else {
                ResourceState::Unconfirmed
            };
        }
        if let Some(pending) = engines.observe_knowledge_resources(session)? {
            resources[4].state = if pending {
                ResourceState::Active
            } else {
                ResourceState::Released
            };
            if pending {
                resources[0].target = None;
            }
        }
    }
    #[cfg(feature = "browser-use")]
    if let Some(engines) = state
        .session_owner
        .native_engines
        .get()
        .and_then(std::sync::Weak::upgrade)
    {
        use nomifun_browser_platform::workspace::BrowserRetentionReason as Retention;
        let browser = engines.observe_browser_workspace(owner, session).await?;
        resources[2].state = if browser.closing {
            ResourceState::Releasing
        } else if browser.cleanup_unconfirmed {
            ResourceState::Unconfirmed
        } else if browser.close_confirmed == Some(true) {
            ResourceState::Released
        } else if matches!(
            browser.retention,
            Retention::CurrentUserUse | Retention::UserPageStateUnknown
        ) {
            ResourceState::HeldForUser
        } else if browser.native_registered == Some(false) {
            ResourceState::Unavailable
        } else if browser.native_registered == Some(true) {
            ResourceState::Active
        } else {
            ResourceState::Unconfirmed
        };
        if let Some((runtime_generation, service_instance_id)) = browser
            .runtime_generation
            .filter(|_| live)
            .zip(browser.service_instance_id)
        {
            resources[2].target = Some(Target::BrowserWorkspace {
                agent_session_id: session.as_ref().into(),
                service_instance_id,
                runtime_generation,
                expected_head_seq: head_seq,
            });
        }
    }
    let _ = owner;
    let effects:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
        .bind(session.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    resources[5].state = if effects {
        ResourceState::Unconfirmed
    } else {
        ResourceState::Released
    };
    if let Some(engines) = state
        .session_owner
        .native_engines
        .get()
        .and_then(std::sync::Weak::upgrade)
    {
        let processes = engines.process_resource_instances(owner, session)?;
        if !processes.is_empty() {
            resources.retain(|resource| resource.kind != ResourceKind::Processes);
            resources.extend(processes.into_iter().map(|process| Resource {
                kind: ResourceKind::Processes,
                state: if process.execution_stopped {
                    ResourceState::Released
                } else if process.stop_in_flight {
                    ResourceState::Releasing
                } else if process.cleanup_unconfirmed || process.stop_requested {
                    ResourceState::Unconfirmed
                } else {
                    ResourceState::Active
                },
                detail: None,
                display_name: process.display_name,
                target: Some(Target::ProcessInstance {
                    agent_session_id: session.as_ref().into(),
                    owner_generation: process.owner_generation,
                    process_id: process.process_id,
                    creator_turn_id: process.creator_turn_id,
                    creator_call_id: process.creator_call_id,
                }),
            }));
        }
    }
    let links = super::super::session_ssh_resources::observe_resources(
        state.session_owner.canonical.store(),
        &state.ssh_pool,
        &PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner.into(),
        },
        session,
    )
    .await?;
    if !links.is_empty() {
        let names = state
            .ssh_pool
            .host_service()
            .list(owner)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .into_iter()
            .map(|host| (host.ssh_host_id, host.name))
            .collect::<std::collections::BTreeMap<_, _>>();
        resources.retain(|resource| resource.kind != ResourceKind::Ssh);
        resources.extend(links.into_iter().map(|link| {
            Resource {
                kind: ResourceKind::Ssh,
                state: if link.close_confirmed == Some(true) {
                    ResourceState::Released
                } else if link.close_in_flight {
                    ResourceState::Releasing
                } else if link.cleanup_unconfirmed || link.closing {
                    ResourceState::Unconfirmed
                } else if link.registered
                    && matches!(
                        link.phase,
                        Some(
                            nomifun_ssh::SshLinkPhase::Connecting
                                | nomifun_ssh::SshLinkPhase::Connected
                                | nomifun_ssh::SshLinkPhase::Reconnecting
                                | nomifun_ssh::SshLinkPhase::Degraded
                        )
                    )
                {
                    ResourceState::Active
                } else {
                    ResourceState::Unconfirmed
                },
                detail: None,
                display_name: link
                    .display_name
                    .or_else(|| names.get(&link.ssh_host_id).cloned()),
                target: Some(Target::SshLink {
                    agent_session_id: session.as_ref().into(),
                    ssh_host_id: link.ssh_host_id,
                    pool_instance_id: link.pool_instance_id,
                    instance_id: link.instance_id,
                }),
            }
        }));
    }
    Ok(resources)
}

async fn owned_resource_snapshot(
    state: &NomiCoreAgentApiState,
    principal: &PrincipalRef,
    session: &AgentSessionId,
) -> Result<Vec<Resource>, AppError> {
    let (record, live) = match state
        .session_owner
        .canonical
        .store()
        .get_live_session(session)
        .await
    {
        Ok(session) => (session, true),
        Err(nomifun_agent_session::SessionStoreError::Deleted(_)) => match state
            .session_owner
            .canonical
            .store()
            .get_deleting_session(session)
            .await
        {
            Ok(session) => (session, false),
            Err(
                nomifun_agent_session::SessionStoreError::Deleted(_)
                | nomifun_agent_session::SessionStoreError::NotFound(_),
            ) => return Ok(unknown_resources()),
            Err(error) => return Err(agent_session_store_error(error)),
        },
        Err(nomifun_agent_session::SessionStoreError::NotFound(_)) => {
            return Ok(unknown_resources());
        }
        Err(error) => return Err(agent_session_store_error(error)),
    };
    if record.owner_ref != *principal {
        return Err(AppError::Forbidden(
            "activity resources belong to another owner".into(),
        ));
    }
    let seq: Option<i64> =
        sqlx::query_scalar("SELECT last_seq FROM agent_session_heads WHERE session_id=?")
            .bind(session.as_ref())
            .fetch_optional(&state.session_owner.pool)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(seq) = seq else {
        return Ok(unknown_resources());
    };
    observe_resources(
        state,
        &principal.principal_id,
        session,
        u64::try_from(seq).map_err(|_| AppError::Internal("invalid resource cursor".into()))?,
        None,
        live,
    )
    .await
}

pub(super) async fn list(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<ApiResponse<AgentActivitiesResponse>>, NomiCoreApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let after = query.after_cursor.as_deref().unwrap_or("");
    if after.len() > 4096 || after.chars().any(char::is_control) {
        return Err(AppError::BadRequest("invalid activity cursor".into()).into());
    }
    let mut inventory: BTreeSet<String> = state
        .session_owner
        .runtime_sessions
        .registered_runtime_sessions()
        .unwrap_or_default()
        .into_iter()
        .collect();
    inventory.extend(state.ssh_pool.retained_resource_sessions());
    // Original requested close facts keep an unavailable old pool visible.
    // A formally Reaped receipt is no longer a discovery pin. The owner read
    // below validates the full typed facts; this query is only an inventory hint.
    let pending_ssh:Vec<String>=sqlx::query_scalar("SELECT DISTINCT requested.session_id FROM agent_events requested \
        JOIN agent_sessions session ON session.agent_session_id=requested.session_id \
        WHERE requested.kind='session/ssh-link-close-requested' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
          AND json_extract(session.owner_ref_json,'$.principal_id')=? \
          AND NOT EXISTS(SELECT 1 FROM agent_events settled LEFT JOIN agent_payloads payload ON payload.session_id=settled.session_id AND payload.payload_id=settled.payload_id \
            WHERE settled.session_id=requested.session_id AND settled.kind='session/ssh-link-closed' AND settled.event_id=requested.event_id||':settled' \
              AND COALESCE(json_extract(settled.inline_json,'$.result.state'),json_extract(CAST(payload.body AS TEXT),'$.value.result.state'))='reaped')")
        .bind(owner.as_ref()).fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    inventory.extend(pending_ssh);
    if let Some(engines) = state
        .session_owner
        .native_engines
        .get()
        .and_then(std::sync::Weak::upgrade)
    {
        inventory.extend(engines.registered_process_resource_sessions(owner.as_ref())?);
        inventory.extend(engines.registered_knowledge_resource_sessions()?);
    }
    #[cfg(feature = "browser-use")]
    if let Some(engines) = state
        .session_owner
        .native_engines
        .get()
        .and_then(std::sync::Weak::upgrade)
    {
        inventory.extend(
            engines
                .registered_browser_workspace_sessions(owner.as_ref())
                .await?,
        );
    }
    let resource_inventory = inventory.clone();
    inventory.extend(
        state
            .session_owner
            .registered_input_preparation_sessions()?,
    );
    let inventory_json =
        serde_json::to_string(&inventory).map_err(|error| AppError::Internal(error.to_string()))?;
    let sessions: Vec<SessionCandidate> = sqlx::query_as(
        "SELECT session.agent_session_id AS session_id,session.state,head.status AS head_status,head.active_turn_id,COALESCE(head.last_seq,0) AS head_seq,\
         EXISTS(SELECT 1 FROM nomi_remote_sessions remote WHERE remote.agent_session_id=session.agent_session_id AND remote.state='opening') AS remote_opening,\
         EXISTS(SELECT 1 FROM agent_events process LEFT JOIN agent_payloads payload ON payload.session_id=process.session_id AND payload.payload_id=process.payload_id \
            WHERE process.session_id=session.agent_session_id AND process.kind='runtime/progress-recorded' \
              AND COALESCE(json_array_length(process.inline_json,'$.event.session_process_retention.processes'),json_array_length(CAST(payload.body AS TEXT),'$.value.event.session_process_retention.processes'),0)>0 \
              AND NOT EXISTS(SELECT 1 FROM agent_events released WHERE released.session_id=process.session_id AND released.kind='session/process-owner-released' \
                AND released.event_id='process-owner-release:'||process.session_id||':'||COALESCE(json_extract(process.inline_json,'$.event.session_process_retention.owner_generation'),json_extract(CAST(payload.body AS TEXT),'$.value.event.session_process_retention.owner_generation')))) AS process_hint \
         FROM agent_sessions session LEFT JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         WHERE session.state IN ('live','deleting') AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? AND 'session:'||session.agent_session_id||':~'>? \
           AND (head.active_turn_id IS NOT NULL OR session.state='deleting' OR session.agent_session_id IN (SELECT value FROM json_each(?)) \
             OR EXISTS(SELECT 1 FROM nomi_remote_sessions remote WHERE remote.agent_session_id=session.agent_session_id AND remote.state='opening') \
             OR EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=session.agent_session_id AND effect.state IN ('pending','unknown')) \
             OR EXISTS(SELECT 1 FROM agent_events started WHERE started.session_id=session.agent_session_id AND started.kind='turn/knowledge-writeback-started' \
                AND NOT EXISTS(SELECT 1 FROM agent_events settled WHERE settled.session_id=started.session_id AND settled.kind='turn/knowledge-writeback-settled' AND settled.correlation_id=started.correlation_id)) \
             OR EXISTS(SELECT 1 FROM agent_events process LEFT JOIN agent_payloads payload ON payload.session_id=process.session_id AND payload.payload_id=process.payload_id \
                WHERE process.session_id=session.agent_session_id AND process.kind='runtime/progress-recorded' \
                  AND COALESCE(json_array_length(process.inline_json,'$.event.session_process_retention.processes'),json_array_length(CAST(payload.body AS TEXT),'$.value.event.session_process_retention.processes'),0)>0 \
                  AND NOT EXISTS(SELECT 1 FROM agent_events released WHERE released.session_id=process.session_id AND released.kind='session/process-owner-released' \
                    AND released.event_id='process-owner-release:'||process.session_id||':'||COALESCE(json_extract(process.inline_json,'$.event.session_process_retention.owner_generation'),json_extract(CAST(payload.body AS TEXT),'$.value.event.session_process_retention.owner_generation')))) \
             OR EXISTS(SELECT 1 FROM agent_events input WHERE input.session_id=session.agent_session_id AND input.kind='message/user-accepted' \
                AND input.kind_version=1 AND input.producer_id='session_api' \
                AND input.idempotency_key LIKE 'session-input:user:'||json_extract(session.owner_ref_json,'$.principal_id')||':'||session.agent_session_id||':%' \
                AND input.seq>COALESCE((SELECT MAX(clear.seq) FROM agent_events clear WHERE clear.session_id=input.session_id AND clear.kind='context/cleared'),0) \
                AND NOT EXISTS(SELECT 1 FROM agent_turns turn WHERE turn.session_id=input.session_id AND turn.source_message_id=input.event_id) \
                AND NOT EXISTS(SELECT 1 FROM agent_events blocked WHERE blocked.session_id=input.session_id AND blocked.event_id='session-input-blocked:'||input.event_id||':cancelled_by_owner') \
                AND NOT EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads payload ON payload.session_id=claim.session_id AND payload.payload_id=claim.payload_id \
                    WHERE claim.session_id=input.session_id AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
                      AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(payload.body AS TEXT),'$.value.input_event_id'))=input.event_id))) \
         ORDER BY session.agent_session_id LIMIT ?",
    ).bind(owner.as_ref()).bind(after).bind(&inventory_json).bind((limit+1) as i64)
        .fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let attempts: Vec<AttemptCandidate> = sqlx::query_as(
        "SELECT execution.execution_id,step.step_id,attempt.attempt_id,step.title,attempt.status AS attempt_status,execution.status AS execution_status, \
         execution.version AS execution_version,step.version AS step_version,attempt.runtime_state,link.conversation_id,attempt.updated_at,execution.initial_plan_input, \
         EXISTS(SELECT 1 FROM conversation_execution_links cleanup WHERE cleanup.execution_id=attempt.execution_id AND cleanup.attempt_id=attempt.attempt_id \
            AND cleanup.relation IN ('attempt','automation') AND cleanup.active=0 AND cleanup.cleanup_completed_at IS NULL \
            AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link WHERE active_link.conversation_id=cleanup.conversation_id \
                AND active_link.execution_id=cleanup.execution_id AND active_link.step_id=cleanup.step_id AND active_link.attempt_id=cleanup.attempt_id \
                AND active_link.relation IN ('attempt','automation') AND active_link.active=1)) AS cleanup_pending \
         FROM agent_execution_attempts attempt JOIN agent_executions execution ON execution.execution_id=attempt.execution_id \
         JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
         LEFT JOIN conversation_execution_links link ON link.id=(SELECT MAX(candidate.id) FROM conversation_execution_links candidate \
            WHERE candidate.execution_id=attempt.execution_id AND candidate.attempt_id=attempt.attempt_id AND candidate.relation IN ('attempt','automation')) \
         WHERE execution.user_id=? AND step.kind='agent' AND 'execution:'||attempt.attempt_id>? \
           AND (attempt.status IN ('queued','running','waiting_input','interrupted') OR \
             EXISTS(SELECT 1 FROM conversation_execution_links cleanup WHERE cleanup.execution_id=attempt.execution_id AND cleanup.attempt_id=attempt.attempt_id \
                AND cleanup.relation IN ('attempt','automation') AND cleanup.active=0 AND cleanup.cleanup_completed_at IS NULL \
                AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link WHERE active_link.conversation_id=cleanup.conversation_id \
                    AND active_link.execution_id=cleanup.execution_id AND active_link.step_id=cleanup.step_id AND active_link.attempt_id=cleanup.attempt_id \
                    AND active_link.relation IN ('attempt','automation') AND active_link.active=1)) \
             OR (link.relation='attempt' AND link.conversation_id IN (SELECT value FROM json_each(?))) \
             OR (json_extract(execution.initial_plan_input,'$.mode')='automation' \
               AND step.introduced_in_revision=(SELECT MIN(original.introduced_in_revision) FROM agent_execution_steps original WHERE original.execution_id=execution.execution_id) \
               AND EXISTS(SELECT 1 FROM requirements requirement WHERE requirement.requirement_id=json_extract(execution.initial_plan_input,'$.source.requirement_id') \
                 AND requirement.claim_generation=json_extract(execution.initial_plan_input,'$.source.claim_generation') AND requirement.status='in_progress'))) \
         ORDER BY attempt.attempt_id LIMIT ?",
    ).bind(owner.as_ref()).bind(after).bind(&inventory_json).bind((limit+1) as i64)
        .fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let scan_more = if sessions.len() == limit + 1 {
        sessions
            .last()
            .map(|row| format!("session:{}:~", row.session_id))
    } else {
        None
    };
    let attempt_scan_more = if attempts.len() == limit + 1 {
        attempts
            .last()
            .map(|row| format!("execution:{}", row.attempt_id))
    } else {
        None
    };
    let cron_runs:Vec<CronCandidate>=sqlx::query_as("SELECT reservation.cron_job_id,reservation.cron_job_run_id,COALESCE(job.name,session.title,'') AS title,\
        reservation.status,reservation.conversation_id,reservation.updated_at_ms,reservation.cancel_requested_at_ms IS NOT NULL AS stop_accepted FROM cron_run_reservations reservation \
        LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
        LEFT JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
        WHERE COALESCE(job.user_id,CASE WHEN json_extract(session.owner_ref_json,'$.principal_kind')='user' THEN json_extract(session.owner_ref_json,'$.principal_id') END)=? \
        AND reservation.status='reserved' \
        AND 'cron:'||reservation.cron_job_run_id>? ORDER BY reservation.cron_job_run_id LIMIT ?")
        .bind(owner.as_ref()).bind(after).bind((limit+1)as i64).fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let cron_scan_more = if cron_runs.len() == limit + 1 {
        cron_runs
            .last()
            .map(|row| format!("cron:{}", row.cron_job_run_id))
    } else {
        None
    };
    let autowork:Vec<AutoWorkCandidate>=sqlx::query_as("SELECT requirement.requirement_id,requirement.title,requirement.claim_generation,\
        requirement.owner_conversation_id AS target_id,requirement.updated_at FROM requirements requirement \
        JOIN agent_sessions session ON session.agent_session_id=requirement.owner_conversation_id \
        WHERE json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=? \
        AND requirement.status='in_progress' AND 'autowork:'||requirement.requirement_id>? \
        AND NOT EXISTS(SELECT 1 FROM agent_executions execution JOIN agent_execution_attempts attempt ON attempt.execution_id=execution.execution_id \
            WHERE json_extract(execution.initial_plan_input,'$.source.requirement_id')=requirement.requirement_id \
              AND json_extract(execution.initial_plan_input,'$.source.claim_generation')=requirement.claim_generation \
              AND (attempt.status IN ('queued','running','waiting_input','interrupted') OR \
                attempt.step_id=(SELECT original.step_id FROM agent_execution_steps original WHERE original.execution_id=execution.execution_id ORDER BY original.introduced_in_revision,original.id LIMIT 1))) \
        ORDER BY requirement.requirement_id LIMIT ?")
        .bind(owner.as_ref()).bind(after).bind((limit+1)as i64).fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let autowork_scan_more = if autowork.len() == limit + 1 {
        autowork
            .last()
            .map(|row| format!("autowork:{}", row.requirement_id))
    } else {
        None
    };
    let queued_steps:Vec<QueuedStepCandidate>=sqlx::query_as("SELECT step.execution_id,step.step_id,step.title,execution.version AS execution_version,step.version AS step_version,step.updated_at,execution.initial_plan_input \
        FROM agent_execution_steps step JOIN agent_executions execution ON execution.execution_id=step.execution_id \
        WHERE execution.user_id=? AND execution.status IN ('running','waiting_input','paused') AND step.kind='agent' AND step.status='pending' \
        AND step.superseded_in_revision IS NULL \
        AND NOT EXISTS(SELECT 1 FROM agent_execution_attempts attempt WHERE attempt.execution_id=step.execution_id AND attempt.step_id=step.step_id \
            AND attempt.status IN ('queued','running','waiting_input','interrupted')) \
        AND 'step:'||step.step_id>? ORDER BY step.step_id LIMIT ?")
        .bind(owner.as_ref()).bind(after).bind((limit+1)as i64).fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let step_scan_more = if queued_steps.len() == limit + 1 {
        queued_steps
            .last()
            .map(|row| format!("step:{}", row.step_id))
    } else {
        None
    };
    let planning_observations = state
        .session_owner
        .execution_progress
        .get()
        .and_then(std::sync::Weak::upgrade)
        .map(|engine| engine.planning_operations())
        .unwrap_or_default();
    let planning = planning_candidates(
        &state.session_owner.pool,
        owner.as_ref(),
        after,
        limit,
        &planning_observations,
    )
    .await?;
    let planning_scan_more = if planning.len() == limit + 1 {
        planning
            .last()
            .map(|row| format!("planning:{}", row.execution_id))
    } else {
        None
    };
    let principal = authenticated_principal(&owner);
    let mut items = Vec::new();
    let mut automation_controls=HashMap::new();
    for candidate in sessions {
        let session_id = AgentSessionId::from(candidate.session_id.clone());
        let session = match if candidate.state == "deleting" {
            state
                .session_owner
                .canonical
                .store()
                .get_deleting_session(&session_id)
                .await
        } else {
            state
                .session_owner
                .canonical
                .store()
                .get_live_session(&session_id)
                .await
        } {
            Ok(session) => session,
            Err(
                nomifun_agent_session::SessionStoreError::NotFound(_)
                | nomifun_agent_session::SessionStoreError::Deleted(_),
            ) => continue,
            Err(error) => return Err(agent_session_store_error(error).into()),
        };
        if session.owner_ref != principal {
            return Err(
                AppError::Forbidden("activity Session belongs to another owner".into()).into(),
            );
        }
        let title = session.metadata.title.clone().unwrap_or_default();
        let head_seq = u64::try_from(candidate.head_seq)
            .map_err(|_| AppError::Internal("invalid activity head cursor".into()))?;
        let mut preparing = BTreeSet::new();
        if candidate.state == "live" {
            for (input, signal_requested) in state
                .session_owner
                .input_preparation_observations(&session_id)?
            {
                let delivery = state
                    .session_owner
                    .canonical
                    .store()
                    .session_input_delivery(&session_id, &input)
                    .await
                    .map_err(agent_session_store_error)?;
                if state
                    .session_owner
                    .canonical
                    .store()
                    .session_input_claim(&session_id, &input)
                    .await
                    .map_err(agent_session_store_error)?
                    .is_some()
                {
                    continue;
                }
                let cancelled = delivery.get("state").and_then(Value::as_str) == Some("cancelled");
                preparing.insert(input.as_ref().to_owned());
                items.push(AgentActivity {
            delegation_target:None,
                    key: format!(
                        "session:{}:preparation:{}",
                        session_id.as_ref(),
                        input.as_ref()
                    ),
                    title: title.clone(),
                    kind: if session.metadata.companion.is_some() {
                        Kind::Companion
                    } else {
                        Kind::Conversation
                    },
                    state: if cancelled || signal_requested {
                        ActivityState::Stopping
                    } else {
                        ActivityState::Preparing
                    },
                    agent_session_id: Some(candidate.session_id.clone()),
                    target: Target::SessionInput {
                        agent_session_id: candidate.session_id.clone(),
                        message_id: input.as_ref().into(),
                    },
                    continuation: Continuation::None,
                    waiting_reason: None,
                    stop_accepted: cancelled,
                    execution_stopped: false,
                    resources: unknown_resources(),
                    can_stop: true,
                    can_retry_cleanup: false,
                    updated_at_ms: None,
                });
            }
        }
        // Post-answer work belongs to its original Operation independently of
        // the Session's current Attempt/Cron/AutoWork presentation.
        if candidate.state == "live" {
            if let Some(engines) = state
                .session_owner
                .native_engines
                .get()
                .and_then(std::sync::Weak::upgrade)
            {
                for background in engines.running_knowledge_operations(&session_id)? {
                    let stopping = engines.knowledge_stop_requested(&session_id, &background)?;
                    let mut resources = unknown_resources();
                    resources[4].state = if stopping {
                        ResourceState::Releasing
                    } else {
                        ResourceState::Active
                    };
                    items.push(AgentActivity {
            delegation_target:None,
                        key: format!(
                            "session:{}:knowledge:{}",
                            session_id.as_ref(),
                            background.as_ref()
                        ),
                        title: title.clone(),
                        kind: Kind::KnowledgeWriteback,
                        state: if stopping {
                            ActivityState::Stopping
                        } else {
                            ActivityState::Running
                        },
                        agent_session_id: Some(candidate.session_id.clone()),
                        target: Target::KnowledgeWriteback {
                            agent_session_id: candidate.session_id.clone(),
                            operation_id: background.as_ref().into(),
                        },
                        continuation: Continuation::None,
                        waiting_reason: None,
                        stop_accepted: stopping,
                        execution_stopped: false,
                        resources,
                        can_stop: true,
                        can_retry_cleanup: false,
                        updated_at_ms: None,
                    });
                }
            }
        }
        let attempt_session: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation='attempt')")
            .bind(session_id.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if attempt_session && candidate.state != "deleting" {
            continue;
        }
        if candidate.state == "live" {
            for pending in queued_inputs(&state, &principal, &session_id, after, limit).await? {
                if preparing.contains(pending.event.event_id.as_ref()) {
                    continue;
                }
                items.push(AgentActivity {
            delegation_target:None,
                    key: format!(
                        "session:{}:input:{:020}",
                        session_id.as_ref(),
                        pending.event.seq
                    ),
                    title: title.clone(),
                    kind: if session.metadata.companion.is_some() {
                        Kind::Companion
                    } else {
                        Kind::Conversation
                    },
                    state: ActivityState::Queued,
                    agent_session_id: Some(candidate.session_id.clone()),
                    target: Target::SessionInput {
                        agent_session_id: candidate.session_id.clone(),
                        message_id: pending.event.event_id.as_ref().into(),
                    },
                    continuation: Continuation::None,
                    waiting_reason: None,
                    stop_accepted: false,
                    execution_stopped: false,
                    resources: unknown_resources(),
                    can_stop: true,
                    can_retry_cleanup: false,
                    updated_at_ms: None,
                });
            }
        }
        let operation = candidate.active_turn_id.clone().map(OperationId::from);
        let domain_operation=operation.as_ref().is_some_and(|operation|operation.as_ref().contains(":cron:"))
            || sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation='automation' AND active=1)")
                .bind(session_id.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if domain_operation && candidate.state == "live" {
            continue;
        }
        if operation.is_none()
            && candidate.state == "live"
            && !resource_inventory.contains(session_id.as_ref())
        {
            let outstanding:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown')) \
                OR EXISTS(SELECT 1 FROM agent_events started WHERE started.session_id=? AND started.kind='turn/knowledge-writeback-started' \
                    AND NOT EXISTS(SELECT 1 FROM agent_events settled WHERE settled.session_id=started.session_id AND settled.kind='turn/knowledge-writeback-settled' AND settled.correlation_id=started.correlation_id)) \
                OR EXISTS(SELECT 1 FROM nomi_remote_sessions WHERE agent_session_id=? AND state='opening')")
                .bind(session_id.as_ref()).bind(session_id.as_ref()).bind(session_id.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
            let retained_processes = candidate.process_hint
                && !state
                    .session_owner
                    .canonical
                    .store()
                    .unreleased_session_process_generations(&principal, &session_id)
                    .await
                    .map_err(agent_session_store_error)?
                    .is_empty();
            if !outstanding && !retained_processes {
                continue;
            }
        }
        let resources = observe_resources(
            &state,
            owner.as_ref(),
            &session_id,
            head_seq,
            operation.as_ref(),
            candidate.state == "live",
        )
        .await?;
        let target = if candidate.state == "deleting" {
            Some(Target::SessionResources {
                agent_session_id: candidate.session_id.clone(),
                expected_head_seq: head_seq,
            })
        } else if let Some(remote) = session
            .remote_binding_provenance
            .as_ref()
            .filter(|_| operation.is_some() || candidate.remote_opening)
        {
            Some(Target::RemoteSession {
                agent_session_id: candidate.session_id.clone(),
                operation_id: operation
                    .as_ref()
                    .map(|operation| operation.as_ref().into()),
                expected_head_seq: head_seq,
                expected_binding_version: i64::try_from(remote.binding_version)
                    .map_err(|_| AppError::Internal("Remote binding version overflow".into()))?,
            })
        } else if let Some(operation) = &operation {
            Some(Target::SessionTurn {
                agent_session_id: candidate.session_id.clone(),
                operation_id: operation.as_ref().into(),
            })
        } else {
            resources
                .iter()
                .filter_map(|resource| resource.target.clone())
                .next()
                .or_else(|| {
                    Some(Target::SessionResources {
                        agent_session_id: candidate.session_id.clone(),
                        expected_head_seq: head_seq,
                    })
                })
        };
        let Some(target) = target else {
            continue;
        };
        let mut row_state =
            activity_state(candidate.head_status.as_deref().unwrap_or("unconfirmed"));
        let mut continuation = Continuation::None;
        if candidate.state == "live" && candidate.head_status.as_deref() == Some("paused") {
            if let Some(inspection) = state
                .session_owner
                .canonical
                .store()
                .inspect_latest_native_execution(&principal, &session_id)
                .await
                .map_err(agent_session_store_error)?
            {
                if inspection.resume_requested {
                    continuation = Continuation::ResumeQueued;
                }
            }
        }
        let carrier = state
            .session_owner
            .runtime_sessions
            .observe_runtime_carrier(session_id.as_ref());
        let mut canonical_stop = false;
        let mut execution_stopped = false;
        if operation.is_none() && candidate.state == "live" {
            let latest:Option<(String,String)>=sqlx::query_as("SELECT turn.operation_id,turn.state FROM agent_turns turn JOIN agent_events started \
                ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id AND started.kind='turn/started' \
                WHERE turn.session_id=? ORDER BY started.seq DESC LIMIT 1")
                .bind(session_id.as_ref()).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
            if let Some((last, status)) = latest {
                canonical_stop = status == "cancelled";
                execution_stopped =
                    operation_stopped(&state.session_owner, &session_id, &OperationId::from(last))
                        .await?;
            }
        }
        let remote_stop = if session.remote_binding_provenance.is_some() {
            remote_has_active_cancel_fence(
                &state.remote_repository,
                owner.as_ref(),
                session_id.as_ref(),
            )
            .await?
        } else {
            false
        };
        if canonical_stop {
            row_state = if execution_stopped {
                ActivityState::Cancelled
            } else {
                ActivityState::Stopping
            };
        }
        if carrier.stop_requested
            || carrier.teardown_pending
            || candidate.state == "deleting"
            || remote_stop
        {
            row_state = ActivityState::Stopping;
        }
        let kind = if operation.is_none() && !candidate.remote_opening {
            Kind::Resources
        } else if session.remote_binding_provenance.is_some() {
            Kind::RemoteSession
        } else if session.metadata.companion.is_some() {
            Kind::Companion
        } else {
            Kind::Conversation
        };
        items.push(AgentActivity {
            delegation_target:None,
            key: format!("session:{}:0", session_id.as_ref()),
            title,
            kind,
            state: row_state,
            agent_session_id: Some(candidate.session_id),
            target,
            continuation,
            waiting_reason: None,
            stop_accepted: carrier.stop_requested || remote_stop || canonical_stop,
            execution_stopped,
            resources,
            can_stop: candidate.state == "live",
            can_retry_cleanup: carrier.teardown_pending
                || candidate.state == "deleting"
                || remote_stop,
            updated_at_ms: None,
        });
    }
    for candidate in cron_runs {
        let resources = if let Some(session) = candidate.conversation_id.as_ref() {
            let session = AgentSessionId::from(session.clone());
            owned_resource_snapshot(&state, &principal, &session).await?
        } else {
            unknown_resources()
        };
        items.push(AgentActivity {
            delegation_target:None,
            key: format!("cron:{}", candidate.cron_job_run_id),
            title: candidate.title,
            kind: Kind::CronRun,
            state: if candidate.stop_accepted {
                ActivityState::Stopping
            } else if candidate.status == "input_required" {
                ActivityState::Waiting
            } else {
                ActivityState::Running
            },
            agent_session_id: candidate.conversation_id,
            target: Target::CronRun {
                cron_job_id: candidate.cron_job_id,
                cron_job_run_id: candidate.cron_job_run_id,
            },
            continuation: Continuation::None,
            waiting_reason: None,
            stop_accepted: candidate.stop_accepted,
            execution_stopped: false,
            resources,
            can_stop: true,
            can_retry_cleanup: candidate.stop_accepted,
            updated_at_ms: Some(candidate.updated_at_ms),
        });
    }
    for candidate in autowork {
        let session = AgentSessionId::from(candidate.target_id.clone());
        let resources = owned_resource_snapshot(&state, &principal, &session).await?;
        items.push(AgentActivity {
            delegation_target:None,
            key: format!("autowork:{}", candidate.requirement_id),
            title: candidate.title,
            kind: Kind::AutoWorkRun,
            state: ActivityState::Running,
            agent_session_id: Some(candidate.target_id.clone()),
            target: Target::AutoWorkRun {
                target_kind: nomifun_api_types::AutoWorkTargetKind::Conversation,
                target_id: candidate.target_id,
                requirement_id: candidate.requirement_id,
                claim_generation: candidate.claim_generation,
            },
            continuation: Continuation::None,
            waiting_reason: None,
            stop_accepted: false,
            execution_stopped: false,
            resources,
            can_stop: true,
            can_retry_cleanup: false,
            updated_at_ms: Some(candidate.updated_at),
        });
    }
    for candidate in planning {
        // A scoped Step Stop may signal a delegation planner without cancelling
        // this whole collaboration. Only its durable aggregate proves this label.
        let stop_accepted = candidate.status == "cancelled";
        items.push(AgentActivity {
            delegation_target:None,
            key: format!("planning:{}", candidate.execution_id),
            title: candidate.goal,
            kind: Kind::Collaboration,
            state: if stop_accepted {
                ActivityState::Stopping
            } else {
                activity_state(&candidate.status)
            },
            agent_session_id: None,
            target: Target::ExecutionGroup {
                execution_id: candidate.execution_id,
                expected_version: candidate.version,
            },
            continuation: Continuation::None,
            waiting_reason: None,
            stop_accepted,
            execution_stopped: false,
            resources: unknown_resources(),
            can_stop: true,
            can_retry_cleanup: false,
            updated_at_ms: Some(candidate.updated_at),
        });
    }
    for candidate in queued_steps {
        let control=automation_activity_control(&state,owner.as_ref(),&candidate.execution_id,&candidate.initial_plan_input,&mut automation_controls).await?;
        let task_target=Target::ExecutionStep {execution_id:candidate.execution_id.clone(),step_id:candidate.step_id.clone(),
            expected_execution_version:candidate.execution_version,expected_step_version:candidate.step_version};
        let root=control.as_ref().filter(|control|control.root_step_id.as_deref()==Some(candidate.step_id.as_str()));
        let (kind,target,delegation_target)=if let Some(root)=root {
            (Kind::AutoWorkRun,Target::AutoWorkRun {target_kind:nomifun_api_types::AutoWorkTargetKind::Conversation,
                target_id:root.lead_conversation_id.clone(),requirement_id:root.requirement_id.clone(),claim_generation:root.claim_generation},Some(task_target))
        } else {(Kind::DelegatedTask,task_target,None)};
        if root.is_some() && let Some(existing)=items.iter_mut().find(|item|item.target==target) {
            existing.delegation_target=delegation_target;continue;
        }
        items.push(AgentActivity {
            delegation_target,
            key: format!("step:{}", candidate.step_id),
            title: candidate.title,
            kind,
            state: ActivityState::Queued,
            agent_session_id: None,
            target,
            continuation: Continuation::None,
            waiting_reason: None,
            stop_accepted: false,
            execution_stopped: false,
            resources: unknown_resources(),
            can_stop: true,
            can_retry_cleanup: false,
            updated_at_ms: Some(candidate.updated_at),
        });
    }
    for candidate in attempts {
        let effects = candidate
            .runtime_state
            .clone()
            .map(|raw| nomifun_db::AttemptConversationEffects::decode(Some(&raw)))
            .transpose()?;
        let mut continuation = if effects.as_ref().is_some_and(|effects| {
            effects.pending_conversation_effects.iter().any(|effect| {
                matches!(
                    effect,
                    nomifun_db::PendingConversationEffect::DecisionInput { .. }
                )
            })
        }) {
            Continuation::AnswerQueued
        } else {
            Continuation::None
        };
        let mut row_state = activity_state(&candidate.attempt_status);
        let mut resources = unknown_resources();
        let mut execution_stopped = false;
        if let Some(session) = candidate.conversation_id.as_ref() {
            let session_id = AgentSessionId::from(session.clone());
            let owned = match state
                .session_owner
                .canonical
                .store()
                .get_live_session(&session_id)
                .await
            {
                Ok(session) => Some(session),
                Err(
                    nomifun_agent_session::SessionStoreError::NotFound(_)
                    | nomifun_agent_session::SessionStoreError::Deleted(_),
                ) => None,
                Err(error) => return Err(agent_session_store_error(error).into()),
            };
            if let Some(owned) = owned {
                if owned.owner_ref != principal {
                    return Err(AppError::Forbidden(
                        "activity Attempt Session belongs to another owner".into(),
                    )
                    .into());
                }
                let head = state
                    .session_owner
                    .canonical
                    .store()
                    .head(&session_id)
                    .await
                    .map_err(agent_session_store_error)?;
                let key = effects
                    .clone()
                    .unwrap_or_default()
                    .current_turn_operation_key(&candidate.attempt_id)?;
                let operation =
                    NomiCoreSessionOwner::turn_operation_id(owner.as_ref(), session, &key);
                let progress = state
                    .session_owner
                    .canonical
                    .store()
                    .read_turn_progress(&session_id, &operation)
                    .await
                    .map_err(agent_session_store_error)?;
                resources = observe_resources(
                    &state,
                    owner.as_ref(),
                    &session_id,
                    head.last_seq,
                    Some(&operation),
                    true,
                )
                .await?;
                let carrier = state
                    .session_owner
                    .runtime_sessions
                    .observe_runtime_carrier(session);
                execution_stopped = terminal_turn(progress.status)
                    && carrier.supported
                    && !carrier.registered
                    && carrier.active_generation.is_none()
                    && carrier.constructing_generation.is_none()
                    && !carrier.uncertain_acquisition
                    && !carrier.teardown_pending;
                if !terminal_turn(progress.status) && progress.suspended {
                    row_state = if head.status == "paused" {
                        ActivityState::Paused
                    } else {
                        ActivityState::Waiting
                    };
                }
                if head.status == "paused" {
                    if let Some(inspection) = state
                        .session_owner
                        .canonical
                        .store()
                        .inspect_latest_native_execution(&principal, &session_id)
                        .await
                        .map_err(agent_session_store_error)?
                    {
                        if inspection.resume_requested {
                            continuation = Continuation::ResumeQueued;
                        }
                    }
                }
            }
        }
        let control=automation_activity_control(&state,owner.as_ref(),&candidate.execution_id,&candidate.initial_plan_input,&mut automation_controls).await?;
        let mut stop_accepted = candidate.attempt_status == "cancelled";
        let task_target=Target::ExecutionTask {execution_id:candidate.execution_id.clone(),step_id:candidate.step_id.clone(),
            attempt_id:candidate.attempt_id.clone(),expected_execution_version:candidate.execution_version,expected_step_version:candidate.step_version};
        let root=control.as_ref().filter(|control|control.root_step_id.as_deref()==Some(candidate.step_id.as_str()));
        let mut root_run_active=false;
        let (kind, target,delegation_target) = if let Some(root)=root {
            let source_status:Option<String>=sqlx::query_scalar("SELECT status FROM requirements WHERE requirement_id=? AND claim_generation=? AND owner_conversation_id=?")
                .bind(&root.requirement_id).bind(root.claim_generation).bind(&root.lead_conversation_id).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
            let cancelled=source_status.as_deref()==Some("cancelled");
            root_run_active=source_status.as_deref()==Some("in_progress");
            if root_run_active && matches!(candidate.attempt_status.as_str(),"completed"|"failed"|"cancelled") {
                row_state=activity_state(&candidate.execution_status);
            }
            if cancelled {
                stop_accepted = true;
                if !execution_stopped {
                    row_state = ActivityState::Stopping;
                }
            }
            (
                Kind::AutoWorkRun,
                Target::AutoWorkRun {
                    target_kind: nomifun_api_types::AutoWorkTargetKind::Conversation,
                    target_id:root.lead_conversation_id.clone(),
                    requirement_id: root.requirement_id.clone(),
                    claim_generation: root.claim_generation,
                },
                (!matches!(candidate.execution_status.as_str(),"completed"|"completed_with_failures"|"failed"|"cancelled")).then_some(task_target),
            )
        } else {
            (
                Kind::DelegatedTask,
                task_target,None,
            )
        };
        let terminal = matches!(
            candidate.attempt_status.as_str(),
            "completed" | "failed" | "cancelled"
        );
        items.push(AgentActivity {
            delegation_target,
            key: format!("execution:{}", candidate.attempt_id),
            title: candidate.title,
            kind,
            state: row_state,
            agent_session_id: candidate.conversation_id,
            target,
            continuation,
            waiting_reason: None,
            stop_accepted,
            execution_stopped,
            resources,
            can_stop: if root.is_some() {root_run_active} else {!terminal},
            can_retry_cleanup: candidate.cleanup_pending,
            updated_at_ms: Some(candidate.updated_at),
        });
        let _ = candidate.execution_status;
    }
    items.retain(|item| item.key.as_str() > after);
    items.sort_by(|left, right| left.key.cmp(&right.key));
    let next_cursor = if items.len() > limit {
        Some(items[limit - 1].key.clone())
    } else {
        scan_more
            .into_iter()
            .chain(attempt_scan_more)
            .chain(cron_scan_more)
            .chain(autowork_scan_more)
            .chain(step_scan_more)
            .chain(planning_scan_more)
            .min()
    };
    items.truncate(limit);
    Ok(Json(ApiResponse::ok(AgentActivitiesResponse {
        items,
        next_cursor,
    })))
}

fn activity_operation_id(value: &str) -> Result<OperationId, NomiCoreApiError> {
    // Operation identity is a canonical Turn fact, not the shorter key of
    // this control request. Preserve the exact original Unicode identity.
    if value.is_empty() || value.trim() != value || value.len() > 1024
        || value.chars().any(char::is_control) {
        return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST, "NOMI_CORE_INVALID_REQUEST",
            "operation_id must be an exact non-empty Turn operation within 1024 UTF-8 bytes"));
    }
    Ok(OperationId::from(value.to_owned()))
}

/// The original host task tracker owns a user command through native receipt
/// and cleanup acknowledgement, even if its HTTP observer goes away.
pub(super) async fn stop(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<nomifun_api_types::StopAgentActivityRequest>,
) -> Result<Json<ApiResponse<nomifun_api_types::AgentActivityCommandReceipt>>, NomiCoreApiError> {
    canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let command_state = state.clone();
    if !state
        .session_owner
        .background_tasks
        .spawn(Box::pin(async move {
            let result = execute_stop(command_state, owner, request).await;
            let _ = sender.send(result);
        }))
    {
        return Err(AppError::Conflict("activity control owner is shutting down".into()).into());
    }
    let receipt = receiver.await.map_err(|_| {
        AppError::Conflict("activity control ended without its original receipt".into())
    })??;
    Ok(Json(ApiResponse::ok(receipt)))
}

async fn execute_stop(
    state: NomiCoreAgentApiState,
    owner: AuthenticatedOwner,
    request: nomifun_api_types::StopAgentActivityRequest,
) -> Result<nomifun_api_types::AgentActivityCommandReceipt, NomiCoreApiError> {
    let principal = authenticated_principal(&owner);
    if request.scope.is_some()
        && !matches!(
            &request.target,
            Target::ExecutionTask { .. } | Target::ExecutionStep { .. }
        )
    {
        return Err(AppError::BadRequest(
            "delegation scope applies only to an original Execution task".into(),
        )
        .into());
    }
    let mut execution_stopped = false;
    let session = match &request.target {
        Target::SessionInput {
            agent_session_id,
            message_id,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            ensure_local_session_target(&state, &owner, &session, None).await?;
            let input = EventId::from(message_id.clone());
            state
                .session_owner
                .canonical
                .store()
                .session_input(&session, &input)
                .await
                .map_err(agent_session_store_error)?;
            // The Store writer serializes cancellation against both initial
            // claim and steering claim; a raced claim retains its exact Turn.
            state
                .session_owner
                .canonical
                .store()
                .mark_session_input_blocked(
                    &session,
                    &input,
                    nomifun_agent_session::SessionInputBlockedReason::CancelledByOwner,
                )
                .await
                .map_err(agent_session_store_error)?;
            let preparation_active = state
                .session_owner
                .cancel_input_preparation(&session, &input)?;
            if let Some(claim) = state
                .session_owner
                .canonical
                .store()
                .session_input_claim(&session, &input)
                .await
                .map_err(agent_session_store_error)?
            {
                ensure_local_session_target(&state, &owner, &session, Some(&claim.operation_id))
                    .await?;
                state
                    .session_owner
                    .cancel_exact_turn(
                        owner.as_ref(),
                        &session,
                        &request.idempotency_key,
                        &claim.operation_id,
                        nomifun_common::AgentKillReason::UserCancelled,
                    )
                    .await?;
                execution_stopped =
                    operation_stopped(&state.session_owner, &session, &claim.operation_id).await?;
            } else {
                execution_stopped = !preparation_active;
            }
            Some(session)
        }
        Target::SessionTurn {
            agent_session_id,
            operation_id,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            let operation = activity_operation_id(operation_id)?;
            ensure_local_session_target(&state, &owner, &session, Some(&operation)).await?;
            state
                .session_owner
                .cancel_exact_turn(
                    owner.as_ref(),
                    &session,
                    &request.idempotency_key,
                    &operation,
                    nomifun_common::AgentKillReason::UserCancelled,
                )
                .await?;
            execution_stopped =
                operation_stopped(&state.session_owner, &session, &operation).await?;
            Some(session)
        }
        Target::ExecutionTask {
            execution_id,
            step_id,
            attempt_id,
            expected_execution_version,
            expected_step_version,
        } => {
            let engine = state
                .session_owner
                .execution_progress
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| {
                    AppError::Conflict("original Execution control owner is unavailable".into())
                })?;
            let detail = engine.get(owner.as_ref(), execution_id).await?;
            let attempt = detail
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_id == *attempt_id && attempt.step_id == *step_id)
                .ok_or_else(|| AppError::Conflict("activity task Attempt changed".into()))?;
            let step = detail
                .steps
                .iter()
                .find(|step| step.step_id == *step_id)
                .ok_or_else(|| AppError::Conflict("activity task is missing".into()))?;
            if !request.retry_cleanup
                && detail
                    .attempts
                    .iter()
                    .filter(|candidate| candidate.step_id == *step_id)
                    .max_by_key(|candidate| candidate.attempt_no)
                    .is_some_and(|candidate| candidate.attempt_id != *attempt_id)
            {
                return Err(
                    AppError::Conflict("activity task now has a successor Attempt".into()).into(),
                );
            }
            let _ = step;
            let _ = attempt;
            if request.retry_cleanup {
                engine
                    .retry_task_cleanup(owner.as_ref(), execution_id, step_id, attempt_id)
                    .await?;
            } else {
                engine
                    .cancel_step(
                        owner.as_ref(),
                        &nomifun_common::AgentExecutionActor::user(owner.as_ref()),
                        execution_id,
                        step_id,
                        nomifun_api_types::CancelExecutionStepRequest {
                            expected_execution_version: *expected_execution_version,
                            expected_step_version: *expected_step_version,
                            scope: request
                                .scope
                                .unwrap_or(nomifun_common::AgentExecutionCancellationScope::Task),
                        },
                    )
                    .await?;
            }
            None
        }
        Target::ExecutionGroup {
            execution_id,
            expected_version,
        } => {
            let engine = state
                .session_owner
                .execution_progress
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| {
                    AppError::Conflict("original collaboration control owner is unavailable".into())
                })?;
            let current = engine.get(owner.as_ref(), execution_id).await?;
            if *expected_version < 0 || current.execution.version < *expected_version {
                return Err(
                    AppError::Conflict("captured collaboration version is invalid".into()).into(),
                );
            }
            // Cancellation is absorbing. A lost HTTP acknowledgement must
            // not force an already-cancelled plan into a new control scope.
            if current.execution.status != nomifun_common::AgentExecutionStatus::Cancelled {
                engine
                    .cancel(
                        owner.as_ref(),
                        &nomifun_common::AgentExecutionActor::user(owner.as_ref()),
                        execution_id,
                        nomifun_api_types::VersionedAgentExecutionCommand {
                            expected_version: *expected_version,
                        },
                    )
                    .await?;
            }
            None
        }
        Target::ExecutionStep {
            execution_id,
            step_id,
            expected_execution_version,
            expected_step_version,
        } => {
            let engine = state
                .session_owner
                .execution_progress
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| {
                    AppError::Conflict("original Execution control owner is unavailable".into())
                })?;
            engine
                .cancel_step(
                    owner.as_ref(),
                    &nomifun_common::AgentExecutionActor::user(owner.as_ref()),
                    execution_id,
                    step_id,
                    nomifun_api_types::CancelExecutionStepRequest {
                        expected_execution_version: *expected_execution_version,
                        expected_step_version: *expected_step_version,
                        scope: request
                            .scope
                            .unwrap_or(nomifun_common::AgentExecutionCancellationScope::Task),
                    },
                )
                .await?;
            None
        }
        Target::RemoteSession {
            agent_session_id,
            operation_id,
            expected_head_seq,
            expected_binding_version,
        } => {
            let Json(receipt) = cancel_nomi_core_remote_with_target(
                state.clone(),
                owner.clone(),
                RemoteCancelRequestDto {
                    agent_session_id: agent_session_id.clone(),
                    idempotency_key: request.idempotency_key.clone(),
                },
                Some(RemoteCancelTarget {
                    expected_operation_id: operation_id.clone().map(OperationId::from),
                    expected_head_seq: *expected_head_seq,
                    expected_binding_version: *expected_binding_version,
                }),
            )
            .await?;
            let _ = receipt;
            execution_stopped = exact_remote_cancellation_stopped(
                &state.session_owner,
                &parse_agent_session_id(agent_session_id)?,
                &RemoteCancelTarget {
                    expected_operation_id: operation_id.clone().map(OperationId::from),
                    expected_head_seq: *expected_head_seq,
                    expected_binding_version: *expected_binding_version,
                },
            )
            .await?;
            Some(parse_agent_session_id(agent_session_id)?)
        }
        Target::RuntimeCarrier {
            agent_session_id,
            instance_id,
            expected_generation,
            expected_head_seq,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            let owned = state
                .session_owner
                .canonical
                .store()
                .get_live_session(&session)
                .await
                .map_err(agent_session_store_error)?;
            if owned.owner_ref != principal {
                return Err(AppError::Forbidden(
                    "activity Runtime belongs to another owner".into(),
                )
                .into());
            }
            let head = state
                .session_owner
                .canonical
                .store()
                .head(&session)
                .await
                .map_err(agent_session_store_error)?;
            if head.last_seq != *expected_head_seq || head.active_turn_id.is_some() {
                return Err(AppError::Conflict(
                    "activity Session changed; refresh the exact task before stopping resources"
                        .into(),
                )
                .into());
            }
            if let Some(engines) = state
                .session_owner
                .native_engines
                .get()
                .and_then(std::sync::Weak::upgrade)
            {
                if engines.observe_knowledge_resources(&session)? == Some(true) {
                    return Err(AppError::Conflict("the original Knowledge task must settle or receive its own exact Stop before Runtime retirement".into()).into());
                }
            }
            if !state.session_owner.runtime_sessions.request_runtime_stop(
                session.as_ref(),
                instance_id,
                *expected_generation,
            )? {
                return Err(AppError::Conflict("activity Runtime instance changed".into()).into());
            }
            let runtimes = state.session_owner.runtime_sessions.clone();
            let target = session.clone();
            let instance = instance_id.clone();
            if !state.session_owner.background_tasks.spawn(Box::pin(async move {
                if let Err(error)=runtimes.finish_runtime_stop(target.as_ref(),&instance).await {
                    tracing::warn!(agent_session_id=target.as_ref(),%error,"requested Runtime cleanup remains unconfirmed");
                }
            })) {return Err(AppError::Conflict("Runtime cleanup owner is shutting down; request remains signalled".into()).into());}
            Some(session)
        }
        Target::BrowserWorkspace {
            agent_session_id,
            service_instance_id,
            runtime_generation,
            expected_head_seq,
        } => {
            #[cfg(feature = "browser-use")]
            {
                let session = parse_agent_session_id(agent_session_id)?;
                let _fence = state
                    .session_owner
                    .session_operation_lock(session.as_ref())
                    .try_write_owned()
                    .map_err(|_| {
                        AppError::Conflict(
                            "Browser resource operation is busy; Stop has not been accepted".into(),
                        )
                    })?;
                let observed = state
                    .session_owner
                    .canonical
                    .get(&principal, &session)
                    .await?;
                if observed.head.last_seq != *expected_head_seq {
                    return Err(AppError::Conflict(
                        "activity Session changed before Browser close".into(),
                    )
                    .into());
                }
                state
                    .browser_user_close
                    .close_user_workspace_instance(
                        owner.as_ref(),
                        session.as_ref(),
                        service_instance_id,
                        *runtime_generation,
                    )
                    .await
                    .map_err(|error| AppError::Conflict(error.to_string()))?;
                Some(session)
            }
            #[cfg(not(feature = "browser-use"))]
            {
                let _ = (
                    agent_session_id,
                    service_instance_id,
                    runtime_generation,
                    expected_head_seq,
                );
                return Err(AppError::Conflict(
                    "Browser cleanup is unavailable in this build".into(),
                )
                .into());
            }
        }
        Target::CronRun {
            cron_job_id,
            cron_job_run_id,
        } => {
            let cron = state.cron_cleanup_owner.get().ok_or_else(|| {
                AppError::Conflict("original Cron control owner is unavailable".into())
            })?;
            let receipt = cron
                .cancel_run(owner.as_ref(), cron_job_id, cron_job_run_id)
                .await
                .map_err(|error| AppError::Conflict(error.to_string()))?;
            execution_stopped = receipt.execution_stopped;
            receipt.conversation_id.map(AgentSessionId::from)
        }
        Target::AutoWorkRun {
            target_kind,
            target_id,
            requirement_id,
            claim_generation,
        } => {
            if *target_kind != nomifun_api_types::AutoWorkTargetKind::Conversation {
                return Err(AppError::BadRequest(
                    "AutoWork activity must target its original Conversation owner".into(),
                )
                .into());
            }
            if request.retry_cleanup {
                let session = parse_agent_session_id(target_id)?;
                require_resource_owner(&state, &principal, &session).await?;
                let sources:Vec<(String,String,String)>=sqlx::query_as("SELECT DISTINCT execution.execution_id,execution.initial_plan_input,execution.status FROM agent_executions execution \
                    JOIN conversation_execution_links link ON link.execution_id=execution.execution_id \
                    WHERE execution.user_id=? AND link.conversation_id=? AND link.relation='automation' AND link.active=0 \
                      AND json_extract(execution.initial_plan_input,'$.mode')='automation' \
                      AND json_extract(execution.initial_plan_input,'$.source.requirement_id')=? \
                      AND json_extract(execution.initial_plan_input,'$.source.claim_generation')=?")
                    .bind(owner.as_ref()).bind(target_id).bind(requirement_id).bind(claim_generation)
                    .fetch_all(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                if sources.len() != 1 {
                    return Err(AppError::Conflict(
                        "original inactive AutoWork cleanup source is unavailable or ambiguous"
                            .into(),
                    )
                    .into());
                }
                let (execution_id, raw, status) = &sources[0];
                let initial: Value = serde_json::from_str(raw)
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                let source: nomifun_agent_execution::AutomationExecutionSource =
                    serde_json::from_value(initial.get("source").cloned().ok_or_else(|| {
                        AppError::Conflict("original AutoWork cleanup source is missing".into())
                    })?)
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                let engine = state
                    .session_owner
                    .execution_progress
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .ok_or_else(|| {
                        AppError::Conflict("original AutoWork cleanup owner is unavailable".into())
                    })?;
                if matches!(
                    status.as_str(),
                    "completed" | "completed_with_failures" | "failed" | "cancelled"
                ) {
                    let _ =
                        nomifun_agent_execution::AgentExecutionAutomationPort::cancel_automation(
                            engine.as_ref(),
                            owner.as_ref(),
                            &source,
                        )
                        .await?;
                } else {
                    // An old inactive Attempt can coexist with a legitimate
                    // retry of this same claim. Cleanup must never cancel it.
                    let pending:Option<(String,String)>=sqlx::query_as("SELECT cleanup.step_id,cleanup.attempt_id FROM conversation_execution_links cleanup \
                        WHERE cleanup.execution_id=? AND cleanup.conversation_id=? AND cleanup.relation='automation' AND cleanup.active=0 \
                          AND cleanup.cleanup_completed_at IS NULL AND NOT EXISTS(SELECT 1 FROM conversation_execution_links active_link \
                            WHERE active_link.conversation_id=cleanup.conversation_id AND active_link.execution_id=cleanup.execution_id \
                              AND active_link.step_id=cleanup.step_id AND active_link.attempt_id=cleanup.attempt_id \
                              AND active_link.relation IN ('attempt','automation') AND active_link.active=1) ORDER BY cleanup.id LIMIT 1")
                        .bind(execution_id).bind(target_id).fetch_optional(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                    if let Some((step, attempt)) = pending {
                        engine
                            .retry_task_cleanup(owner.as_ref(), execution_id, &step, &attempt)
                            .await?;
                    }
                }
                Some(session)
            } else {
                let runner = state.autowork_cleanup_owner.get().ok_or_else(|| {
                    AppError::Conflict("original AutoWork control owner is unavailable".into())
                })?;
                let receipt = runner
                    .stop_current_claim(
                        owner.as_ref(),
                        *target_kind,
                        target_id,
                        requirement_id,
                        *claim_generation,
                    )
                    .await?;
                execution_stopped = receipt.execution_stopped;
                if let Some(error) = receipt.cleanup_error {
                    tracing::warn!(%requirement_id,%error,"accepted AutoWork Stop retains original unconfirmed cleanup");
                }
                Some(parse_agent_session_id(target_id)?)
            }
        }
        Target::SessionResources {
            agent_session_id,
            expected_head_seq,
        } => {
            state
                .retry_session_resources(owner.as_ref(), agent_session_id, *expected_head_seq)
                .await?;
            Some(parse_agent_session_id(agent_session_id)?)
        }
        Target::KnowledgeWriteback {
            agent_session_id,
            operation_id,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            let owned = state
                .session_owner
                .canonical
                .store()
                .get_live_session(&session)
                .await
                .map_err(agent_session_store_error)?;
            if owned.owner_ref != principal {
                return Err(
                    AppError::Forbidden("Knowledge work belongs to another owner".into()).into(),
                );
            }
            let operation = activity_operation_id(operation_id)?;
            let engines = state
                .session_owner
                .native_engines
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| {
                    AppError::Conflict("original Knowledge task owner is unavailable".into())
                })?;
            if !engines.cancel_knowledge_writeback(&session, &operation)? {
                let settled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/knowledge-writeback-settled')")
                    .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
                if !settled {
                    return Err(AppError::Conflict("original Knowledge task is no longer observable; cleanup remains unconfirmed".into()).into());
                }
                execution_stopped = true;
            }
            Some(session)
        }
        Target::ProcessInstance {
            agent_session_id,
            owner_generation,
            process_id,
            creator_turn_id,
            creator_call_id,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            require_resource_owner(&state, &principal, &session).await?;
            let engines = state
                .session_owner
                .native_engines
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| {
                    AppError::Conflict("original process resource owner is unavailable".into())
                })?;
            let receipt = engines
                .stop_process_resource(
                    owner.as_ref(),
                    &session,
                    owner_generation,
                    process_id,
                    creator_turn_id,
                    creator_call_id,
                )
                .await?;
            if !receipt.accepted {
                return Err(
                    AppError::Conflict("original process Stop was not accepted".into()).into(),
                );
            }
            execution_stopped = receipt.execution_stopped;
            Some(session)
        }
        Target::SshLink {
            agent_session_id,
            ssh_host_id,
            pool_instance_id,
            instance_id,
        } => {
            let session = parse_agent_session_id(agent_session_id)?;
            require_resource_owner(&state, &principal, &session).await?;
            let receipt = super::super::session_ssh_resources::request_close(
                state.session_owner.canonical.store().clone(),
                state.ssh_pool.clone(),
                state.session_owner.background_tasks.clone(),
                nomifun_agent_contracts::SessionSshLinkCloseV1 {
                    agent_session_id: session.clone(),
                    owner_ref: principal.clone(),
                    ssh_host_id: ssh_host_id.clone(),
                    pool_instance_id: pool_instance_id.clone(),
                    instance_id: *instance_id,
                },
            )
            .await?;
            if !receipt.accepted {
                return Err(AppError::Conflict("original SSH Stop was not accepted".into()).into());
            }
            execution_stopped = receipt.execution_stopped;
            Some(session)
        }
    };
    let resources = if let Some(session) = session {
        match state.session_owner.canonical.store().head(&session).await {
            Ok(head) => {
                observe_resources(&state, owner.as_ref(), &session, head.last_seq, None, true)
                    .await?
            }
            Err(
                nomifun_agent_session::SessionStoreError::NotFound(_)
                | nomifun_agent_session::SessionStoreError::Deleted(_),
            ) => unknown_resources(),
            Err(error) => return Err(agent_session_store_error(error).into()),
        }
    } else {
        unknown_resources()
    };
    Ok(nomifun_api_types::AgentActivityCommandReceipt {
        accepted: true,
        execution_stopped,
        resources,
    })
}

async fn require_resource_owner(
    state: &NomiCoreAgentApiState,
    principal: &PrincipalRef,
    session: &AgentSessionId,
) -> Result<(), AppError> {
    let record = match state
        .session_owner
        .canonical
        .store()
        .get_live_session(session)
        .await
    {
        Ok(record) => record,
        Err(nomifun_agent_session::SessionStoreError::Deleted(_)) => state
            .session_owner
            .canonical
            .store()
            .get_deleting_session(session)
            .await
            .map_err(agent_session_store_error)?,
        Err(error) => return Err(agent_session_store_error(error)),
    };
    if record.owner_ref != *principal {
        return Err(AppError::Forbidden(
            "resource belongs to another Session owner".into(),
        ));
    }
    Ok(())
}

async fn ensure_local_session_target(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session: &AgentSessionId,
    operation: Option<&OperationId>,
) -> Result<(), AppError> {
    let observed = state
        .session_owner
        .canonical
        .get(&authenticated_principal(owner), session)
        .await?;
    if observed.session.remote_binding_provenance.is_some() {
        return Err(AppError::Forbidden(
            "Remote work requires its original domain Stop".into(),
        ));
    }
    let domain_bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation='attempt') \
        OR EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND relation='automation' AND active=1)")
        .bind(session.as_ref()).bind(session.as_ref()).fetch_one(&state.session_owner.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
    if domain_bound && operation.is_some() {
        return Err(AppError::Forbidden(
            "this work requires its exact task domain Stop".into(),
        ));
    }
    if operation.is_some_and(|operation| operation.as_ref().contains(":cron:")) {
        return Err(AppError::Forbidden(
            "scheduled work requires its original run Stop".into(),
        ));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct StopSessionCandidate {
    session_id: String,
    active_turn_id: Option<String>,
    head_seq: i64,
    remote: bool,
    remote_version: Option<i64>,
    domain_bound: bool,
    dedicated: bool,
    remote_pending: bool,
}

async fn process_target_at_cutoff_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session: &str,
    head_seq: i64,
    process: super::super::engine_process_host::ProcessInstanceObservation,
) -> Result<Option<Target>, AppError> {
    if process.execution_stopped {
        return Ok(None);
    }
    // The live owner proves the native capability and immutable creator call.
    // The same SQLite cutoff proves that its creator work was already accepted;
    // a process of a later Turn never enters a retry of this finite snapshot.
    let accepted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_events started \
        ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id AND started.kind='turn/started' \
        WHERE turn.session_id=? AND turn.operation_id=? AND started.seq<=?)")
        .bind(session).bind(&process.creator_turn_id).bind(head_seq).fetch_one(&mut **tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    Ok(accepted.then(|| Target::ProcessInstance {
        agent_session_id: session.into(),
        owner_generation: process.owner_generation,
        process_id: process.process_id,
        creator_turn_id: process.creator_turn_id,
        creator_call_id: process.creator_call_id,
    }))
}

/// Capture command identities in one database read snapshot. Runtime inventory
/// contributes only exact process-local resource instances; it grants no task
/// cancellation authority and cannot select a later Turn.
pub(super) async fn stop_targets(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
) -> Result<Json<ApiResponse<nomifun_api_types::AgentActivityStopSnapshot>>, NomiCoreApiError> {
    let planning_observations = state
        .session_owner
        .execution_progress
        .get()
        .and_then(std::sync::Weak::upgrade)
        .map(|engine| engine.planning_operations())
        .unwrap_or_default();
    let inventory = state
        .session_owner
        .runtime_sessions
        .registered_runtime_sessions()
        .unwrap_or_default()
        .into_iter()
        .map(|session| {
            let observed = state
                .session_owner
                .runtime_sessions
                .observe_runtime_carrier(&session);
            (session, observed)
        })
        .collect::<BTreeMap<_, _>>();
    let mut tx = state
        .session_owner
        .pool
        .begin()
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let sessions:Vec<StopSessionCandidate>=sqlx::query_as("SELECT session.agent_session_id AS session_id,head.active_turn_id,\
        COALESCE(head.last_seq,0) AS head_seq,session.remote_binding_id IS NOT NULL AS remote,session.remote_binding_version AS remote_version,\
        EXISTS(SELECT 1 FROM conversation_execution_links link WHERE link.conversation_id=session.agent_session_id \
            AND (link.relation='attempt' OR (link.relation='automation' AND link.active=1))) \
            OR COALESCE(instr(head.active_turn_id,':cron:'),0)>0 AS domain_bound,\
        EXISTS(SELECT 1 FROM conversation_execution_links link WHERE link.conversation_id=session.agent_session_id AND link.relation='attempt') AS dedicated,\
        EXISTS(SELECT 1 FROM nomi_remote_sessions remote WHERE remote.agent_session_id=session.agent_session_id AND remote.state='opening') AS remote_pending \
        FROM agent_sessions session \
        LEFT JOIN agent_session_heads head ON head.session_id=session.agent_session_id WHERE session.state='live' \
        AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?")
        .bind(owner.as_ref()).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    let captured_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| AppError::Internal(error.to_string()))?
        .as_millis()
        .try_into()
        .map_err(|_| AppError::Internal("activity snapshot clock overflow".into()))?;
    let mut targets = Vec::new();
    for session in sessions {
        let engines = state
            .session_owner
            .native_engines
            .get()
            .and_then(std::sync::Weak::upgrade);
        let session_id = AgentSessionId::from(session.session_id.clone());
        let knowledge_pending = engines
            .as_ref()
            .map(|engines| engines.observe_knowledge_resources(&session_id))
            .transpose()?
            .flatten()
            == Some(true);
        if let Some(engines) = &engines {
            for process in engines.process_resource_instances(owner.as_ref(), &session_id)? {
                if let Some(target) = process_target_at_cutoff_tx(
                    &mut tx,
                    &session.session_id,
                    session.head_seq,
                    process,
                )
                .await?
                {
                    targets.push(target);
                }
            }
            for operation in engines.running_knowledge_operations(&session_id)? {
                let accepted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=?)")
                    .bind(&session.session_id).bind(operation.as_ref()).fetch_one(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
                if accepted {
                    targets.push(Target::KnowledgeWriteback {
                        agent_session_id: session.session_id.clone(),
                        operation_id: operation.as_ref().into(),
                    });
                }
            }
        }
        if session.remote
            && !session.domain_bound
            && (session.active_turn_id.is_some() || session.remote_pending)
        {
            targets.push(Target::RemoteSession {
                agent_session_id: session.session_id.clone(),
                operation_id: session.active_turn_id.clone(),
                expected_head_seq: u64::try_from(session.head_seq)
                    .map_err(|_| AppError::Internal("invalid Remote activity head".into()))?,
                expected_binding_version: session.remote_version.ok_or_else(|| {
                    AppError::Conflict("Remote activity has no original binding version".into())
                })?,
            });
        } else if !session.remote && !session.domain_bound && session.active_turn_id.is_some() {
            targets.push(Target::SessionTurn {
                agent_session_id: session.session_id.clone(),
                operation_id: session.active_turn_id.unwrap(),
            });
        } else if !session.domain_bound
            && !knowledge_pending
            && inventory.contains_key(&session.session_id)
        {
            let carrier = &inventory[&session.session_id];
            if let Some(instance_id) = &carrier.instance_id {
                targets.push(Target::RuntimeCarrier {
                    agent_session_id: session.session_id.clone(),
                    instance_id: instance_id.clone(),
                    expected_generation: carrier
                        .active_generation
                        .or(carrier.constructing_generation),
                    expected_head_seq: u64::try_from(session.head_seq)
                        .map_err(|_| AppError::Internal("invalid activity head cursor".into()))?,
                });
            }
        }
        if !session.remote && !session.dedicated {
            for (input, _) in state
                .session_owner
                .input_preparation_observations(&session_id)?
            {
                let accepted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND event_id=? AND kind='message/user-accepted' AND kind_version=1 AND producer_id='session_api')")
                    .bind(&session.session_id).bind(input.as_ref()).fetch_one(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
                if accepted {
                    targets.push(Target::SessionInput {
                        agent_session_id: session.session_id.clone(),
                        message_id: input.as_ref().into(),
                    });
                }
            }
            let inputs:Vec<String>=sqlx::query_scalar("SELECT input.event_id FROM agent_events input WHERE input.session_id=? \
                AND input.kind='message/user-accepted' AND input.kind_version=1 AND input.producer_id='session_api' \
                AND input.seq>COALESCE((SELECT MAX(clear.seq) FROM agent_events clear WHERE clear.session_id=input.session_id AND clear.kind='context/cleared'),0) \
                AND NOT EXISTS(SELECT 1 FROM agent_turns turn WHERE turn.session_id=input.session_id AND turn.source_message_id=input.event_id) \
                AND NOT EXISTS(SELECT 1 FROM agent_events blocked WHERE blocked.session_id=input.session_id AND blocked.event_id='session-input-blocked:'||input.event_id||':cancelled_by_owner') \
                AND NOT EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads payload ON payload.session_id=claim.session_id AND payload.payload_id=claim.payload_id \
                    WHERE claim.session_id=input.session_id AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
                    AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(payload.body AS TEXT),'$.value.input_event_id'))=input.event_id)")
                .bind(&session.session_id).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
            targets.extend(inputs.into_iter().map(|message_id| Target::SessionInput {
                agent_session_id: session.session_id.clone(),
                message_id,
            }));
        }
    }
    let tasks:Vec<(String,String,String,i64,i64)>=sqlx::query_as("SELECT attempt.execution_id,attempt.step_id,attempt.attempt_id,execution.version,step.version \
        FROM agent_execution_attempts attempt JOIN agent_executions execution ON execution.execution_id=attempt.execution_id \
        JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
        WHERE execution.user_id=? AND step.kind='agent' AND attempt.status IN ('queued','running','waiting_input','interrupted') \
        AND COALESCE(json_extract(execution.initial_plan_input,'$.mode'),'')<>'automation' \
        AND attempt.attempt_no=(SELECT MAX(latest.attempt_no) FROM agent_execution_attempts latest WHERE latest.execution_id=attempt.execution_id AND latest.step_id=attempt.step_id)")
        .bind(owner.as_ref()).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    targets.extend(tasks.into_iter().map(
        |(execution_id, step_id, attempt_id, expected_execution_version, expected_step_version)| {
            Target::ExecutionTask {
                execution_id,
                step_id,
                attempt_id,
                expected_execution_version,
                expected_step_version,
            }
        },
    ));
    let steps:Vec<(String,String,i64,i64)>=sqlx::query_as("SELECT step.execution_id,step.step_id,execution.version,step.version \
        FROM agent_execution_steps step JOIN agent_executions execution ON execution.execution_id=step.execution_id \
        WHERE execution.user_id=? AND execution.status IN ('running','waiting_input','paused') AND step.kind='agent' AND step.status='pending' \
        AND step.superseded_in_revision IS NULL AND COALESCE(json_extract(execution.initial_plan_input,'$.mode'),'')<>'automation' \
        AND NOT EXISTS(SELECT 1 FROM agent_execution_attempts attempt WHERE attempt.execution_id=step.execution_id AND attempt.step_id=step.step_id \
            AND attempt.status IN ('queued','running','waiting_input','interrupted'))")
        .bind(owner.as_ref()).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    targets.extend(steps.into_iter().map(
        |(execution_id, step_id, expected_execution_version, expected_step_version)| {
            Target::ExecutionStep {
                execution_id,
                step_id,
                expected_execution_version,
                expected_step_version,
            }
        },
    ));
    let observed = serde_json::to_string(&planning_observations)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let planning:Vec<(String,i64)>=sqlx::query_as("SELECT execution_id,version FROM agent_executions WHERE user_id=? \
        AND (status='planning' OR EXISTS(SELECT 1 FROM json_each(?) observed \
          WHERE json_extract(observed.value,'$[0]')=execution_id AND json_extract(observed.value,'$[1]') BETWEEN 0 AND version)) \
        AND COALESCE(json_extract(initial_plan_input,'$.mode'),'')<>'automation'")
        .bind(owner.as_ref()).bind(observed).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    targets.extend(
        planning
            .into_iter()
            .map(|(execution_id, expected_version)| Target::ExecutionGroup {
                execution_id,
                expected_version,
            }),
    );
    let cron:Vec<(String,String)>=sqlx::query_as("SELECT reservation.cron_job_id,reservation.cron_job_run_id FROM cron_run_reservations reservation \
        LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
        LEFT JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
        WHERE COALESCE(job.user_id,CASE WHEN json_extract(session.owner_ref_json,'$.principal_kind')='user' THEN json_extract(session.owner_ref_json,'$.principal_id') END)=? \
        AND reservation.status='reserved'")
        .bind(owner.as_ref()).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    targets.extend(
        cron.into_iter()
            .map(|(cron_job_id, cron_job_run_id)| Target::CronRun {
                cron_job_id,
                cron_job_run_id,
            }),
    );
    let automatic:Vec<(String,String,i64)>=sqlx::query_as("SELECT requirement.owner_conversation_id,requirement.requirement_id,requirement.claim_generation FROM requirements requirement \
        JOIN agent_sessions session ON session.agent_session_id=requirement.owner_conversation_id WHERE requirement.status='in_progress' \
        AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?")
        .bind(owner.as_ref()).fetch_all(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
    targets.extend(
        automatic
            .into_iter()
            .map(
                |(target_id, requirement_id, claim_generation)| Target::AutoWorkRun {
                    target_kind: nomifun_api_types::AutoWorkTargetKind::Conversation,
                    target_id,
                    requirement_id,
                    claim_generation,
                },
            ),
    );
    tx.commit()
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut identities = BTreeSet::new();
    targets.retain(|target| {
        identities.insert(
            serde_json::to_string(target).expect("typed activity target serialization cannot fail"),
        )
    });
    Ok(Json(ApiResponse::ok(
        nomifun_api_types::AgentActivityStopSnapshot {
            captured_at_ms,
            targets,
        },
    )))
}
