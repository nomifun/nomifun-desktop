//! Resolve terminal content from the exact canonical event inside the reader's
//! transaction. Turn and Effect rows retain identity and lifecycle indexes only.
use super::*;

async fn event(
    tx: &mut Transaction<'_, Sqlite>, id: &str, session: &str, correlation: &str,
) -> Result<SessionEventRecord, SessionStoreError> {
    let event = event_by_event_id_tx(tx, id).await?.map(event_from_row).transpose()?
        .ok_or_else(|| SessionStoreError::InvalidEvent("canonical lifecycle event is missing".into()))?;
    if event.agent_session_id.as_ref() != session || event.correlation_id.as_ref() != correlation || event.kind_version != 1 {
        return Err(SessionStoreError::InvalidEvent("canonical lifecycle event identity differs from its index".into()));
    }
    Ok(event)
}

pub(super) async fn turn_payload(
    tx: &mut Transaction<'_, Sqlite>, row: &StoredTurnHistoryRow,
) -> Result<Option<Value>, SessionStoreError> {
    if row.turn_id != row.operation_id {
        return Err(SessionStoreError::InvalidSession("Turn history operation identity differs".into()));
    }
    let started = event(tx, &row.started_event_id, &row.session_id, &row.operation_id).await?;
    if started.kind.0 != "turn/started" || as_i64(started.seq, "Turn start sequence")? != row.started_at.unwrap_or(row.accepted_at) {
        return Err(SessionStoreError::InvalidSession("Turn history start differs from its canonical event".into()));
    }
    let start = payload_value_for_event_tx(tx, &started).await?;
    if start.get("source_message_id").or_else(|| start.get("message_id")).and_then(Value::as_str)
        != Some(row.source_message_id.as_str()) {
        return Err(SessionStoreError::InvalidSession("Turn history source differs from its canonical start".into()));
    }
    let expected_kind = match row.state.as_str() {
        "accepted" | "running" => {
            if row.terminal_event_id.is_some() || row.finished_at.is_some() {
                return Err(SessionStoreError::InvalidSession("active Turn has a terminal index".into()));
            }
            return Ok(None);
        }
        "completed" => "turn/completed",
        "failed" => "turn/failed",
        "cancelled" | "interrupted" => "turn/cancelled",
        _ => return Err(SessionStoreError::InvalidSession("Turn history state is invalid".into())),
    };
    let id = row.terminal_event_id.as_deref()
        .ok_or_else(|| SessionStoreError::InvalidSession("closed Turn has no canonical terminal reference".into()))?;
    let terminal = event(tx, id, &row.session_id, &row.operation_id).await?;
    if terminal.kind.0 != expected_kind || terminal.seq <= started.seq
        || Some(as_i64(terminal.seq, "Turn terminal sequence")?) != row.finished_at {
        return Err(SessionStoreError::InvalidSession("Turn terminal differs from its lifecycle index".into()));
    }
    let payload = payload_value_for_event_tx(tx, &terminal).await?;
    if !payload.is_object() {
        return Err(SessionStoreError::InvalidPayload("Turn terminal content must be an object".into()));
    }
    Ok(Some(payload))
}

fn effect_identity(row: &StoredEffectRow, payload: &Value) -> Result<(), SessionStoreError> {
    for (field, expected) in [
        ("effect_id", row.effect_id.as_str()), ("turn_id", row.turn_id.as_str()),
        ("operation_id", row.operation_id.as_str()), ("owner_domain", row.owner_domain.as_str()),
        ("capability_module", row.capability_module.as_str()), ("action_id", row.action_id.as_str()),
        ("input_digest", row.input_digest.as_str()), ("strategy", row.strategy.as_str()),
    ] {
        if payload.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(SessionStoreError::InvalidEvent(format!("Effect canonical identity differs at {field}")));
        }
    }
    for (field, expected) in [("resource_binding_id", row.resource_binding_id.as_deref()), ("resource_key", row.resource_key.as_deref())] {
        if payload.get(field).and_then(Value::as_str) != expected {
            return Err(SessionStoreError::InvalidEvent(format!("Effect canonical resource identity differs at {field}")));
        }
    }
    Ok(())
}

pub(super) async fn effect_observation(
    tx: &mut Transaction<'_, Sqlite>, row: &StoredEffectRow,
) -> Result<Option<Value>, SessionStoreError> {
    let started = event(tx, &row.started_event_id, &row.session_id, &row.effect_id).await?;
    let start = payload_value_for_event_tx(tx, &started).await?;
    effect_identity(row, &start)?;
    effect_native_targets::from_payload(&start)?;
    if started.kind.0 != "effect/started" || start.get("recorded_at").and_then(Value::as_i64) != Some(row.created_at) {
        return Err(SessionStoreError::InvalidEvent("Effect start differs from its lifecycle index".into()));
    }
    if row.state == "pending" {
        if row.terminal_event_id.is_some() || row.settled_at.is_some() {
            return Err(SessionStoreError::InvalidEvent("pending Effect has a terminal index".into()));
        }
        return Ok(None);
    }
    let id = row.terminal_event_id.as_deref()
        .ok_or_else(|| SessionStoreError::InvalidEvent("settled Effect has no canonical terminal reference".into()))?;
    let terminal = event(tx, id, &row.session_id, &row.effect_id).await?;
    if terminal.seq <= started.seq || terminal.idempotency_key != started.idempotency_key {
        return Err(SessionStoreError::InvalidEvent("Effect terminal order or command identity differs".into()));
    }
    let payload = payload_value_for_event_tx(tx, &terminal).await?;
    effect_identity(row, &payload)?;
    effect_native_targets::validate_settlement(&start, &payload)?;
    let state = match terminal.kind.0.as_str() {
        "effect/succeeded" => "returned",
        "effect/failed" => "rejected",
        "effect/uncertain" => "unknown",
        "effect/reconciled" => match payload.get("outcome").and_then(Value::as_str) {
            Some("confirmed_succeeded") => "returned",
            Some("confirmed_failed") => "rejected",
            Some("still_uncertain") => "unknown",
            _ => return Err(SessionStoreError::InvalidEvent("Effect reconciliation outcome is invalid".into())),
        },
        _ => return Err(SessionStoreError::InvalidEvent("Effect terminal event kind is invalid".into())),
    };
    if state != row.state || payload.get("recorded_at").and_then(Value::as_i64) != row.settled_at {
        return Err(SessionStoreError::InvalidEvent("Effect terminal differs from its lifecycle index".into()));
    }
    Ok(Some(payload))
}
