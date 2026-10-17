//! Typed, exact-source delivery of a settled desktop delegation into its Robot
//! Session. This appends one existing assistant projection, never another Turn.

use super::{
    NomiCoreSessionOwner, agent_session_store_error, canonical_companion_origin,
    frozen_workspace_root,
};
use nomifun_agent_contracts::{
    AgentSessionId, CompanionSessionRole, CorrelationId, EventId, EventProducerId, IdempotencyKey,
    OperationId, PrincipalRef, SemanticSessionEventDraft, SessionEventAppend, SessionEventKind,
    SessionEventPayloadRef, StrictJsonValue,
};
use nomifun_agent_execution::{DirectedDelegationReport, DirectedDelegationSource};
use nomifun_agent_session::{AgentEffectState, SessionStoreError};
use nomifun_common::AppError;
use nomifun_db::sqlx;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

const REPORT_ORIGIN: &str = "agent_execution_report";

fn invalid(detail: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("directed delegation report: {detail}"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectedReport {
    content: String,
    origin: String,
    operation_id: String,
    delegation_report: DirectedDelegationReport,
}

#[derive(sqlx::FromRow)]
struct ExistingProjection {
    session_id: String,
    event_id: String,
    kind: String,
    kind_version: i64,
    correlation_id: String,
    causation_event_id: Option<String>,
    inline_json: Option<String>,
    payload_id: Option<String>,
}

/// File verification is an observation made at first delivery. Later retries
/// retain that original observation even when files have subsequently changed.
fn same_report_identity(left: &DirectedDelegationReport, right: &DirectedDelegationReport) -> bool {
    if left.attempts.len() != right.attempts.len() {
        return false;
    }
    let mut normalized = right.clone();
    for (original, retry) in left.attempts.iter().zip(&mut normalized.attempts) {
        retry.output_files = original.output_files.clone();
        retry.output_integrity_ok = original.output_integrity_ok;
    }
    *left == normalized
}

async fn existing_projection(
    owner: &NomiCoreSessionOwner,
    key: &str,
    conversation_id: &str,
    cause: &str,
    expected: &ProjectedReport,
) -> Result<Option<String>, AppError> {
    let existing = sqlx::query_as::<_, ExistingProjection>(
        "SELECT session_id,event_id,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
         FROM agent_events WHERE producer_id='session_api' AND idempotency_key=?",
    )
    .bind(key)
    .fetch_optional(&owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.session_id != conversation_id
        || existing.kind != "message/assistant-projected"
        || existing.kind_version != 1
        || existing.correlation_id != existing.event_id
        || existing.causation_event_id.as_deref() != Some(cause)
        || existing.payload_id.is_some()
    {
        return Err(invalid(
            "report idempotency identity differs from its committed event",
        ));
    }
    let stored: ProjectedReport = serde_json::from_str(
        existing
            .inline_json
            .as_deref()
            .ok_or_else(|| invalid("committed report payload is missing"))?,
    )
    .map_err(invalid)?;
    if stored.content != expected.content
        || stored.origin != expected.origin
        || stored.operation_id != expected.operation_id
        || !same_report_identity(&stored.delegation_report, &expected.delegation_report)
    {
        return Err(invalid("report replay changed its committed payload"));
    }
    Ok(Some(existing.event_id))
}

async fn validate_execution(
    owner: &NomiCoreSessionOwner,
    owner_id: &str,
    operation_id: &str,
    report: &DirectedDelegationReport,
) -> Result<(), AppError> {
    let initial: Option<String> = sqlx::query_scalar(
        "SELECT initial_plan_input FROM agent_executions \
         WHERE execution_id=? AND user_id=? AND deleted_at IS NULL",
    )
    .bind(&report.execution_id)
    .bind(owner_id)
    .fetch_optional(&owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let initial: Value =
        serde_json::from_str(&initial.ok_or_else(|| invalid("owned Execution is missing"))?)
            .map_err(invalid)?;
    if initial.get("mode").and_then(Value::as_str) != Some("directed") {
        return Err(invalid("Execution was not created by directed delegation"));
    }
    let source: DirectedDelegationSource = serde_json::from_value(
        initial
            .get("source")
            .cloned()
            .ok_or_else(|| invalid("Execution source is missing"))?,
    )
    .map_err(invalid)?;
    if source != report.source {
        return Err(invalid(
            "Execution source differs from the exact original tool Effect",
        ));
    }
    let linked: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
         WHERE execution_id=? AND conversation_id=? AND relation='lead')",
    )
    .bind(&report.execution_id)
    .bind(&report.source.agent_session_id)
    .fetch_one(&owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if linked == 0 {
        return Err(invalid("Execution has no exact source lead link"));
    }
    let terminal: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM agent_execution_events WHERE execution_id=? AND sequence=? \
         AND event_type='status_changed' AND on_behalf_of_user_id=?",
    )
    .bind(&report.execution_id)
    .bind(report.execution_event_sequence)
    .bind(owner_id)
    .fetch_optional(&owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let terminal: Value = serde_json::from_str(
        &terminal.ok_or_else(|| invalid("exact terminal Execution event is missing"))?,
    )
    .map_err(invalid)?;
    if terminal.get("status").and_then(Value::as_str) != Some(report.status.as_str())
        || terminal
            .get("lead_report_operation_id")
            .and_then(Value::as_str)
            != Some(operation_id)
    {
        return Err(invalid(
            "terminal Execution event differs from the requested report epoch",
        ));
    }
    Ok(())
}

async fn validate_attempts(
    owner: &NomiCoreSessionOwner,
    principal: &PrincipalRef,
    report: &DirectedDelegationReport,
    verify_files: bool,
) -> Result<(), AppError> {
    let mut seen = BTreeSet::new();
    for attempt in &report.attempts {
        if !seen.insert(attempt.attempt_id.as_str()) {
            return Err(invalid("report repeats an Attempt"));
        }
        let linked: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_execution_attempts attempt \
             JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
             JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id \
                 AND link.step_id=attempt.step_id AND link.attempt_id=attempt.attempt_id \
             WHERE attempt.execution_id=? AND attempt.step_id=? AND attempt.attempt_id=? \
                 AND attempt.status IN ('completed','failed','cancelled','interrupted') \
                 AND attempt.finished_at IS NOT NULL AND link.relation='attempt' AND link.conversation_id=?)",
        )
        .bind(&report.execution_id)
        .bind(&attempt.step_id)
        .bind(&attempt.attempt_id)
        .bind(&attempt.agent_session_id)
        .fetch_one(&owner.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if linked == 0 {
            return Err(invalid(
                "Attempt has no exact settled execution/step/Session link",
            ));
        }
        let session_id = AgentSessionId::from(attempt.agent_session_id.clone());
        let operation = OperationId::from(attempt.turn_operation_id.clone());
        let facts = owner
            .canonical
            .store()
            .turn_output_facts(&session_id, &operation)
            .await
            .map_err(agent_session_store_error)?;
        if facts.session.owner_ref != *principal {
            return Err(invalid("Attempt Session has another owner"));
        }
        let receipt = owner
            .canonical
            .store()
            .read_turn_receipt(&session_id, &operation)
            .await
            .map_err(agent_session_store_error)?;
        if receipt
            .terminal_event
            .as_ref()
            .map(|event| event.event_id.as_ref())
            != Some(attempt.terminal_event_id.as_str())
        {
            return Err(invalid(
                "Attempt report does not reference its exact canonical terminal event",
            ));
        }
        let delivery = nomifun_agent_execution::canonical_turn_delivery(&facts, &receipt, true)?;
        if !delivery.completed || delivery.result_ok != attempt.result_ok {
            return Err(invalid(
                "Attempt result differs from its canonical Turn delivery",
            ));
        }
        if verify_files {
            let workspace = frozen_workspace_root(
                &owner.managed_workspace_root,
                &principal.principal_id,
                &session_id,
                &facts.session.agent_binding,
            )?;
            let verified = tokio::task::spawn_blocking(move || {
                nomifun_agent_execution::canonical_turn_output_files(
                    &facts,
                    &receipt,
                    workspace.as_deref().map(std::path::Path::new),
                )
            })
            .await
            .map_err(|error| {
                AppError::Internal(format!("verify directed report output: {error}"))
            })?;
            let (files, integrity) = match verified {
                Ok(files) => (files, true),
                Err(_) => (Vec::new(), false),
            };
            if integrity != attempt.output_integrity_ok || files != attempt.output_files {
                return Err(invalid(
                    "Attempt output changed before its first report was committed",
                ));
            }
        }
    }
    if report.status == "completed"
        && !report
            .attempts
            .iter()
            .any(|attempt| attempt.result_ok == Some(true))
    {
        return Err(invalid(
            "completed report has no exact successful Attempt delivery",
        ));
    }
    Ok(())
}

pub(super) async fn project_directed_report(
    owner: &NomiCoreSessionOwner,
    owner_id: &str,
    conversation_id: &str,
    operation_id: &str,
    content: &str,
    report: &DirectedDelegationReport,
) -> Result<String, AppError> {
    if content.trim().is_empty() || content.len() > 1024 * 1024 {
        return Err(AppError::BadRequest(
            "directed report content must be non-empty and bounded".into(),
        ));
    }
    if conversation_id != report.source.agent_session_id
        || report.execution_event_sequence <= 0
        || !matches!(
            report.status.as_str(),
            "completed" | "completed_with_failures" | "failed" | "cancelled"
        )
        || operation_id
            != format!(
                "exec-lead-report:{}:event:{}",
                report.execution_id, report.execution_event_sequence
            )
    {
        return Err(invalid(
            "report has inconsistent source, terminal status or operation identity",
        ));
    }
    let session_id = AgentSessionId::from(conversation_id.to_owned());
    let principal = PrincipalRef {
        principal_kind: "user".into(),
        principal_id: owner_id.into(),
    };
    let observation = owner.canonical.get(&principal, &session_id).await?;
    if !canonical_companion_origin(owner_id, &observation)?
        .is_some_and(|origin| origin.role == CompanionSessionRole::Robot)
    {
        return Err(invalid(
            "source Session is not the owner's canonical Robot role",
        ));
    }
    let effect = owner
        .canonical
        .store()
        .read_effect(&session_id, &report.source.effect_id)
        .await
        .map_err(agent_session_store_error)?
        .ok_or_else(|| invalid("source delegation Effect is missing"))?;
    if effect.turn_id.as_ref() != report.source.turn_operation_id
        || effect.operation_id.as_ref() != report.source.tool_operation_id
        || effect.owner_domain != "wave5.product"
        || effect.capability_module.as_ref() != "agent.collaboration"
        || effect.action_id.as_ref() != "agent/delegate"
        || effect.state != AgentEffectState::Returned
    {
        return Err(invalid(
            "source Effect differs from the exact completed delegation call",
        ));
    }
    // The original source Turn/tool remain authoritative after later inputs.
    let source_receipt = owner
        .canonical
        .store()
        .read_turn_receipt(&session_id, &effect.turn_id)
        .await
        .map_err(agent_session_store_error)?;
    let source_start = source_receipt
        .started_event
        .as_ref()
        .ok_or_else(|| invalid("source Turn has no canonical start"))?;
    let source_terminal = source_receipt
        .terminal_event
        .as_ref()
        .ok_or_else(|| invalid("source Turn has not reached its canonical terminal boundary"))?;
    // The normal Effect-admission helper requires a still-active Turn. Reports
    // instead verify the original committed tool cause after that Turn closes.
    let started_matches: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_events started \
         JOIN agent_events tool ON tool.session_id=started.session_id AND tool.event_id=started.causation_event_id \
         WHERE started.session_id=? AND started.event_id=? AND started.kind='effect/started' \
         AND started.kind_version=1 AND started.correlation_id=? \
         AND json_extract(started.inline_json,'$.turn_id')=? AND json_extract(started.inline_json,'$.operation_id')=? \
         AND tool.kind='tool/call-started' AND tool.kind_version=1 AND tool.causation_event_id=? \
         AND json_extract(tool.inline_json,'$.operation_id')=? \
         AND json_extract(tool.inline_json,'$.capability_id')='agent.collaboration' \
         AND json_extract(tool.inline_json,'$.action_id')='agent/delegate')",
    )
    .bind(conversation_id).bind(effect.started_event_id.as_ref()).bind(&report.source.effect_id)
    .bind(&report.source.turn_operation_id).bind(&report.source.tool_operation_id)
    .bind(source_start.event_id.as_ref()).bind(&report.source.tool_operation_id)
    .fetch_one(&owner.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
    if started_matches == 0 {
        return Err(invalid(
            "source Effect start differs from its canonical tool cause",
        ));
    }
    validate_execution(owner, owner_id, operation_id, report).await?;
    let expected = ProjectedReport {
        content: content.into(),
        origin: REPORT_ORIGIN.into(),
        operation_id: operation_id.into(),
        delegation_report: report.clone(),
    };
    let key = format!("agent-execution-projection:{operation_id}");
    let cause = source_terminal.event_id.as_ref();
    let existing = existing_projection(owner, &key, conversation_id, cause, &expected).await?;
    // Desktop file hashing must not occupy the Robot's input/metadata fence.
    validate_attempts(owner, &principal, report, existing.is_none()).await?;
    let _fence = owner
        .session_operation_lock(conversation_id)
        .write_owned()
        .await;
    let current = owner.canonical.get(&principal, &session_id).await?;
    if !canonical_companion_origin(owner_id, &current)?
        .is_some_and(|origin| origin.role == CompanionSessionRole::Robot)
    {
        return Err(invalid(
            "source Session no longer has its canonical Robot role",
        ));
    }
    validate_execution(owner, owner_id, operation_id, report).await?;
    if let Some(existing) =
        existing_projection(owner, &key, conversation_id, cause, &expected).await?
    {
        return Ok(existing);
    }
    let payload =
        serde_json::to_value(&expected).map_err(|error| AppError::Internal(error.to_string()))?;
    if serde_json::to_vec(&payload)
        .map_err(|error| AppError::Internal(error.to_string()))?
        .len()
        > nomifun_agent_session::MAX_INLINE_JSON_BYTES
    {
        return Err(AppError::BadRequest(
            "directed report exceeds the canonical inline payload budget".into(),
        ));
    }
    let message_id = uuid::Uuid::now_v7().to_string();
    let appended = owner
        .canonical
        .store()
        .append_event(&SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: EventId::from(message_id.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(key.clone()),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/assistant-projected".into()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id.clone()),
                causation_event_id: Some(source_terminal.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
            },
        })
        .await;
    match appended {
        Ok(_) => {}
        Err(SessionStoreError::IdempotencyConflict(_)) => {
            return existing_projection(owner, &key, conversation_id, cause, &expected)
                .await?
                .ok_or_else(|| invalid("report append raced with a different canonical event"));
        }
        Err(error) => return Err(agent_session_store_error(error)),
    }
    owner.user_events.send_to_user(
        owner_id,
        NomiCoreSessionOwner::canonical_projected_assistant_wire_event(
            &session_id,
            &message_id,
            content,
        ),
    );
    Ok(message_id)
}
