//! Canonical input provenance for the installed Companion memory writer.
//! SQL selects exact identities only; the Session Store validates the Effect,
//! native causation and input payloads. No transcript or recovery lease is read.
use nomifun_agent_contracts::{AgentInputSource, CompanionSessionOrigin, CompanionSessionRole};
use nomifun_agent_domain_wave1::{Wave1HostContext, Wave1HostPortError};
use nomifun_agent_session::{AgentEffectState, AgentSessionStore, SessionObservation};
use nomifun_companion::store::CompanionMemoryMessageSource;
use nomifun_db::{SqlitePool, sqlx};
use serde::Deserialize;
use std::collections::BTreeSet;

fn invalid(error: impl std::fmt::Display) -> Wave1HostPortError {
    Wave1HostPortError::new("COMPANION_MEMORY_SOURCE_UNAVAILABLE", error.to_string())
}

#[derive(Deserialize)]
struct CanonicalInput {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    input_source: Option<AgentInputSource>,
}


fn is_human(input: &CanonicalInput, origin: Option<&CompanionSessionOrigin>, companion: &str) -> bool {
    if input.hidden { return false; }
    match (input.origin.as_deref().filter(|value| !value.is_empty()), &input.input_source) {
        (None | Some("mobile_voice"), None) => true,
        (Some("robot"), Some(AgentInputSource::AuthenticatedRobot {companion_id, robot_id, connection_id, ..})) =>
            origin.is_some_and(|scope| scope.role == CompanionSessionRole::Robot
                && scope.companion_id == companion && companion_id == companion
                && scope.robot_id.as_deref() == Some(robot_id.as_str()) && !connection_id.trim().is_empty()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn established_mobile_voice_is_only_a_nonhidden_memory_input_source() {
        let input = |hidden: bool, origin: &str, input_source: serde_json::Value| serde_json::from_value::<CanonicalInput>(
            json!({"hidden":hidden,"origin":origin,"input_source":input_source})).unwrap();
        assert!(is_human(&input(false,"mobile_voice",serde_json::Value::Null),None,"companion"));
        assert!(!is_human(&input(true,"mobile_voice",serde_json::Value::Null),None,"companion"));
        for origin in ["idmm","cron","companion","mobile_voice "] {
            assert!(!is_human(&input(false,origin,serde_json::Value::Null),None,"companion"));
        }
        // An origin label cannot override a typed device/host source. This
        // classifier does not establish product scope or physical authority.
        let robot = json!({"kind":"authenticated_robot","companion_id":"companion",
            "robot_id":"robot","connection_id":"original-connection","presented_question_id":null});
        assert!(!is_human(&input(false,"mobile_voice",robot),None,"companion"));
    }
}

pub(super) async fn resolve(
    store: &AgentSessionStore,
    pool: &SqlitePool,
    context: &Wave1HostContext,
) -> Result<Vec<CompanionMemoryMessageSource>, Wave1HostPortError> {
    use super::super::agent_memory_authority::{authorize_memory_resource, ProductMemoryAction};
    if context.capability_id.as_ref() != nomifun_agent_domain_wave1::COMPANION_MEMORY_MODULE_ID
        || context.action_id.as_ref() != nomifun_agent_domain_wave1::COMPANION_MEMORY_WRITE_ACTION_ID {
        return Err(invalid("memory provenance requires the exact installed write Action"));
    }
    let binding = authorize_memory_resource(&context.principal.principal_id,
        ProductMemoryAction::CompanionWrite, &context.resource_bindings).map_err(invalid)?;
    let companion = binding.resource_id.as_ref();
    let session = store.get_live_session(&context.agent_session_id).await.map_err(invalid)?;
    if session.owner_ref != context.principal || context.principal.principal_kind != "user"
        || session.agent_binding.resolved_snapshot_ref != context.resolved_snapshot_ref {
        return Err(invalid("memory source differs from the current canonical owner or binding"));
    }
    let identities: Vec<String> = sqlx::query_scalar(
        "SELECT effect_id FROM agent_effects WHERE session_id=? AND operation_id=? \
         AND capability_module=? AND action_id=? LIMIT 2")
        .bind(context.agent_session_id.as_ref()).bind(context.operation_id.as_ref())
        .bind(context.capability_id.as_ref()).bind(context.action_id.as_ref())
        .fetch_all(pool).await.map_err(invalid)?;
    if identities.len() != 1 { return Err(invalid("memory write requires one exact canonical Effect")); }
    let effect = store.read_effect(&context.agent_session_id, &identities[0]).await.map_err(invalid)?
        .ok_or_else(|| invalid("memory write Effect disappeared"))?;
    if effect.agent_session_id != context.agent_session_id || effect.turn_id != context.turn_id || effect.operation_id != context.operation_id
        || effect.capability_module != context.capability_id || effect.action_id != context.action_id
        || effect.state != AgentEffectState::Pending {
        return Err(invalid("memory write does not own its pending Effect"));
    }
    let causation = store.native_tool_call_causation(&context.principal, &context.agent_session_id,
        &effect.turn_id, &context.operation_id, &context.capability_id, &context.action_id).await.map_err(invalid)?;
    if causation.execution_scope.resolved_snapshot_ref() != &context.resolved_snapshot_ref {
        return Err(invalid("memory write native binding changed"));
    }
    let (root, payload) = store.turn_accepted_input(&context.agent_session_id, &effect.turn_id).await.map_err(invalid)?;
    if root.seq >= causation.tool_call.seq { return Err(invalid("memory source follows its tool call")); }
    let opening = store.read_events(&context.agent_session_id, None, 1).await.map_err(invalid)?;
    let head = store.head(&context.agent_session_id).await.map_err(invalid)?;
    let observation = SessionObservation {session, head, events:opening.events,
        messages:Vec::new(), next_cursor:opening.next_cursor, active_turn_outcome:None};
    let origin = super::super::nomi_core_session::canonical_companion_origin(
        &context.principal.principal_id, &observation).map_err(invalid)?;
    let input: CanonicalInput = serde_json::from_value(payload.0).map_err(invalid)?;
    let mut sources = BTreeSet::new();
    if is_human(&input, origin.as_ref(), companion) {
        // turn_accepted_input proves event_id == source_message_id == the
        // canonical user message identity, including stored-payload integrity.
        sources.insert((context.agent_session_id.as_ref().to_owned(), root.event_id.as_ref().to_owned()));
    } else {
        let related = super::super::engine_session_host::resolve_companion_turn_human_sources(
            pool, store, &context.principal, companion, &context.agent_session_id,
            effect.turn_id.as_ref(), causation.tool_call.seq).await.map_err(invalid)?
            .ok_or_else(|| invalid("this host input has no proven human Companion source"))?;
        sources.extend(related.into_iter().map(|source| (source.session_id, source.message_id)));
    }
    // Bound the exact Turn read before resolving its payloads. This is neither
    // an all-Session scan nor an authority to recover a closed native worker.
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events e JOIN agent_turns t ON t.session_id=e.session_id \
         JOIN agent_events started ON started.event_id=t.started_event_id \
         WHERE t.session_id=? AND t.operation_id=? AND e.seq>=started.seq")
        .bind(context.agent_session_id.as_ref()).bind(effect.turn_id.as_ref())
        .fetch_one(pool).await.map_err(invalid)?;
    if event_count > 4096 { return Err(invalid("memory source Turn exceeds its bounded provenance window")); }
    let facts = store.turn_output_facts(&context.agent_session_id, &effect.turn_id).await.map_err(invalid)?;
    let started = facts.events.iter().find(|event| event.event_id == effect.started_event_id)
        .ok_or_else(|| invalid("memory Effect has no canonical start event"))?;
    if started.kind.0 != "effect/started" || started.kind_version != 1
        || started.producer_id.as_ref() != "capability_host" || started.correlation_id.as_ref() != effect.effect_id
        || started.causation_event_id.as_ref() != Some(&causation.tool_call.event_id) {
        return Err(invalid("memory Effect does not descend from its exact tool call"));
    }
    let identity = facts.event_payloads.get(started.event_id.as_ref())
        .ok_or_else(|| invalid("memory Effect start payload is unavailable"))?;
    for (field, expected) in [("effect_id",effect.effect_id.as_str()), ("turn_id",effect.turn_id.as_ref()),
        ("operation_id",context.operation_id.as_ref()), ("capability_module",context.capability_id.as_ref()),
        ("action_id",context.action_id.as_ref()), ("input_digest",effect.input_digest.as_ref())] {
        if identity.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(invalid(format!("memory Effect start differs at {field}")));
        }
    }
    let mut steering_count = 0;
    for event in &facts.events {
        if event.kind.0 != "turn/steer-accepted" || event.seq >= causation.tool_call.seq { continue; }
        if !matches!(event.kind_version,1|2) || event.producer_id.as_ref() != "session_api"
            || event.correlation_id.as_ref() != effect.turn_id.as_ref() {
            return Err(invalid("steering source does not belong to this exact Turn"));
        }
        steering_count += 1;
        if steering_count > 16 { return Err(invalid("memory source has too many accepted steering inputs")); }
        let (source_id, payload) = nomifun_agent_session::canonical_steering_input(&facts,event).map_err(invalid)?;
        let input:CanonicalInput=serde_json::from_value(payload.0).map_err(invalid)?;
        if is_human(&input, origin.as_ref(), companion) {
            sources.insert((context.agent_session_id.as_ref().to_owned(), source_id.as_ref().to_owned()));
        }
    }
    if sources.is_empty() || sources.len() > 64 { return Err(invalid("memory source cannot prove a bounded human input set")); }
    Ok(sources.into_iter().map(|(session_id, message_id)| CompanionMemoryMessageSource {session_id, message_id}).collect())
}
