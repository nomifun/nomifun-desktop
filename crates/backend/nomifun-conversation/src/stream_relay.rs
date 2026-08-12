use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::Duration;

use futures_util::FutureExt;
use nomifun_ai_agent::{
    AgentSendError, AgentStreamEvent,
    artifact_store::{
        ArtifactRecoveryEnvelope, ArtifactRecoveryOwner, ArtifactRecoverySource,
        ArtifactRecoveryState, ArtifactStore, PersistedArtifact,
    },
    protocol::events::{
        FinishEventData, PlanEventData, TextEventData, ThinkingEventData, TurnStopReason,
        tool_call::{
            AcpToolCallSessionUpdateKind, AcpToolCallStatus, ToolCallEventData,
            ToolCallStatus, validate_artifact_receipt_integrity,
            validate_completed_artifact_contract,
        },
    },
};

use crate::response_middleware::{ICronService, MessageMiddleware, MiddlewareResult};
use crate::runtime_state::{AgentTurnCancellation, ConversationRuntimeStateService};
use nomifun_api_types::{AgentErrorCode, ConversationRuntimeSummary, WebSocketMessage};
use nomifun_common::{
    CompanionId, ErrorChain, MessageId, generate_id, normalize_keys_to_snake_case, now_ms,
    stage_direction::StageDirectionFilter,
};

use crate::service::ConversationService;
use nomifun_db::{
    DbError, IConversationRepository, MessageRowUpdate, SortOrder, TurnArtifactMessageCommit,
};
use nomifun_db::models::MessageRow;
use nomifun_realtime::UserEventSink;
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, Notify, broadcast, oneshot};
use tracing::{debug, error, info, warn};

/// Number of text chunks to accumulate before flushing to the database.
const FLUSH_INTERVAL: u32 = 20;
const MAX_TERMINAL_ACTIVE_ITEMS: usize = 256;
const ARTIFACT_DELIVERY_COMMITTED_FIELD: &str = "artifact_delivery_committed";
const ARTIFACT_DELIVERY_PENDING_OUTPUT: &str =
    "Artifact delivery is pending final turn validation";

/// A relay owns the producer-side in-process leases for its exact wire until
/// every terminal path (including task cancellation/drop) exits. Dropping the
/// guard only relinquishes the lease: the durable journal remains the sole
/// recovery owner and the next relay must still acquire and reconcile it.
struct ArtifactRecoveryLeaseHandoff {
    store: Option<ArtifactStore>,
    source: ArtifactRecoverySource,
}

impl ArtifactRecoveryLeaseHandoff {
    fn new(workspace: Option<&PathBuf>, conversation_id: &str, wire_msg_id: &str) -> Self {
        Self {
            store: workspace.map(ArtifactStore::new),
            source: ArtifactRecoverySource {
                conversation_id: conversation_id.to_owned(),
                wire_msg_id: wire_msg_id.to_owned(),
            },
        }
    }
}

impl Drop for ArtifactRecoveryLeaseHandoff {
    fn drop(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if let Err(error) = store.release_recovery_leases_for_source(&self.source) {
            warn!(
                error = %error,
                conversation_id = self.source.conversation_id,
                wire_msg_id = self.source.wire_msg_id,
                "Could not hand artifact recovery leases to the next relay"
            );
        }
    }
}

fn track_bounded<V>(map: &mut HashMap<String, V>, key: String, value: V, kind: &'static str) -> bool {
    if map.contains_key(&key) || map.len() < MAX_TERMINAL_ACTIVE_ITEMS {
        map.insert(key, value);
        true
    } else {
        warn!(kind, max = MAX_TERMINAL_ACTIVE_ITEMS, "Relay terminal tracking limit reached");
        false
    }
}

fn remember_bounded(set: &mut HashSet<String>, value: String, kind: &'static str) -> bool {
    if set.contains(&value) || set.len() < MAX_TERMINAL_ACTIVE_ITEMS {
        set.insert(value);
        true
    } else {
        warn!(kind, max = MAX_TERMINAL_ACTIVE_ITEMS, "Relay terminal deduplication limit reached");
        false
    }
}

/// Apply the normalized ToolCall artifact contract to an externally-produced
/// ACP update. Only locally verified `Artifact` receipts count; a remote
/// ResourceLink is a locator, not proof that a requested image/export exists.
fn validate_completed_acp_artifact_contract(
    data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
) -> Result<(), String> {
    if data.update.status != Some(AcpToolCallStatus::Completed) {
        return Ok(());
    }
    let artifacts = data
        .update
        .content
        .iter()
        .flatten()
        .filter_map(|item| match item {
            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                artifact,
                ..
            } => Some(artifact.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let has_resource_link = data.update.content.iter().flatten().any(|item| {
        matches!(
            item,
            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
        )
    });
    if has_resource_link && artifacts.is_empty() {
        return Err(
            "ACP ResourceLink-only output has no locally verified artifact receipt".to_owned(),
        );
    }
    validate_artifact_receipt_integrity("ACP artifact delivery", &artifacts)
        .map_err(|error| format!("ACP {error}"))?;
    const IDENTITY_KEYS: &[&str] = &[
        "tool",
        "tool_name",
        "toolName",
        "name",
        "operation",
        "operation_name",
        "operationName",
    ];
    let mut identities = data.update.title.iter().map(String::as_str).collect::<Vec<_>>();
    for value in [&data.update.raw_input, &data.update.raw_output]
        .into_iter()
        .filter_map(Option::as_ref)
    {
        let Some(object) = value.as_object() else {
            continue;
        };
        identities.extend(
            IDENTITY_KEYS
                .iter()
                .filter_map(|key| object.get(*key).and_then(Value::as_str)),
        );
    }
    identities.sort_unstable();
    identities.dedup();

    for name in identities {
        validate_completed_artifact_contract(&ToolCallEventData {
            call_id: data.update.tool_call_id.clone(),
            name: name.to_owned(),
            args: data.update.raw_input.clone().unwrap_or(Value::Null),
            status: ToolCallStatus::Completed,
            input: None,
            output: None,
            description: None,
            artifacts: artifacts.clone(),
            retry: None,
        })
        .map_err(|error| format!("ACP {error}"))?;
    }
    Ok(())
}

/// Materialize a provider's sparse ACP update against the latest lifecycle
/// snapshot before validating or persisting it. ACP `ToolCallUpdate` fields are
/// optional and prompt-boundary completion synthesis intentionally carries only
/// the call id, terminal status and verified receipts. Committing that sparse
/// frame directly would discard the tool identity/input that established the
/// artifact contract.
fn effective_acp_tool_call_projection(
    active: Option<&nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData>,
    incoming: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
) -> nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData {
    let Some(active) = active else {
        return incoming.clone();
    };
    let mut effective = incoming.clone();
    if effective.session_id.trim().is_empty() {
        effective.session_id.clone_from(&active.session_id);
    }
    if effective.update.status.is_none() {
        effective.update.status = active.update.status;
    }
    if effective.update.title.is_none() {
        effective.update.title.clone_from(&active.update.title);
    }
    if effective.update.kind.is_none() {
        effective.update.kind = active.update.kind;
    }
    if effective.update.raw_input.is_none() {
        effective.update.raw_input.clone_from(&active.update.raw_input);
    }
    if effective.update.raw_output.is_none() {
        effective.update.raw_output.clone_from(&active.update.raw_output);
    }
    if effective.update.content.is_none() {
        effective.update.content.clone_from(&active.update.content);
    } else if effective.update.status == Some(AcpToolCallStatus::Completed) {
        // A synthesized completion carries an authoritative delivery receipt
        // list but no narration/diff/terminal blocks. Retain those non-delivery
        // blocks from the active snapshot while replacing (rather than
        // duplicating) provisional artifact/resource locators.
        let mut merged = active
            .update
            .content
            .iter()
            .flatten()
            .filter(|item| {
                !matches!(
                    item,
                    nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                        | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut seen = merged
            .iter()
            .filter_map(|item| serde_json::to_string(item).ok())
            .collect::<HashSet<_>>();
        for item in incoming.update.content.iter().flatten() {
            let duplicate = serde_json::to_string(item)
                .ok()
                .is_some_and(|encoded| !seen.insert(encoded));
            if !duplicate {
                merged.push(item.clone());
            }
        }
        effective.update.content = Some(merged);
    }
    if effective.update.locations.is_none() {
        effective.update.locations.clone_from(&active.update.locations);
    }
    if effective.meta.is_none() {
        effective.meta.clone_from(&active.meta);
    }
    effective
}

/// ToolGroup is a legacy summary event and has no artifact receipt field. A
/// Completed high-signal generator/exporter entry therefore cannot establish
/// delivery and must be corrected to Error before the enclosing Finish.
fn tool_group_artifact_contract_errors(
    entries: &[nomifun_ai_agent::protocol::events::tool_call::ToolGroupEntry],
    completed_artifact_tool_calls: &HashMap<String, ToolCallEventData>,
) -> Vec<(usize, String)> {
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let paired_delivery = completed_artifact_tool_calls.get(&entry.call_id);
            let result = validate_completed_artifact_contract(&ToolCallEventData {
                call_id: entry.call_id.clone(),
                name: entry.name.clone(),
                args: paired_delivery
                    .map(|delivery| delivery.args.clone())
                    .unwrap_or(Value::Null),
                status: entry.status,
                input: None,
                output: None,
                description: entry.description.clone(),
                artifacts: paired_delivery
                    .map(|delivery| delivery.artifacts.clone())
                    .unwrap_or_default(),
                retry: None,
            });
            result.err().map(|error| (index, error))
        })
        .collect()
}

fn tool_group_entry_has_artifact_contract(
    entry: &nomifun_ai_agent::protocol::events::tool_call::ToolGroupEntry,
) -> bool {
    validate_completed_artifact_contract(&ToolCallEventData {
        call_id: entry.call_id.clone(),
        name: entry.name.clone(),
        args: Value::Null,
        status: ToolCallStatus::Completed,
        input: None,
        output: None,
        description: entry.description.clone(),
        artifacts: Vec::new(),
        retry: None,
    })
    .is_err()
}

#[derive(Debug, Clone)]
struct TextSegmentState {
    id: String,
    buffer: String,
    created_at: i64,
    record_created: bool,
    flush_counter: u32,
}

#[derive(Debug, Clone)]
struct PersistedTextSegment {
    id: String,
}

#[derive(Debug, Clone)]
struct ThinkingSegmentState {
    id: String,
    buffer: String,
    started_at: i64,
    completed_duration_ms: Option<u64>,
}

/// Result returned after a relay turn has fully drained and finalized.
#[derive(Debug, Clone, Default)]
pub struct RelayOutcome {
    pub system_responses: Vec<String>,
    pub terminal: RelayTerminal,
    /// Normalized terminal reason carried by Finish. `Cancelled` is never a
    /// successful completion and must suppress failover, continuation, and
    /// post-turn writeback in the service send loop.
    pub stop_reason: Option<TurnStopReason>,
    /// Phase 3 (plan D4): whether this turn emitted **any** externally-visible
    /// response before terminating — assistant `Text` OR a forwarded/persisted
    /// tool action (ToolCall / AcpToolCall / ToolGroup / persisted Thinking).
    /// The failover seam only switches models pre-response (`!emitted_response`)
    /// so a fault AFTER any visible output is never failed over — that would
    /// duplicate already-streamed text OR re-run a tool side effect (and re-bill).
    pub emitted_response: bool,
    /// Phase 3 (review #1/#5): when the relay SUPPRESSED a pre-response provider
    /// fault (no WS error event, no error `tips` row — because the send loop was
    /// expected to fail over), this carries the swallowed `Error` event. The send
    /// loop re-surfaces it (broadcast + persist) if the failover did NOT actually
    /// fire (e.g. the picker found no usable candidate at runtime) — preserving
    /// the "queue-exhausted → ORIGINAL error" invariant. `None` = nothing suppressed.
    pub suppressed_error: Option<AgentStreamEvent>,
    /// Final visible assistant text after response middleware rewrites. Used by
    /// turn-final knowledge write-back after the relay has persisted the text and
    /// completed the turn.
    pub final_text: Option<String>,
    /// Message id of the visible text row that should own turn-final
    /// post-processing UI state. This may differ from the turn's primary msg_id
    /// when the turn starts with thinking/tool output before final text.
    pub final_text_msg_id: Option<String>,
    /// Number of locally verified artifacts whose complete turn batch crossed
    /// the durable repository commit barrier. Provisional receipts and
    /// partially/ambiguously committed batches never increment this value.
    pub committed_artifact_count: usize,
}

fn turn_writeback_status_label(status: nomifun_knowledge::TurnWritebackStatus) -> &'static str {
    match status {
        nomifun_knowledge::TurnWritebackStatus::Disabled => "disabled",
        nomifun_knowledge::TurnWritebackStatus::NoCompleter => "no_completer",
        nomifun_knowledge::TurnWritebackStatus::NoCandidate => "no_candidate",
        nomifun_knowledge::TurnWritebackStatus::Written => "written",
        nomifun_knowledge::TurnWritebackStatus::Partial => "partial",
        nomifun_knowledge::TurnWritebackStatus::Failed => "failed",
    }
}

fn turn_writeback_phase_label(phase: nomifun_knowledge::TurnWritebackPhase) -> &'static str {
    match phase {
        nomifun_knowledge::TurnWritebackPhase::Extracting => "extracting",
        nomifun_knowledge::TurnWritebackPhase::Writing => "writing",
    }
}

fn turn_writeback_running_state(
    status: &str,
    attempt_id: &str,
    attempt_generation: u64,
    started_at: i64,
    updated_at: i64,
    prior_written: &[Value],
    prior_failures: &[Value],
) -> Value {
    json!({
        "status": status,
        "attempt_id": attempt_id,
        "attempt_generation": attempt_generation,
        "started_at": started_at,
        "updated_at": updated_at,
        "finished_at": Value::Null,
        "retryable": false,
        "candidates": 0,
        "written": prior_written,
        "failures": prior_failures,
    })
}

fn turn_writeback_interrupted_state(
    attempt_id: &str,
    attempt_generation: u64,
    started_at: i64,
    interrupted_at: i64,
    reason: &str,
    prior_written: &[Value],
    prior_failures: &[Value],
) -> Value {
    // A global/provider failure describes one attempt, not a durable target.
    // Keep target-specific failures across partial retries, but replace any
    // historical global failure with this interruption so retry metadata stays
    // bounded even when providers include unique request IDs.
    let mut failures = prior_failures
        .iter()
        .filter(|failure| {
            failure.get("kb_id").and_then(Value::as_str).is_some()
                && failure.get("rel_path").and_then(Value::as_str).is_some()
        })
        .cloned()
        .collect::<Vec<_>>();
    failures.push(json!({
        "kb_id": Value::Null,
        "rel_path": Value::Null,
        "error": reason,
    }));
    json!({
        "status": "interrupted",
        "attempt_id": attempt_id,
        "attempt_generation": attempt_generation,
        "started_at": started_at,
        "updated_at": interrupted_at,
        "finished_at": interrupted_at,
        "interrupted_at": interrupted_at,
        // The process may have stopped after a direct file merge committed but
        // before its terminal message state committed. Retrying this attempt
        // generically could duplicate that side effect.
        "retryable": false,
        "commit_ambiguous": true,
        "candidates": 0,
        "written": prior_written,
        "failures": failures,
    })
}

fn turn_writeback_not_started_state(
    attempt_id: &str,
    attempt_generation: u64,
    started_at: i64,
    failed_at: i64,
    reason: &str,
    prior_written: &[Value],
    prior_failures: &[Value],
) -> Value {
    let mut failures = prior_failures
        .iter()
        .filter(|failure| {
            failure.get("kb_id").and_then(Value::as_str).is_some()
                && failure.get("rel_path").and_then(Value::as_str).is_some()
        })
        .cloned()
        .collect::<Vec<_>>();
    failures.push(json!({
        "kb_id": Value::Null,
        "rel_path": Value::Null,
        "error": reason,
    }));
    json!({
        "status": "failed",
        "attempt_id": attempt_id,
        "attempt_generation": attempt_generation,
        "started_at": started_at,
        "updated_at": failed_at,
        "finished_at": failed_at,
        "retryable": true,
        "commit_ambiguous": false,
        "candidates": 0,
        "written": prior_written,
        "failures": failures,
    })
}

fn turn_writeback_final_state(
    report: &nomifun_knowledge::TurnWritebackReport,
    attempt_id: &str,
    attempt_generation: u64,
    started_at: i64,
    finished_at: i64,
    prior_written: &[Value],
    prior_failures: &[Value],
) -> Value {
    // A write-back target is now just the document it addresses: there is no
    // staged storage path to map back to a logical one. Message rows written
    // before that change may still carry an `_inbox/{scope}/` prefix, and they
    // simply key differently — a retry against such a row re-proposes the
    // material rather than mis-deduplicating it against the base document.
    let target_key = |kb_id: &str, rel_path: &str| {
        format!(
            "{kb_id}\0{}",
            nomifun_knowledge::service::portable_writeback_path_identity(rel_path)
        )
    };
    let value_target_key = |item: &Value| {
        Some(target_key(
            item.get("kb_id")?.as_str()?,
            item.get("rel_path")?.as_str()?,
        ))
    };

    let mut written = Vec::new();
    let mut seen_written = HashSet::new();
    for item in prior_written {
        let dedupe_key = value_target_key(item)
            .or_else(|| serde_json::to_string(item).ok());
        if dedupe_key.is_none_or(|key| seen_written.insert(key)) {
            written.push(item.clone());
        }
    }
    for outcome in &report.written {
        let item = json!({
            "kb_id": outcome.kb_id.clone(),
            "rel_path": outcome.final_rel_path.clone(),
        });
        let key = target_key(
            outcome.kb_id.as_str(),
            &outcome.final_rel_path,
        );
        if seen_written.insert(key)
        {
            written.push(item);
        }
    }
    // Preserve unresolved target failures when a retry produces no candidate
    // (or only resolves a subset). A single failed target may legitimately be
    // corrected to a different path, so one successful write clears that lone
    // historical target; with several historical targets, only an exact
    // successful target is cleared and the rest remain retryable.
    let prior_target_failures = prior_failures
        .iter()
        .filter(|failure| value_target_key(failure).is_some())
        .cloned()
        .collect::<Vec<_>>();
    let corrected_single_target = prior_target_failures
        .first()
        .and_then(|failure| failure.get("kb_id"))
        .and_then(Value::as_str)
        .is_some_and(|prior_kb_id| {
            prior_target_failures.len() == 1
                && report
                    .written
                    .iter()
                    .any(|outcome| outcome.kb_id.as_str() == prior_kb_id)
        });
    let mut failures = if corrected_single_target {
        Vec::new()
    } else {
        prior_target_failures
    };
    for outcome in &report.written {
        let key = target_key(
            outcome.kb_id.as_str(),
            &outcome.final_rel_path,
        );
        failures.retain(|existing| {
            value_target_key(existing).as_deref() != Some(key.as_str())
        });
    }
    for failure in &report.failures {
        let item = json!({
            "kb_id": failure.kb_id.clone(),
            "rel_path": failure.rel_path.clone(),
            "error": failure.error.clone(),
        });
        if let (Some(kb_id), Some(rel_path)) =
            (failure.kb_id.as_ref(), failure.rel_path.as_deref())
        {
            let key = target_key(kb_id.as_str(), rel_path);
            failures.retain(|existing| {
                value_target_key(existing).as_deref() != Some(key.as_str())
            });
            failures.push(item);
        } else if !failures.iter().any(|existing| existing == &item) {
            failures.push(item);
        }
    }
    let status = if !written.is_empty() && !failures.is_empty() {
        "partial"
    } else if !failures.is_empty() {
        "failed"
    } else {
        turn_writeback_status_label(report.status)
    };
    let retryable = matches!(status, "partial" | "failed" | "no_completer");
    json!({
        "status": status,
        "attempt_id": attempt_id,
        "attempt_generation": attempt_generation,
        "started_at": started_at,
        "updated_at": finished_at,
        "finished_at": finished_at,
        "retryable": retryable,
        "candidates": report.candidates,
        "written": written,
        "failures": failures,
    })
}

fn turn_writeback_event_payload(conversation_id: &str, msg_id: &str, state: &Value) -> Value {
    let mut payload = state.clone();
    if let Some(obj) = payload.as_object_mut() {
        // These fields are persisted solely so an explicit retry can recreate
        // the exact source turn. They are not part of the realtime presentation
        // contract. `scope` is no longer written — it was the staged inbox
        // namespace — but rows persisted before that change still carry it, so
        // the strip stays to keep it out of the wire payload.
        obj.remove("source_message_id");
        obj.remove("scope");
        obj.remove("assistant_text");
        obj.insert("conversation_id".to_owned(), json!(conversation_id));
        obj.insert("msg_id".to_owned(), json!(msg_id));
    }
    payload
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnWritebackPersistOutcome {
    Committed,
    MessageMissing,
    IgnoredTerminalAttempt,
    IgnoredStaleAttempt,
    IgnoredStaleProgress,
    IgnoredDuplicate,
}

fn turn_writeback_status_is_running(status: &str) -> bool {
    matches!(status, "started" | "extracting" | "writing")
}

fn turn_writeback_running_phase(status: &str) -> Option<u8> {
    match status {
        "started" => Some(0),
        "extracting" => Some(1),
        "writing" => Some(2),
        _ => None,
    }
}

fn turn_writeback_attempt_identity(state: &Value) -> Option<(Option<u64>, i64, &str)> {
    Some((
        state.get("attempt_generation").and_then(Value::as_u64),
        state.get("started_at")?.as_i64()?,
        state.get("attempt_id")?.as_str()?,
    ))
}

/// Decide whether `incoming` may replace an already persisted write-back
/// state. Unknown status labels are deliberately terminal (fail closed): a
/// future version's durable terminal state must not be regressed to a running
/// state by an older binary.
fn reject_turn_writeback_transition(
    existing: &Value,
    incoming: &Value,
) -> Option<TurnWritebackPersistOutcome> {
    let Some((existing_generation, existing_started_at, existing_attempt_id)) =
        turn_writeback_attempt_identity(existing)
    else {
        return None;
    };
    let Some((incoming_generation, incoming_started_at, incoming_attempt_id)) =
        turn_writeback_attempt_identity(incoming)
    else {
        return Some(TurnWritebackPersistOutcome::IgnoredStaleProgress);
    };

    if existing_attempt_id != incoming_attempt_id {
        // Retry generation is the durable ordering authority. Fall back to the
        // process-monotonic timestamp only for legacy states that predate it.
        // This prevents a late worker from an older generation winning after a
        // wall-clock rollback or after the application restarts.
        if let (Some(existing_generation), Some(incoming_generation)) =
            (existing_generation, incoming_generation)
        {
            return (incoming_generation <= existing_generation)
                .then_some(TurnWritebackPersistOutcome::IgnoredStaleAttempt);
        }
        let existing_order = (
            existing_generation.unwrap_or_default(),
            existing_started_at,
            existing_attempt_id,
        );
        let incoming_order = (
            incoming_generation.unwrap_or_default(),
            incoming_started_at,
            incoming_attempt_id,
        );
        return (incoming_order <= existing_order)
            .then_some(TurnWritebackPersistOutcome::IgnoredStaleAttempt);
    }

    let existing_status = existing
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("__unknown_terminal__");
    let incoming_status = incoming
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("__unknown_terminal__");

    if !turn_writeback_status_is_running(existing_status) {
        return Some(TurnWritebackPersistOutcome::IgnoredTerminalAttempt);
    }

    if turn_writeback_status_is_running(incoming_status) {
        let existing_updated_at = existing
            .get("updated_at")
            .and_then(Value::as_i64)
            .unwrap_or(existing_started_at);
        let incoming_updated_at = incoming
            .get("updated_at")
            .and_then(Value::as_i64)
            .unwrap_or(incoming_started_at);
        if incoming_updated_at < existing_updated_at {
            return Some(TurnWritebackPersistOutcome::IgnoredStaleProgress);
        }

        if let (Some(existing_phase), Some(incoming_phase)) = (
            turn_writeback_running_phase(existing_status),
            turn_writeback_running_phase(incoming_status),
        ) && incoming_phase < existing_phase
        {
            return Some(TurnWritebackPersistOutcome::IgnoredStaleProgress);
        }
    }

    (existing == incoming).then_some(TurnWritebackPersistOutcome::IgnoredDuplicate)
}

type TurnWritebackMessageLock = AsyncMutex<()>;

fn turn_writeback_message_lock(
    conversation_id: &str,
    msg_id: &str,
) -> Arc<TurnWritebackMessageLock> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Weak<TurnWritebackMessageLock>>>> =
        OnceLock::new();
    let key = format!("{conversation_id}\0{msg_id}");
    let mut locks = LOCKS
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn next_turn_writeback_started_at() -> i64 {
    static LAST_STARTED_AT: AtomicI64 = AtomicI64::new(0);
    let wall_clock = now_ms();
    let mut observed = LAST_STARTED_AT.load(Ordering::Relaxed);
    loop {
        let next = wall_clock.max(observed.saturating_add(1));
        match LAST_STARTED_AT.compare_exchange_weak(
            observed,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => observed = actual,
        }
    }
}

async fn persist_turn_writeback_state(
    repo: &Arc<dyn IConversationRepository>,
    conversation_id: &str,
    msg_id: &str,
    state: &Value,
) -> Result<TurnWritebackPersistOutcome, DbError> {
    // The repository currently exposes a read/update pair rather than a JSON
    // compare-and-swap. Serialize every write-back state mutation for a message
    // inside this backend process so the monotonic check and update are one
    // critical section.
    let persistence_lock = turn_writeback_message_lock(conversation_id, msg_id);
    let _guard = persistence_lock.lock().await;
    let row = match repo.get_message(conversation_id, msg_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            debug!(conversation_id, msg_id, "skip writeback state persist; assistant message row not found");
            return Ok(TurnWritebackPersistOutcome::MessageMissing);
        }
        Err(error) => return Err(error),
    };

    let mut content: Value =
        serde_json::from_str(&row.content).unwrap_or_else(|_| json!({ "content": row.content }));
    if !content.is_object() {
        content = json!({ "content": content });
    }
    if let Some(obj) = content.as_object_mut() {
        if let Some(existing) = obj.get("knowledge_writeback")
            && let Some(outcome) = reject_turn_writeback_transition(existing, state)
        {
            debug!(
                conversation_id,
                msg_id,
                ?outcome,
                "ignored non-monotonic knowledge write-back state transition"
            );
            return Ok(outcome);
        }
        obj.insert("knowledge_writeback".to_owned(), state.clone());
    }

    let update = MessageRowUpdate {
        content: Some(content.to_string()),
        status: None,
        hidden: None,
    };
    repo.update_message(&row.message_id, &update).await?;
    Ok(TurnWritebackPersistOutcome::Committed)
}

async fn emit_turn_writeback_state(
    repo: &Arc<dyn IConversationRepository>,
    user_events: &Arc<dyn UserEventSink>,
    user_id: &str,
    conversation_id: &str,
    msg_id: &str,
    state: Value,
) -> Result<TurnWritebackPersistOutcome, DbError> {
    let outcome = persist_turn_writeback_state(repo, conversation_id, msg_id, &state).await?;
    if outcome != TurnWritebackPersistOutcome::Committed {
        return Ok(outcome);
    }

    // Persistence is authoritative. The event is only a projection of the
    // committed state, and an event sink panic must not unwind into the worker's
    // panic finalizer and attempt to replace a durable terminal state.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        user_events.send_to_user(
            user_id,
            WebSocketMessage::new(
                "knowledge.writeback",
                turn_writeback_event_payload(conversation_id, msg_id, &state),
            ),
        );
    }))
    .is_err()
    {
        warn!(
            conversation_id,
            msg_id,
            "knowledge write-back event sink panicked after durable persistence"
        );
    }
    Ok(outcome)
}

#[derive(Clone)]
pub(crate) struct TurnWritebackAttempt {
    repo: Arc<dyn IConversationRepository>,
    user_events: Arc<dyn UserEventSink>,
    user_id: String,
    conversation_id: String,
    msg_id: String,
    source_message_id: String,
    assistant_text: String,
    prior_written: Vec<Value>,
    prior_failures: Vec<Value>,
    attempt_id: String,
    attempt_generation: u64,
    started_at: i64,
}

#[derive(Debug)]
struct TurnWritebackActivity {
    attempt_id: String,
    completed: AtomicBool,
    completed_notify: Notify,
}

impl TurnWritebackActivity {
    fn complete(&self) {
        if !self.completed.swap(true, Ordering::AcqRel) {
            self.completed_notify.notify_waiters();
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.completed_notify.notified();
            if self.completed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

struct TurnWritebackActivityCompletionGuard(Arc<TurnWritebackActivity>);

impl Drop for TurnWritebackActivityCompletionGuard {
    fn drop(&mut self) {
        self.0.complete();
    }
}

fn turn_writeback_activity_registry(
) -> &'static StdMutex<HashMap<String, Vec<Weak<TurnWritebackActivity>>>> {
    static ACTIVITIES: OnceLock<
        StdMutex<HashMap<String, Vec<Weak<TurnWritebackActivity>>>>,
    > = OnceLock::new();
    ACTIVITIES.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn register_turn_writeback_activity(
    conversation_id: &str,
    attempt_id: &str,
) -> Arc<TurnWritebackActivity> {
    let activity = Arc::new(TurnWritebackActivity {
        attempt_id: attempt_id.to_owned(),
        completed: AtomicBool::new(false),
        completed_notify: Notify::new(),
    });
    let mut registry = turn_writeback_activity_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let activities = registry.entry(conversation_id.to_owned()).or_default();
    activities.retain(|activity| {
        activity
            .upgrade()
            .is_some_and(|activity| !activity.completed.load(Ordering::Acquire))
    });
    activities.push(Arc::downgrade(&activity));
    activity
}

fn active_turn_writeback_activities(
    conversation_id: &str,
) -> Vec<Arc<TurnWritebackActivity>> {
    let mut registry = turn_writeback_activity_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut active = Vec::new();
    let remove_entry = if let Some(activities) = registry.get_mut(conversation_id) {
        activities.retain(|activity| {
            let Some(activity) = activity.upgrade() else {
                return false;
            };
            if activity.completed.load(Ordering::Acquire) {
                return false;
            }
            active.push(activity);
            true
        });
        activities.is_empty()
    } else {
        false
    };
    if remove_entry {
        registry.remove(conversation_id);
    }
    active
}

/// Await every process-local knowledge write-back worker for one Conversation.
///
/// A write-back worker is detached from the outer relay owner but remains
/// registered here until all filesystem work and terminal message-state
/// persistence have returned. This is intentional: aborting the outer owner
/// must not detach write-back work from the lifecycle fence. The knowledge
/// layer additionally keeps each final target-path syscall cancellation-
/// indivisible, so activity completion proves that no publication can land
/// after a replacement turn starts.
///
/// Stop/reset/delete callers must establish their exact Conversation tombstone,
/// wait for the outer turn owner to quiesce, then await this fence before
/// reconciling write-back state or committing durable Finished.  The tombstone
/// is what excludes a new activity registration while this method rescans.
pub(crate) async fn await_turn_writeback_quiesced(conversation_id: &str) {
    loop {
        let activities = active_turn_writeback_activities(conversation_id);
        if activities.is_empty() {
            return;
        }
        for activity in activities {
            debug!(
                conversation_id,
                attempt_id = %activity.attempt_id,
                "Waiting for exact-turn knowledge write-back activity to quiesce"
            );
            activity.wait().await;
        }
    }
}

/// Abort-safe owner for one write-back attempt.
///
/// Keep this guard alive for the entire asynchronous write-back operation and
/// disarm it only after a terminal state is durably committed (or the attempt
/// is proven stale). If the owning future is aborted or dropped while a Tokio
/// runtime is still live, `Drop` schedules an `interrupted` terminal persist.
pub(crate) struct TurnWritebackOwnerGuard {
    attempt: Option<TurnWritebackAttempt>,
    reason: &'static str,
}

impl TurnWritebackOwnerGuard {
    pub(crate) fn disarm(&mut self) {
        self.attempt = None;
    }
}

impl Drop for TurnWritebackOwnerGuard {
    fn drop(&mut self) {
        let Some(attempt) = self.attempt.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                "knowledge write-back owner dropped without a live Tokio runtime"
            );
            return;
        };
        let reason = self.reason;
        let _ = runtime.spawn(async move {
            attempt.interrupt(reason).await;
        });
    }
}

impl TurnWritebackAttempt {
    pub(crate) fn new(
        repo: Arc<dyn IConversationRepository>,
        user_events: Arc<dyn UserEventSink>,
        user_id: String,
        conversation_id: String,
        msg_id: String,
        source_message_id: String,
        assistant_text: String,
        prior_written: Vec<Value>,
        prior_failures: Vec<Value>,
        attempt_generation: u64,
    ) -> Self {
        let started_at = next_turn_writeback_started_at();
        Self {
            repo,
            user_events,
            user_id,
            conversation_id,
            source_message_id,
            assistant_text: nomifun_knowledge::turn_writeback::bounded_assistant_text(
                &assistant_text,
            ),
            prior_written,
            prior_failures,
            attempt_id: format!(
                "{msg_id}:{attempt_generation}:{started_at}:{}",
                generate_id()
            ),
            attempt_generation,
            msg_id,
            started_at,
        }
    }

    fn durable_state(&self, mut state: Value) -> Value {
        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "source_message_id".to_owned(),
                json!(self.source_message_id),
            );
            obj.insert("assistant_text".to_owned(), json!(self.assistant_text));
        }
        state
    }

    pub(crate) async fn persist_started_intent(&self) -> Result<(), String> {
        let state = self.durable_state(turn_writeback_running_state(
            "started",
            &self.attempt_id,
            self.attempt_generation,
            self.started_at,
            self.started_at,
            &self.prior_written,
            &self.prior_failures,
        ));
        persist_turn_writeback_state(
            &self.repo,
            &self.conversation_id,
            &self.msg_id,
            &state,
        )
        .await
        .map_err(|error| format!("failed to persist write-back intent: {error}"))
        .and_then(Self::require_intent_outcome)
    }

    /// Persist AND broadcast the durable "started" intent. The detached
    /// turn-final path publishes the running chip before its owning turn
    /// completes; the worker's own "started" emit then lands as an ignored
    /// duplicate instead of a second projection.
    pub(crate) async fn emit_started_intent(&self) -> Result<(), String> {
        self.emit(turn_writeback_running_state(
            "started",
            &self.attempt_id,
            self.attempt_generation,
            self.started_at,
            self.started_at,
            &self.prior_written,
            &self.prior_failures,
        ))
        .await
        .map_err(|error| format!("failed to persist write-back intent: {error}"))
        .and_then(Self::require_intent_outcome)
    }

    fn require_intent_outcome(outcome: TurnWritebackPersistOutcome) -> Result<(), String> {
        match outcome {
            TurnWritebackPersistOutcome::Committed
            | TurnWritebackPersistOutcome::IgnoredDuplicate => Ok(()),
            other => Err(format!(
                "write-back intent was rejected by the monotonic state fence: {other:?}"
            )),
        }
    }

    pub(crate) fn owner_guard(&self, reason: &'static str) -> TurnWritebackOwnerGuard {
        TurnWritebackOwnerGuard {
            attempt: Some(self.clone()),
            reason,
        }
    }

    async fn emit(&self, state: Value) -> Result<TurnWritebackPersistOutcome, DbError> {
        let state = self.durable_state(state);
        emit_turn_writeback_state(
            &self.repo,
            &self.user_events,
            &self.user_id,
            &self.conversation_id,
            &self.msg_id,
            state,
        )
        .await
    }

    /// Publish a durable terminal state when the write-back owner panics or is
    /// aborted. This updates only the assistant message's post-processing state;
    /// the conversation lifecycle remains owned by `ConversationService`.
    pub(crate) async fn interrupt(&self, reason: &'static str) {
        let interrupted_at = now_ms();
        persist_terminal_writeback_until_resolved(
            self,
            turn_writeback_interrupted_state(
                &self.attempt_id,
                self.attempt_generation,
                self.started_at,
                interrupted_at,
                reason,
                &self.prior_written,
                &self.prior_failures,
            ),
        )
        .await;
    }
}

fn terminal_writeback_outcome_is_resolved(outcome: TurnWritebackPersistOutcome) -> bool {
    matches!(
        outcome,
        TurnWritebackPersistOutcome::Committed
            | TurnWritebackPersistOutcome::MessageMissing
            | TurnWritebackPersistOutcome::IgnoredTerminalAttempt
            | TurnWritebackPersistOutcome::IgnoredStaleAttempt
    )
}

/// Persist a post-side-effect terminal state without a business timeout.
///
/// Once knowledge file effects may have happened, callers must never rerun the
/// extractor/writer to recover a message-state failure. Retrying this one JSON
/// transition is side-effect free and keeps the attempt owner alive until the
/// durable state is committed, proven already terminal/stale, or the message no
/// longer exists.
async fn persist_terminal_writeback_until_resolved(
    attempt: &TurnWritebackAttempt,
    state: Value,
) -> TurnWritebackPersistOutcome {
    const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(25);
    const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);

    let mut retry_delay = INITIAL_RETRY_DELAY;
    loop {
        match attempt.emit(state.clone()).await {
            Ok(outcome) if terminal_writeback_outcome_is_resolved(outcome) => return outcome,
            Ok(outcome) => {
                warn!(
                    conversation_id = %attempt.conversation_id,
                    msg_id = %attempt.msg_id,
                    ?outcome,
                    retry_delay_ms = retry_delay.as_millis(),
                    "terminal knowledge write-back state was rejected; retrying without replaying side effects"
                );
            }
            Err(error) => {
                warn!(
                    conversation_id = %attempt.conversation_id,
                    msg_id = %attempt.msg_id,
                    error = %ErrorChain(&error),
                    retry_delay_ms = retry_delay.as_millis(),
                    "terminal knowledge write-back persistence failed; retrying without replaying side effects"
                );
            }
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
    }
}

async fn await_turn_writeback_report_or_interrupt<F>(
    attempt: &TurnWritebackAttempt,
    owner_guard: &mut TurnWritebackOwnerGuard,
    report_future: F,
) -> Option<nomifun_knowledge::TurnWritebackReport>
where
    F: Future<Output = nomifun_knowledge::TurnWritebackReport>,
{
    match std::panic::AssertUnwindSafe(report_future)
        .catch_unwind()
        .await
    {
        Ok(report) => Some(report),
        Err(_) => {
            error!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                "turn-final knowledge write-back panicked; persisting an interrupted terminal state before releasing turn ownership"
            );
            persist_terminal_writeback_until_resolved(
                attempt,
                turn_writeback_interrupted_state(
                    &attempt.attempt_id,
                    attempt.attempt_generation,
                    attempt.started_at,
                    now_ms(),
                    "knowledge write-back panicked after side effects may have started",
                    &attempt.prior_written,
                    &attempt.prior_failures,
                ),
            )
            .await;
            owner_guard.disarm();
            None
        }
    }
}

/// Convert write-back states left running by a dead process into durable,
/// commit-ambiguous `interrupted` terminal states without replaying extraction
/// or file writes.
///
/// The caller must first establish that this conversation has no process-local
/// live turn/write-back owner. Calling this while an owner is active would
/// intentionally terminate that attempt's UI state even though its side effect
/// may still be running.
pub(crate) async fn reconcile_orphaned_writebacks(
    repo: Arc<dyn IConversationRepository>,
    user_events: Option<Arc<dyn UserEventSink>>,
    user_id: &str,
    conversation_id: &str,
) -> Result<usize, DbError> {
    const PAGE_SIZE: u32 = 200;
    const REASON: &str = "application stopped before knowledge write-back completed";

    let mut page = 1;
    let mut reconciled = 0;
    loop {
        let rows = repo
            .get_messages(conversation_id, page, PAGE_SIZE, SortOrder::Asc)
            .await?;
        for row in rows.items {
            let Ok(content) = serde_json::from_str::<Value>(&row.content) else {
                continue;
            };
            let Some(state) = content.get("knowledge_writeback") else {
                continue;
            };
            let Some(status) = state.get("status").and_then(Value::as_str) else {
                continue;
            };
            if !turn_writeback_status_is_running(status) {
                continue;
            }
            let Some((stored_generation, started_at, attempt_id)) =
                turn_writeback_attempt_identity(state)
            else {
                continue;
            };
            let interrupted_at = now_ms().max(
                state
                    .get("updated_at")
                    .and_then(Value::as_i64)
                    .unwrap_or(started_at),
            );
            let attempt_generation = stored_generation.unwrap_or_default();
            let prior_written = state
                .get("written")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let prior_failures = state
                .get("failures")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut interrupted = turn_writeback_interrupted_state(
                attempt_id,
                attempt_generation,
                started_at,
                interrupted_at,
                REASON,
                &prior_written,
                &prior_failures,
            );
            if let (Some(existing), Some(next)) =
                (state.as_object(), interrupted.as_object_mut())
            {
                for key in ["source_message_id", "assistant_text"] {
                    if let Some(value) = existing.get(key) {
                        next.insert(key.to_owned(), value.clone());
                    }
                }
            }
            let outcome = if let Some(events) = user_events.as_ref() {
                emit_turn_writeback_state(
                    &repo,
                    events,
                    user_id,
                    conversation_id,
                    &row.message_id,
                    interrupted,
                )
                .await?
            } else {
                persist_turn_writeback_state(
                    &repo,
                    conversation_id,
                    &row.message_id,
                    &interrupted,
                )
                .await?
            };
            if outcome == TurnWritebackPersistOutcome::Committed {
                reconciled += 1;
            }
        }
        if !rows.has_more {
            break;
        }
        page += 1;
    }
    Ok(reconciled)
}

/// Terminalize every persisted running write-back after the process-local
/// activity fence proves its worker is gone.
///
/// This retry loop intentionally has no total timeout.  It is the persistence
/// half of [`await_turn_writeback_quiesced`]: callers must await the activity
/// fence first, then keep their exact stop/preparation tombstones until this
/// function returns.  Only after both barriers may Conversation Finished and
/// an accepted receipt be committed.
pub(crate) async fn reconcile_quiesced_writebacks_until_resolved(
    repo: Arc<dyn IConversationRepository>,
    user_events: Option<Arc<dyn UserEventSink>>,
    user_id: &str,
    conversation_id: &str,
) -> usize {
    const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(25);
    const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);

    let mut retry_delay = INITIAL_RETRY_DELAY;
    loop {
        match reconcile_orphaned_writebacks(
            Arc::clone(&repo),
            user_events.clone(),
            user_id,
            conversation_id,
        )
        .await
        {
            Ok(reconciled) => return reconciled,
            Err(error) => {
                warn!(
                    conversation_id,
                    error = %ErrorChain(&error),
                    retry_delay_ms = retry_delay.as_millis(),
                    "quiesced knowledge write-back reconciliation failed; retaining exact turn ownership"
                );
                tokio::time::sleep(retry_delay).await;
                retry_delay =
                    retry_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
            }
        }
    }
}

async fn persist_panicked_writeback_until_resolved(
    attempt: &TurnWritebackAttempt,
    reason: &'static str,
) {
    const RETRY_DELAY: Duration = Duration::from_millis(100);
    loop {
        match std::panic::AssertUnwindSafe(attempt.interrupt(reason))
            .catch_unwind()
            .await
        {
            Ok(()) => return,
            Err(_) => {
                error!(
                    conversation_id = %attempt.conversation_id,
                    msg_id = %attempt.msg_id,
                    "knowledge write-back panic recovery also panicked; retaining the activity fence and retrying"
                );
                tokio::time::sleep(RETRY_DELAY).await;
            }
        }
    }
}

async fn run_registered_turn_writeback<F>(
    attempt: TurnWritebackAttempt,
    work: F,
) -> Result<(), DbError>
where
    F: Future<Output = Result<(), DbError>> + Send + 'static,
{
    let activity =
        register_turn_writeback_activity(&attempt.conversation_id, &attempt.attempt_id);
    let completion_activity = Arc::clone(&activity);
    let panic_attempt = attempt.clone();
    let (result_tx, result_rx) = oneshot::channel();
    tokio::spawn(async move {
        let completion_guard =
            TurnWritebackActivityCompletionGuard(completion_activity);
        let result = match std::panic::AssertUnwindSafe(work).catch_unwind().await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                persist_panicked_writeback_until_resolved(
                    &panic_attempt,
                    "knowledge write-back worker returned before terminal persistence",
                )
                .await;
                Err(error)
            }
            Err(_) => {
                persist_panicked_writeback_until_resolved(
                    &panic_attempt,
                    "knowledge write-back worker panicked after side effects may have started",
                )
                .await;
                Ok(())
            }
        };
        // Wake stop/reset/delete before the normal owner observes completion:
        // both paths may now proceed, but neither can pass its durable
        // finalization fence until this activity is absent.
        drop(completion_guard);
        let _ = result_tx.send(result);
    });

    result_rx.await.map_err(|_| {
        DbError::Init(
            "knowledge write-back worker exited without reporting terminal completion"
                .to_owned(),
        )
    })?
}

pub(crate) async fn run_turn_writeback_report(
    service: Arc<nomifun_knowledge::KnowledgeService>,
    request: nomifun_knowledge::TurnWritebackRequest,
    final_text: String,
    attempt: TurnWritebackAttempt,
) -> Result<(), DbError> {
    let worker_attempt = attempt.clone();
    run_registered_turn_writeback(
        attempt,
        async move {
            run_turn_writeback_report_inner(
                service,
                request,
                final_text,
                worker_attempt,
            )
            .await
        },
    )
    .await
}

async fn begin_turn_writeback_attempt(
    attempt: &TurnWritebackAttempt,
    owner_guard: &mut TurnWritebackOwnerGuard,
) -> bool {
    let state = turn_writeback_running_state(
        "started",
        &attempt.attempt_id,
        attempt.attempt_generation,
        attempt.started_at,
        attempt.started_at,
        &attempt.prior_written,
        &attempt.prior_failures,
    );
    match attempt.emit(state).await {
        Ok(TurnWritebackPersistOutcome::Committed)
        | Ok(TurnWritebackPersistOutcome::IgnoredDuplicate) => true,
        Ok(outcome) => {
            owner_guard.disarm();
            debug!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                ?outcome,
                "knowledge write-back start was stale, terminal, or no longer owned; skipping side effects"
            );
            false
        }
        Err(error) => {
            warn!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                error = %ErrorChain(&error),
                "knowledge write-back owner state failed; closing the attempt without running side effects"
            );
            persist_terminal_writeback_until_resolved(
                attempt,
                turn_writeback_not_started_state(
                    &attempt.attempt_id,
                    attempt.attempt_generation,
                    attempt.started_at,
                    now_ms(),
                    "knowledge write-back did not start because its owner state could not be persisted",
                    &attempt.prior_written,
                    &attempt.prior_failures,
                ),
            )
            .await;
            owner_guard.disarm();
            false
        }
    }
}

async fn persist_turn_writeback_report_terminal(
    attempt: &TurnWritebackAttempt,
    owner_guard: &mut TurnWritebackOwnerGuard,
    report: &nomifun_knowledge::TurnWritebackReport,
) {
    match report.status {
        nomifun_knowledge::TurnWritebackStatus::Written
        | nomifun_knowledge::TurnWritebackStatus::Partial => {
            info!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                candidates = report.candidates,
                written = report.written.len(),
                failures = report.failures.len(),
                "turn-final knowledge write-back completed"
            );
        }
        nomifun_knowledge::TurnWritebackStatus::Failed => {
            warn!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                candidates = report.candidates,
                failures = report.failures.len(),
                "turn-final knowledge write-back failed"
            );
        }
        other => {
            debug!(
                conversation_id = %attempt.conversation_id,
                msg_id = %attempt.msg_id,
                status = ?other,
                "turn-final knowledge write-back skipped"
            );
        }
    }

    persist_terminal_writeback_until_resolved(
        attempt,
        turn_writeback_final_state(
            report,
            &attempt.attempt_id,
            attempt.attempt_generation,
            attempt.started_at,
            now_ms(),
            &attempt.prior_written,
            &attempt.prior_failures,
        ),
    )
    .await;
    owner_guard.disarm();
}

async fn run_turn_writeback_report_inner(
    service: Arc<nomifun_knowledge::KnowledgeService>,
    mut request: nomifun_knowledge::TurnWritebackRequest,
    final_text: String,
    attempt: TurnWritebackAttempt,
) -> Result<(), DbError> {
    let mut owner_guard =
        attempt.owner_guard("knowledge write-back future was aborted before terminal persistence");
    if !begin_turn_writeback_attempt(&attempt, &mut owner_guard).await {
        return Ok(());
    }

    request.assistant_text = final_text;
    let started_at = attempt.started_at;
    let attempt_id = attempt.attempt_id.clone();

    let progress_attempt = attempt.clone();
    let progress_attempt_id = attempt_id.clone();
    let report = if request.model.is_none() {
        Some(nomifun_knowledge::TurnWritebackReport::failed(
            "This session has no provider-backed model for knowledge write-back; configure a knowledge model and retry",
        ))
    } else {
        await_turn_writeback_report_or_interrupt(
            &attempt,
            &mut owner_guard,
            service.finalize_turn_writeback_with_progress(request, move |phase| {
                let attempt = progress_attempt.clone();
                let attempt_id = progress_attempt_id.clone();
                let status = turn_writeback_phase_label(phase);
                async move {
                    let updated_at = now_ms();
                    match attempt
                        .emit(turn_writeback_running_state(
                            status,
                            &attempt_id,
                            attempt.attempt_generation,
                            started_at,
                            updated_at,
                            &attempt.prior_written,
                            &attempt.prior_failures,
                        ))
                        .await
                    {
                        Ok(TurnWritebackPersistOutcome::Committed)
                        | Ok(TurnWritebackPersistOutcome::IgnoredDuplicate) => {}
                        Ok(outcome) => {
                            debug!(
                                conversation_id = %attempt.conversation_id,
                                msg_id = %attempt.msg_id,
                                ?outcome,
                                "ignored stale knowledge write-back progress projection"
                            );
                        }
                        Err(error) => {
                            warn!(
                                conversation_id = %attempt.conversation_id,
                                msg_id = %attempt.msg_id,
                                error = %ErrorChain(&error),
                                "failed to persist knowledge write-back progress state"
                            );
                        }
                    }
                }
            }),
        )
        .await
    };
    let Some(report) = report else {
        return Ok(());
    };
    persist_turn_writeback_report_terminal(&attempt, &mut owner_guard, &report).await;
    Ok(())
}

pub(crate) async fn finish_turn_writeback_failure(
    attempt: TurnWritebackAttempt,
    error: String,
) -> Result<(), DbError> {
    let worker_attempt = attempt.clone();
    run_registered_turn_writeback(
        attempt,
        async move {
            let mut owner_guard = worker_attempt.owner_guard(
                "knowledge write-back failure finalizer was aborted before terminal persistence",
            );
            if !begin_turn_writeback_attempt(&worker_attempt, &mut owner_guard).await {
                return Ok(());
            }
            let report = nomifun_knowledge::TurnWritebackReport::failed(error);
            persist_turn_writeback_report_terminal(
                &worker_attempt,
                &mut owner_guard,
                &report,
            )
            .await;
            Ok(())
        },
    )
    .await
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RelayTerminal {
    #[default]
    Finish,
    Error {
        code: Option<AgentErrorCode>,
        retryable: Option<bool>,
    },
    ChannelClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactCommitReconciliation {
    AllDurable,
    DefinitelyNotCommitted,
    Indeterminate,
}

#[derive(Debug)]
struct ArtifactCommitFailure {
    error: DbError,
    rollback_safe: bool,
    commit_state: &'static str,
}

impl ArtifactCommitFailure {
    fn before_commit(error: DbError) -> Self {
        Self {
            error,
            rollback_safe: true,
            commit_state: "not_started",
        }
    }

    fn after_reconciliation(
        error: DbError,
        reconciliation: ArtifactCommitReconciliation,
    ) -> Self {
        let (rollback_safe, commit_state) = match reconciliation {
            ArtifactCommitReconciliation::DefinitelyNotCommitted => (true, "not_committed"),
            ArtifactCommitReconciliation::Indeterminate => (false, "indeterminate"),
            ArtifactCommitReconciliation::AllDurable => {
                unreachable!("an all-durable artifact commit is successful")
            }
        };
        Self {
            error,
            rollback_safe,
            commit_state,
        }
    }
}

impl RelayTerminal {
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }

    pub fn code(&self) -> Option<AgentErrorCode> {
        match self {
            Self::Error { code, .. } => *code,
            Self::Finish | Self::ChannelClosed => None,
        }
    }

    pub fn retryable(&self) -> Option<bool> {
        match self {
            Self::Error { retryable, .. } => *retryable,
            Self::Finish | Self::ChannelClosed => None,
        }
    }
}

/// Relays agent stream events to WebSocket and persists messages.
///
/// This struct is created for each `send_message` call and runs as a
/// background tokio task until the agent finishes or errors out.
pub struct StreamRelay {
    conversation_id: String,
    /// Stable identity of the user-visible logical turn. This remains fixed
    /// across model failover and system continuations.
    root_turn_id: String,
    /// Set only after the structural root row is known durable. The service
    /// may run the public preflight before starting billable agent work; the
    /// relay repeats the check defensively when callers omit that preflight.
    turn_root_ready: AtomicBool,
    /// Identity of the current provider wire segment within `root_turn_id`.
    ///
    /// This is only a transport/stream identity. Durable child messages and
    /// artifact commits belong to `root_turn_id`, otherwise a continuation is
    /// grouped under a different turn after history hydration than it was on
    /// the live WebSocket stream.
    msg_id: String,
    user_id: String,
    repo: Arc<dyn IConversationRepository>,
    user_events: Arc<dyn UserEventSink>,
    cron_service: Option<Arc<dyn ICronService>>,
    /// Legacy relay-owned completion exists only for isolated unit tests.
    /// Production completion is owned by ConversationService's durable
    /// finalize -> exact release -> event fence.
    #[cfg(test)]
    complete_turn: bool,
    #[cfg(test)]
    allow_legacy_unjournaled_artifacts: bool,
    /// Companion-companion wire markers (from `conversation.extra.companion_session` /
    /// `.companion_id`), stamped onto every `message.stream` / `turn.completed`
    /// payload so the companion collector can classify the turn off the wire.
    companion: bool,
    companion_id: Option<CompanionId>,
    /// Originator of the user message that started this turn when it was NOT
    /// typed by the human owner (`"companion"` / `"cron"` / `"autowork"` /
    /// `"idmm"`; `None` = a real person). Stamped onto every `message.stream`
    /// / `turn.completed` payload of the turn so downstream consumers (the
    /// companion collector) can tell agent-driven replies from owner-driven work.
    origin: Option<String>,
    /// IM platform of a Channel Agent conversation (from
    /// `conversation.extra.channel_platform`, e.g. `"telegram"`; `None` = not
    /// a channel conversation). Stamped onto every `message.stream` /
    /// `turn.completed` payload so the companion window can tell remote IM turns
    /// from local companion turns off the wire.
    channel_platform: Option<String>,
    /// True when this relay serves a robot gateway thread
    /// (`conversation.extra.robot_session`). It makes the relay delete bracketed
    /// stage directions from assistant `Text` before the WebSocket forward, so
    /// `segment.buffer` and `full_text_buffer` read the cleaned copy.
    ///
    /// (i) This is a content guard, not a protocol. The robot prompt REQUIRES
    ///     plain spoken sentences — no brackets, no stage directions, no emoji,
    ///     no markdown — but a prompt is not a guarantee: the previous design
    ///     asked the model for an `[emotion:name]` marker and got `[winking]`,
    ///     which every `emotion:`-keyed stripper missed, so it was printed here
    ///     AND read aloud by TTS. The requirement is absolute (要么展示正常内容，
    ///     要么别展示), so the guard is syntax-agnostic and lives in
    ///     `nomifun_common::stage_direction`.
    /// (ii) The device path guards independently
    ///     (`nomifun-robot`'s `sanitize_for_speech` / `sanitize_for_display`) off
    ///     its own `broadcast` clone of the same stream. The two are not
    ///     duplicates: this one owns the desktop transcript and the persisted
    ///     row, that one owns TTS and the OLED, and neither crate may depend on
    ///     the other.
    /// (iii) `false` — the default, and the value for every ordinary chat,
    ///     customer-service, channel and ACP conversation — means assistant text
    ///     is never touched. Deliberately narrower than the ungated precedent of
    ///     `strip_think_tags` / `strip_cron_commands`.
    robot_session: bool,
    /// Phase 3 (review #1/#5): predicate telling the relay whether a PRE-RESPONSE
    /// terminal provider-fault with this error code WILL be failed over by the
    /// send loop. When it returns `true` the relay suppresses the user-visible
    /// error AT SOURCE — it does NOT forward the WS error event NOR persist the
    /// error `tips` row — so a recovered fault shows only the backup model's turn,
    /// never the swallowed error. `None` (the default) = never suppress. The
    /// send loop is the only caller that wires this (it knows nomi + enabled +
    /// within-bound up front; pre-response + provider-fault are evaluated here).
    #[allow(clippy::type_complexity)]
    failover_suppressor: Option<Arc<dyn Fn(AgentErrorCode) -> bool + Send + Sync>>,
    /// Process-wide runtime state, used here only to accumulate this turn's
    /// `TurnCompleted` token usage (`input + output`) into the conversation's
    /// running total so the owning execution attempt can read it after the turn
    /// settles. `None` (the default) =
    /// no token accumulation (the common chat/companion path is unaffected).
    /// `ConversationService::send_message` wires it only when the authoritative
    /// Conversation↔Execution relation identifies an active attempt. Once wired,
    /// the relay always accumulates; it does not perform a second identity lookup.
    runtime_state: Option<Arc<ConversationRuntimeStateService>>,
    /// Generation-scoped service cancellation. This is independent of every
    /// backend transport, so a CLI/gateway that ignores its abort request cannot
    /// leave the relay waiting forever for a terminal event.
    cancellation: Option<AgentTurnCancellation>,
    /// Stable canonical row IDs for streamed sub-records that receive multiple
    /// updates during one relay. Protocol call/session IDs are correlation keys,
    /// never database entity IDs.
    derived_message_ids: std::sync::Mutex<HashMap<String, String>>,
    /// Canonical session workspace used to re-verify every local receipt at
    /// the final database commit barrier. Runtime event payloads are untrusted:
    /// a marker proves an atomic DB transition, not that bytes exist.
    artifact_workspace: Option<PathBuf>,
}

impl StreamRelay {
    /// Await one ordered stream projection to a definitive repository result.
    ///
    /// These mutations must never be wrapped in a local timeout or cancelled
    /// independently of the turn owner. SQLite may already have queued a
    /// command when its Rust future is dropped; allowing the relay to continue
    /// could then commit a stale `work` update after terminal cleanup wrote
    /// `finish`/`error`. Backpressure or a wedged repository therefore retains
    /// turn ownership and withholds the terminal boundary.
    async fn ordered_event_side_effect<T, F>(
        &self,
        label: &'static str,
        future: F,
    ) -> T
    where
        F: Future<Output = T>,
    {
        debug!(
            conversation_id = %self.conversation_id,
            msg_id = %self.msg_id,
            side_effect = label,
            "Awaiting ordered relay persistence"
        );
        future.await
    }

    pub fn new(
        conversation_id: String,
        msg_id: String,
        user_id: String,
        repo: Arc<dyn IConversationRepository>,
        user_events: Arc<dyn UserEventSink>,
        cron_service: Option<Arc<dyn ICronService>>,
    ) -> Self {
        let root_turn_id = msg_id.clone();
        Self {
            conversation_id,
            root_turn_id,
            turn_root_ready: AtomicBool::new(false),
            msg_id,
            user_id,
            repo,
            user_events,
            cron_service,
            #[cfg(test)]
            complete_turn: false,
            #[cfg(test)]
            allow_legacy_unjournaled_artifacts: false,
            companion: false,
            companion_id: None,
            origin: None,
            channel_platform: None,
            robot_session: false,
            failover_suppressor: None,
            runtime_state: None,
            cancellation: None,
            derived_message_ids: std::sync::Mutex::new(HashMap::new()),
            artifact_workspace: None,
        }
    }

    #[cfg(test)]
    fn with_test_turn_completion(mut self) -> Self {
        self.complete_turn = true;
        self
    }

    #[cfg(test)]
    fn with_test_legacy_unjournaled_artifacts(mut self) -> Self {
        self.allow_legacy_unjournaled_artifacts = true;
        self
    }

    pub fn with_root_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.root_turn_id = turn_id.into();
        self.turn_root_ready.store(false, Ordering::Release);
        self
    }

    /// Persist the hidden structural owner of every root-scoped child row.
    ///
    /// Call this before starting provider work. SQLite enforces that any
    /// `msg_id` which differs from a child's `message_id` already exists, so a
    /// tool/status/artifact event cannot safely race the first visible text
    /// segment. The root is deliberately hidden and terminal-looking: it is an
    /// immutable relationship anchor, not a user-visible lifecycle message.
    ///
    /// This method is idempotent for the same relay and reconciles a concurrent
    /// insert. It also accepts a pre-upgrade turn whose first visible text or
    /// thinking row already owns the root id; new turns never create that
    /// representation.
    pub async fn ensure_turn_root_persisted(&self) -> Result<(), DbError> {
        if self.turn_root_ready.load(Ordering::Acquire) {
            return Ok(());
        }

        let expected = MessageRow {
            id: 0,
            message_id: self.root_turn_id.clone(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(self.root_turn_id.clone()),
            r#type: "turn_root".to_owned(),
            content: json!({ "kind": "turn_root" }).to_string(),
            position: Some("center".to_owned()),
            status: Some("finish".to_owned()),
            hidden: true,
            created_at: now_ms(),
        };

        let insert_error = match self.repo.insert_message(&expected).await {
            Ok(()) => {
                self.turn_root_ready.store(true, Ordering::Release);
                return Ok(());
            }
            Err(error) => error,
        };

        match self
            .repo
            .get_message(&self.conversation_id, &self.root_turn_id)
            .await
        {
            Ok(Some(existing)) if self.is_compatible_turn_root(&existing) => {
                self.turn_root_ready.store(true, Ordering::Release);
                Ok(())
            }
            Ok(Some(existing)) => Err(DbError::Conflict(format!(
                "logical turn root '{}' conflicts with an existing {} message owned by {:?}",
                self.root_turn_id, existing.r#type, existing.msg_id
            ))),
            Ok(None) => Err(insert_error),
            Err(reconcile_error) => Err(DbError::Conflict(format!(
                "logical turn root '{}' insert failed ({insert_error}) and its durable state could not be reconciled ({reconcile_error})",
                self.root_turn_id
            ))),
        }
    }

    /// Convert a failed pre-send root preflight directly into the relay's
    /// terminal contract. This deliberately does not retry the database and
    /// does not wait on an agent receiver: provider work has not started, and
    /// no child row may be persisted without its structural owner.
    pub fn into_turn_root_failure_outcome(self, root_error: DbError) -> RelayOutcome {
        error!(
            error = %ErrorChain(&root_error),
            conversation_id = %self.conversation_id,
            root_turn_id = %self.root_turn_id,
            "Refusing to start or consume an agent stream without a durable logical turn root"
        );
        let event = AgentStreamEvent::Error(
            nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                "The assistant turn could not be initialized in conversation history",
                Some(AgentErrorCode::NomifunStateInconsistent),
            ),
        );
        let terminal = Self::terminal_from_event(&event);
        let terminal_claimed = self
            .cancellation
            .as_ref()
            .map(AgentTurnCancellation::try_claim_terminal_surface)
            .unwrap_or(true);
        if terminal_claimed {
            self.forward_to_websocket(&event);
            if let Some(cancellation) = self.cancellation.as_ref() {
                cancellation.mark_terminal_observed();
            }
        }
        RelayOutcome {
            terminal,
            ..RelayOutcome::default()
        }
    }

    fn is_compatible_turn_root(&self, row: &MessageRow) -> bool {
        if row.message_id != self.root_turn_id
            || row.conversation_id != self.conversation_id
            || row.msg_id.as_deref() != Some(self.root_turn_id.as_str())
        {
            return false;
        }

        let content = serde_json::from_str::<Value>(&row.content).ok();
        let canonical = matches!(row.r#type.as_str(), "turn_root" | "system")
            && row.position.as_deref() == Some("center")
            && row.status.as_deref() == Some("finish")
            && row.hidden
            && content
                .as_ref()
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str)
                == Some("turn_root");
        if canonical {
            return true;
        }

        // Before structural roots existed, the first visible segment reused
        // the logical root id. Accept only the two representations the old
        // relay could create, with their exact assistant-side ownership.
        if row.position.as_deref() != Some("left") {
            return false;
        }
        match (row.r#type.as_str(), content.as_ref()) {
            ("text", Some(content)) => {
                content.get("turn_id").and_then(Value::as_str)
                    == Some(self.root_turn_id.as_str())
                    && content.get("content").is_some_and(Value::is_string)
            }
            ("thinking", Some(content)) => {
                !row.hidden
                    && content.get("content").is_some_and(Value::is_string)
                    && content.get("status").and_then(Value::as_str) == Some("done")
            }
            _ => false,
        }
    }

    /// Wire the process-wide runtime state so this relay accumulates each turn's
    /// `TurnCompleted` token usage into the conversation's running total (read
    /// back by the owning execution attempt after the turn settles). The
    /// Conversation service wires it only for an active attempt relation. Default
    /// chat and companion turns leave it unset.
    pub fn with_runtime_state(mut self, runtime_state: Arc<ConversationRuntimeStateService>) -> Self {
        self.runtime_state = Some(runtime_state);
        self
    }

    pub fn with_cancellation(mut self, cancellation: AgentTurnCancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn with_artifact_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.artifact_workspace = Some(workspace.into());
        self
    }

    fn allows_legacy_unjournaled_artifacts(&self) -> bool {
        #[cfg(test)]
        {
            return self.allow_legacy_unjournaled_artifacts;
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    /// Wire the pre-response failover error-suppressor (review #1/#5). When the
    /// predicate returns `true` for a pre-response provider-fault's error code,
    /// the relay swallows the user-visible error (no WS error event, no error
    /// `tips` row) because the send loop will fail over and re-run the turn.
    pub fn with_failover_suppressor(
        mut self,
        suppressor: Arc<dyn Fn(AgentErrorCode) -> bool + Send + Sync>,
    ) -> Self {
        self.failover_suppressor = Some(suppressor);
        self
    }

    /// Tag this relay's broadcasts with the conversation's companion-companion
    /// markers (no-op markers by default; see field docs).
    pub fn with_companion_context(
        mut self,
        companion: bool,
        companion_id: Option<CompanionId>,
    ) -> Self {
        self.companion = companion;
        self.companion_id = companion_id;
        self
    }

    /// Tag this relay's broadcasts with the originating user message's
    /// `origin` marker (see field docs). Blank values normalize to `None`.
    pub fn with_origin(mut self, origin: Option<String>) -> Self {
        self.origin = origin
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        self
    }

    /// Tag this relay's broadcasts with the conversation's IM platform
    /// marker (see field docs). Blank values normalize to `None`.
    pub fn with_channel_platform(mut self, channel_platform: Option<String>) -> Self {
        self.channel_platform = channel_platform
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        self
    }

    /// Mark this relay as serving a robot gateway thread, which makes it delete
    /// bracketed stage directions from assistant text (see field docs). Off by
    /// default; the robot's own stream clone guards itself either way.
    pub fn with_robot_session(mut self, robot_session: bool) -> Self {
        self.robot_session = robot_session;
        self
    }

    /// Run the relay loop. Consumes `self` and runs until the agent stream ends.
    #[tracing::instrument(
        skip_all,
        fields(
            conversation_id = %self.conversation_id,
            msg_id = %self.msg_id,
        )
    )]
    pub async fn consume(self, rx: broadcast::Receiver<AgentStreamEvent>) -> RelayOutcome {
        self.consume_inner(rx, None).await
    }

    /// Re-surface a terminal `Error` event the relay previously SUPPRESSED for a
    /// pending failover that then did NOT fire (review #1/#5). Mirrors the relay's
    /// own terminal-error side effects: broadcast the WS `message.stream` error
    /// event and persist the error `tips` row — so a queue-exhausted failover
    /// still shows the ORIGINAL error. No-op for non-`Error` events.
    pub async fn surface_terminal_error(
        &self,
        event: &AgentStreamEvent,
        cancellation: &AgentTurnCancellation,
    ) -> bool {
        let AgentStreamEvent::Error(data) = event else {
            return false;
        };
        if !cancellation.try_claim_terminal_surface() {
            return false;
        }
        if let Err(error) = self.ensure_turn_root_persisted().await {
            error!(
                error = %ErrorChain(&error),
                conversation_id = %self.conversation_id,
                root_turn_id = %self.root_turn_id,
                "Could not persist the logical turn root before surfacing a terminal error"
            );
            cancellation.mark_terminal_observed();
            return false;
        }
        if cancellation.is_cancelled() {
            self.forward_to_websocket(&Self::cancelled_finish_event());
            cancellation.mark_terminal_observed();
            return false;
        }
        let error_message_id = ConversationService::mint_msg_id();
        self.forward_to_websocket_with_msg_id(&error_message_id, event);
        // This projection belongs to the still-authoritative turn owner.  Do
        // not detach or time out the insert: cancelling an in-flight database
        // future can make its commit result ambiguous and lets a later turn
        // race a write from this terminal generation.  A stop may still abort
        // the whole owner after it has established its stronger tombstone.
        self.persist_error_tips(&error_message_id, data).await;
        cancellation.mark_terminal_observed();
        true
    }

    /// Run the relay loop while also accepting a typed send failure from the
    /// task that called `AgentRuntimeControl::send_message`.
    #[tracing::instrument(
        skip_all,
        fields(
            conversation_id = %self.conversation_id,
            msg_id = %self.msg_id,
        )
    )]
    pub async fn consume_with_send_error(
        self,
        rx: broadcast::Receiver<AgentStreamEvent>,
        send_error_rx: oneshot::Receiver<Result<(), AgentSendError>>,
    ) -> RelayOutcome {
        self.consume_inner(rx, Some(send_error_rx)).await
    }

    async fn consume_inner(
        self,
        mut rx: broadcast::Receiver<AgentStreamEvent>,
        mut send_error_rx: Option<oneshot::Receiver<Result<(), AgentSendError>>>,
    ) -> RelayOutcome {
        let started_at = now_ms();
        info!("StreamRelay started");
        if let Err(root_error) = self.ensure_turn_root_persisted().await {
            return self.into_turn_root_failure_outcome(root_error);
        }
        let _artifact_recovery_lease_handoff = ArtifactRecoveryLeaseHandoff::new(
            self.artifact_workspace.as_ref(),
            self.conv_id(),
            &self.msg_id,
        );
        self.reconcile_pending_artifact_recovery_journal().await;

        let mut full_text_buffer = String::new();
        // Robot threads only (see `robot_session`): withholds at most one
        // partial bracketed run across delta boundaries. Inert when the gate is off.
        let mut stage_filter = StageDirectionFilter::default();
        let mut text_segments: Vec<PersistedTextSegment> = Vec::new();
        let mut active_text: Option<TextSegmentState> = None;
        let mut active_thinking: Option<ThinkingSegmentState> = None;
        let mut active_tool_calls: HashMap<String, ToolCallEventData> = HashMap::new();
        let mut completed_artifact_tool_calls: HashMap<String, ToolCallEventData> = HashMap::new();
        let mut terminal_tool_calls: HashSet<String> = HashSet::new();
        let mut failed_terminal_tool_calls: HashSet<String> = HashSet::new();
        let mut active_acp_tool_calls: HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        > = HashMap::new();
        let mut completed_artifact_acp_tool_calls: HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        > = HashMap::new();
        let mut terminal_acp_tool_calls: HashSet<String> = HashSet::new();
        let mut failed_terminal_acp_tool_calls: HashSet<String> = HashSet::new();
        let mut active_tool_groups: HashMap<
            String,
            Vec<nomifun_ai_agent::protocol::events::tool_call::ToolGroupEntry>,
        > = HashMap::new();
        let mut active_plan_ids: HashSet<String> = HashSet::new();
        let mut active_agent_status: Option<nomifun_ai_agent::protocol::events::AgentStatusEventData> = None;
        let mut first_agent_event_logged = false;
        let mut first_visible_output_logged = false;
        let mut fatal_tracking_error: Option<String> = None;
        // Phase 3 (plan D4): tracks whether any externally-visible response has
        // been emitted this turn — assistant Text OR a forwarded/persisted tool
        // action. Surfaced on the RelayOutcome so the failover seam can restrict
        // switching to faults that produced NO visible output (no duplicate
        // text, no duplicate tool side effect / billing).
        let mut emitted_response = false;
        let mut committed_artifact_count = 0usize;
        let mut send_error_done = send_error_rx.is_none();

        loop {
            let recv_result = if let Some(message) = fatal_tracking_error.take() {
                Ok(AgentStreamEvent::Error(
                    nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                        message,
                        Some(AgentErrorCode::NomifunStreamBroken),
                    ),
                ))
            } else {
                match (self.cancellation.as_ref(), send_error_done) {
                (Some(cancellation), true) => {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => Ok(Self::cancelled_finish_event()),
                        recv = rx.recv() => recv,
                    }
                }
                (Some(cancellation), false) => {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => Ok(Self::cancelled_finish_event()),
                        recv = rx.recv() => recv,
                        send_error = send_error_rx.as_mut().expect("send_error_rx exists while pending") => {
                            send_error_done = true;
                            match send_error {
                                Ok(Err(send_error)) => {
                                    warn!(
                                        code = ?send_error.code(),
                                        ownership = ?send_error.ownership(),
                                        "Injecting stream error for failed agent send"
                                    );
                                    Ok(AgentStreamEvent::Error(send_error.into_stream_error()))
                                }
                                Ok(Ok(())) => continue,
                                Err(_) => Ok(AgentStreamEvent::Error(
                                    nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                        "Agent send task exited before reporting acceptance",
                                        None,
                                    ),
                                )),
                            }
                        }
                    }
                }
                (None, true) => rx.recv().await,
                (None, false) => {
                    tokio::select! {
                        recv = rx.recv() => recv,
                        send_error = send_error_rx.as_mut().expect("send_error_rx exists while pending") => {
                            send_error_done = true;
                            match send_error {
                                Ok(Err(send_error)) => {
                                    warn!(
                                        code = ?send_error.code(),
                                        ownership = ?send_error.ownership(),
                                        "Injecting stream error for failed agent send"
                                    );
                                    Ok(AgentStreamEvent::Error(send_error.into_stream_error()))
                                }
                                Ok(Ok(())) => continue,
                                Err(_) => Ok(AgentStreamEvent::Error(
                                    nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                        "Agent send task exited before reporting acceptance",
                                        None,
                                    ),
                                )),
                            }
                        }
                    }
                }
            }
            };
            let recv_result = match recv_result {
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(lagged = skipped, "Stream relay lagged; terminating the incomplete event stream");
                    Ok(AgentStreamEvent::Error(
                        nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                            format!(
                                "Agent event stream lagged and skipped {skipped} events; the turn was terminated to preserve terminal-state integrity"
                            ),
                            Some(AgentErrorCode::NomifunStreamBroken),
                        ),
                    ))
                }
                result => result,
            };

            match recv_result {
                Ok(mut event) => {
                    // Cancellation is authoritative even if `rx.recv()` won
                    // just before the token fired. Re-check after receive so a
                    // concurrently queued ordinary Finish cannot execute
                    // middleware/cron or be reported as successful.
                    if self
                        .cancellation
                        .as_ref()
                        .is_some_and(AgentTurnCancellation::is_cancelled)
                        && matches!(event, AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_))
                    {
                        event = Self::cancelled_finish_event();
                    }
                    if !first_agent_event_logged {
                        first_agent_event_logged = true;
                        info!(
                            event_type = Self::event_kind(&event),
                            elapsed_ms = now_ms().saturating_sub(started_at),
                            "StreamRelay received first agent event"
                        );
                    }
                    // Repository ordering is part of the turn's durability
                    // boundary. Never drop an issued mutation to consume a
                    // later terminal: SQLite may commit the abandoned command
                    // after terminal cleanup and regress a row to `work`.

                    // Robot threads: the prompt requires plain spoken sentences,
                    // so a bracketed stage direction (`[winking]`, `[laughs]`,
                    // the dead `[emotion:x]` syntax) is a prompt violation, not
                    // content. Rewrite the event HERE, at the single point where
                    // the WS forward, `segment.buffer` and `full_text_buffer` all
                    // read `data.content` — one mutation cleans the live stream,
                    // the persisted row, /api/messages/search and the knowledge
                    // writeback together. Precedent: the cancellation rewrite
                    // above. The device path guards its own copy off its own
                    // broadcast clone.
                    if self.robot_session {
                        if let AgentStreamEvent::Text(data) = &mut event {
                            data.content = stage_filter.push(&data.content);
                        } else {
                            // Any non-Text event ends the text run, so a
                            // withheld partial bracket was literal text after
                            // all — release it verbatim, matching
                            // `strip_stage_directions`. Every
                            // `close_active_text_segment` call in this branch is
                            // downstream of here.
                            self.release_withheld_text(
                                &mut stage_filter,
                                &mut active_text,
                                &mut full_text_buffer,
                            );
                        }
                    }

                    match &event {
                        AgentStreamEvent::Thinking(data) => {
                            if data.status.as_deref() == Some("done") {
                                let _ = self
                                    .ordered_event_side_effect(
                                        "complete_thinking",
                                        self.complete_active_thinking(&mut active_thinking),
                                    )
                                    .await;
                                continue;
                            }

                            // Plan D4: a broadcast/persisted thinking segment is
                            // externally visible — once it streams we are no
                            // longer pre-response, so the failover seam stands down.
                            emitted_response = true;
                            let _ = self
                                .ordered_event_side_effect(
                                    "close_text_before_thinking",
                                    self.close_active_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        "finish",
                                    ),
                                )
                                .await;
                            if !first_visible_output_logged && !data.content.is_empty() {
                                first_visible_output_logged = true;
                                info!(
                                    event_type = "Thinking",
                                    elapsed_ms = now_ms().saturating_sub(started_at),
                                    "StreamRelay received first visible output"
                                );
                            }

                            let segment = active_thinking.get_or_insert_with(|| ThinkingSegmentState {
                                id: ConversationService::mint_msg_id(),
                                buffer: String::new(),
                                started_at: now_ms(),
                                completed_duration_ms: None,
                            });
                            segment.buffer.push_str(&data.content);
                            self.forward_to_websocket_with_msg_id(&segment.id, &event);
                        }
                        AgentStreamEvent::Text(data) => {
                            let _ = self
                                .ordered_event_side_effect(
                                    "complete_thinking_before_text",
                                    self.complete_active_thinking(&mut active_thinking),
                                )
                                .await;
                            // Plan D4: any assistant Text means we are no longer
                            // pre-response. The failover seam keys off this.
                            emitted_response = true;
                            if !first_visible_output_logged && !data.content.is_empty() {
                                first_visible_output_logged = true;
                                info!(
                                    event_type = "Text",
                                    elapsed_ms = now_ms().saturating_sub(started_at),
                                    "StreamRelay received first visible output"
                                );
                            }

                            let segment = active_text.get_or_insert_with(|| TextSegmentState {
                                id: ConversationService::mint_msg_id(),
                                buffer: String::new(),
                                created_at: now_ms(),
                                record_created: false,
                                flush_counter: 0,
                            });
                            self.forward_to_websocket_with_msg_id(&segment.id, &event);
                            segment.buffer.push_str(&data.content);
                            full_text_buffer.push_str(&data.content);
                            segment.flush_counter += 1;
                            if segment.flush_counter >= FLUSH_INTERVAL {
                                let _ = self
                                    .ordered_event_side_effect(
                                        "flush_text",
                                        self.flush_text_segment(segment),
                                    )
                                    .await;
                                segment.flush_counter = 0;
                            }
                        }
                        AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_) => {
                            if self
                                .cancellation
                                .as_ref()
                                .is_some_and(AgentTurnCancellation::is_cancelled)
                                && !Self::is_cancelled_finish(&event)
                            {
                                event = Self::cancelled_finish_event();
                            }
                            let mut terminal = Self::terminal_from_event(&event);
                            // Decide suppression before any persistence await.
                            // Terminal publication is scoped to the current
                            // wire segment. The send loop resets that scope for
                            // every continuation/failover resend, so ordinary
                            // intermediate terminals cannot mask cancellation
                            // of a later segment.
                            let mut suppress_error = !emitted_response
                                && matches!(event, AgentStreamEvent::Error(_))
                                && terminal
                                    .code()
                                    .zip(self.failover_suppressor.as_ref())
                                    .is_some_and(|(code, suppressor)| suppressor(code));
                            let mut terminal_claimed = false;
                            if !suppress_error {
                                terminal_claimed = self
                                    .cancellation
                                    .as_ref()
                                    .map(AgentTurnCancellation::try_claim_terminal_surface)
                                    .unwrap_or(true);
                                if !terminal_claimed {
                                    // A bounded stop fallback (or another
                                    // terminal publisher for this exact wire
                                    // segment) already won. Never publish or
                                    // middleware-process a late ordinary
                                    // terminal after that cancelled terminal.
                                    event = Self::cancelled_finish_event();
                                    terminal = Self::terminal_from_event(&event);
                                    suppress_error = false;
                                }
                            }
                            if let Err(recovery_error) = self
                                .merge_prepared_generic_artifact_recoveries(
                                    &mut completed_artifact_tool_calls,
                                )
                                .await
                            {
                                error!(
                                    error = %ErrorChain(&recovery_error),
                                    "Artifact terminal could not reconstruct its durable recovery envelope"
                                );
                                event = AgentStreamEvent::Error(
                                    nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                        "The generated artifacts could not be recovered from the event stream",
                                        Some(AgentErrorCode::NomifunStateInconsistent),
                                    ),
                                );
                                terminal = Self::terminal_from_event(&event);
                                suppress_error = false;
                            }
                            if let Err(recovery_error) = self
                                .merge_prepared_acp_artifact_recoveries(
                                    &mut completed_artifact_acp_tool_calls,
                                )
                                .await
                            {
                                error!(error = %ErrorChain(&recovery_error), "ACP artifact terminal could not reconstruct its recovery envelope");
                                event = AgentStreamEvent::Error(
                                    nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                        "The generated ACP artifacts could not be recovered from the event stream",
                                        Some(AgentErrorCode::NomifunStateInconsistent),
                                    ),
                                );
                                terminal = Self::terminal_from_event(&event);
                                suppress_error = false;
                            }
                            // Physical artifact snapshots are provisional until
                            // the exact terminal projection is durable. A normal
                            // unsuccessful terminal has never attempted the
                            // artifact transaction, so ownership is unambiguous
                            // and those store-owned snapshots may be rolled back.
                            let mut rollback_completed_artifact_receipts =
                                Self::invalidates_completed_artifacts(&event);
                            let mut preserve_indeterminate_artifact_rows = false;

                            // Visible assistant-segment durability is a
                            // prerequisite for committing successful artifact
                            // receipts. If this bounded write cannot settle,
                            // convert Finish before the
                            // artifact commit gate so the ordinary terminal
                            // correction path retracts every provisional
                            // receipt instead of leaving a green artifact on an
                            // otherwise inconsistent turn.
                            let text_status = if matches!(event, AgentStreamEvent::Error(_))
                                || Self::is_cancelled_finish(&event)
                            {
                                "error"
                            } else {
                                "finish"
                            };
                            // A terminal stream event is not execution-release
                            // authority.  Retain this generation and await a
                            // definitive repository result instead of dropping
                            // a database future at an arbitrary timeout
                            // cutpoint.  The service's durable Finished
                            // finalizer remains the only release point.
                            let thinking_persistence_complete = self
                                .complete_active_thinking(&mut active_thinking)
                                .await;
                            let thinking_persistence_complete = if thinking_persistence_complete {
                                true
                            } else {
                                self.retry_terminal_thinking_segment(&mut active_thinking)
                                    .await
                            };
                            self.close_active_text_segment(
                                &mut active_text,
                                &mut text_segments,
                                text_status,
                            )
                            .await;
                            let text_persistence_complete = self
                                .retry_terminal_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        text_status,
                                    )
                                    .await;
                            if (!thinking_persistence_complete || !text_persistence_complete)
                                && matches!(event, AgentStreamEvent::Finish(_))
                            {
                                event = Self::assistant_segment_persistence_error_event();
                                terminal = Self::terminal_from_event(&event);
                                suppress_error = false;
                                rollback_completed_artifact_receipts = true;
                            }

                            if terminal_claimed
                                && !Self::invalidates_completed_artifacts(&event)
                                && (!completed_artifact_tool_calls.is_empty()
                                    || !completed_artifact_acp_tool_calls.is_empty())
                            {
                                // The transaction commit is a terminal
                                // linearization point.  Timing out COMMIT would
                                // make success ambiguous and could let its late
                                // projection race the next turn, so keep the
                                // current turn admission until it returns.
                                let commit_result = self
                                    .commit_pending_artifact_deliveries(
                                        &completed_artifact_tool_calls,
                                        &completed_artifact_acp_tool_calls,
                                    )
                                    .await;

                                match commit_result {
                                    Ok(durable_artifact_count) => {
                                        committed_artifact_count = durable_artifact_count;
                                        // The transaction is now the linearization
                                        // point for artifact success. Publish every
                                        // receipt-bearing Completed frame only after
                                        // all rows committed, and still before Finish.
                                        self.broadcast_committed_artifact_tool_calls(
                                            &completed_artifact_tool_calls,
                                        );
                                        self.broadcast_committed_artifact_acp_tool_calls(
                                            &completed_artifact_acp_tool_calls,
                                        );
                                        self.finalize_generic_artifact_recovery(
                                            &completed_artifact_tool_calls,
                                        );
                                        self.finalize_acp_artifact_recovery(
                                            &completed_artifact_acp_tool_calls,
                                        );
                                        completed_artifact_tool_calls.clear();
                                        completed_artifact_acp_tool_calls.clear();
                                    }
                                    Err(commit_failure) => {
                                        error!(
                                            error = %ErrorChain(&commit_failure.error),
                                            commit_state = commit_failure.commit_state,
                                            rollback_safe = commit_failure.rollback_safe,
                                            "Atomic artifact projection failed; rejecting turn success"
                                        );
                                        rollback_completed_artifact_receipts =
                                            commit_failure.rollback_safe;
                                        if !commit_failure.rollback_safe {
                                            preserve_indeterminate_artifact_rows = true;
                                            self.mark_generic_artifact_recovery_needs_reconcile(
                                                &completed_artifact_tool_calls,
                                            );
                                            self.mark_acp_artifact_recovery_needs_reconcile(
                                                &completed_artifact_acp_tool_calls,
                                            );
                                        }
                                        event = AgentStreamEvent::Error(
                                            nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                                "The generated artifacts could not be committed to conversation history",
                                                Some(AgentErrorCode::NomifunStateInconsistent),
                                            ),
                                        );
                                        terminal = Self::terminal_from_event(&event);
                                        suppress_error = false;
                                    }
                                }
                            }
                            // A terminal error is its own durable message, not
                            // another update of the assistant text/thinking
                            // message that happened to use the turn's primary
                            // wire id. Mint the identity once and use it for
                            // both the live frame and the persisted tips row;
                            // `turn_id` retains the owning turn relation.
                            let terminal_message_id = if matches!(event, AgentStreamEvent::Error(_))
                                && !suppress_error
                            {
                                ConversationService::mint_msg_id()
                            } else {
                                self.msg_id.clone()
                            };
                            let elapsed_ms = now_ms() - started_at;
                            let event_type = if matches!(event, AgentStreamEvent::Finish(_)) {
                                "Finish"
                            } else {
                                "Error"
                            };
                            match &terminal {
                                RelayTerminal::Error { code, retryable } => {
                                    info!(
                                        event_type,
                                        elapsed_ms,
                                        text_len = full_text_buffer.len(),
                                        error_code = ?code,
                                        retryable = ?retryable,
                                        "StreamRelay received terminal event"
                                    );
                                }
                                RelayTerminal::Finish | RelayTerminal::ChannelClosed => {
                                    info!(
                                        event_type,
                                        elapsed_ms,
                                        text_len = full_text_buffer.len(),
                                        "StreamRelay received terminal event"
                                    );
                                }
                            }

                            let terminal_cleanup = async {
                            // Artifact corrections are the first terminal side
                            // effect and are all broadcast before any repository
                            // await. Even a wedged DB cannot leave strict live
                            // consumers with an earlier green receipt.
                            let invalidates_artifacts =
                                !suppress_error && Self::invalidates_completed_artifacts(&event);
                            if invalidates_artifacts && rollback_completed_artifact_receipts {
                                self.rollback_completed_artifact_receipts(
                                    &completed_artifact_tool_calls,
                                    &completed_artifact_acp_tool_calls,
                                );
                            }
                            let (failed_completed_tools, failed_completed_acp_tools) =
                                if invalidates_artifacts {
                                    let reason = Self::incomplete_tool_reason(&event)
                                        .unwrap_or("incomplete_turn");
                                    let tools = Self::take_failed_tool_calls(
                                        &mut completed_artifact_tool_calls,
                                        reason,
                                    );
                                    let acp_tools = Self::take_failed_acp_tool_calls(
                                        &mut completed_artifact_acp_tool_calls,
                                        reason,
                                    );
                                    self.broadcast_failed_tool_calls(&tools);
                                    self.broadcast_failed_acp_tool_calls(&acp_tools);
                                    (tools, acp_tools)
                                } else {
                                    (Vec::new(), Vec::new())
                                };

                            if !preserve_indeterminate_artifact_rows {
                                let _ = tokio::join!(
                                    self.persist_failed_tool_calls(&failed_completed_tools),
                                    self.persist_failed_acp_tool_calls(&failed_completed_acp_tools),
                                );
                            }
                            // review #1/#5: a pre-response provider-fault that the
                            // send loop will fail over must NOT reach the user —
                            // suppress the WS error event AND the error `tips` row
                            // at source, so a recovered turn shows only the backup
                            // model's output. Only the Error terminal with no
                            // emitted response and a positive suppressor verdict
                            // qualifies; everything else broadcasts/persists as before.
                            if suppress_error {
                                info!("StreamRelay suppressing pre-response error pending model failover");
                            } else {
                                if let Some(reason) = Self::incomplete_tool_reason(&event) {
                                    // A provider can emit a per-tool Completed frame and then
                                    // fail/cancel/truncate the enclosing turn. Artifact success
                                    // is a turn-level contract, so retract those receipts on an
                                    // unsuccessful terminal. A normal EndTurn/unspecified Finish
                                    // keeps already verified completed artifacts, while still
                                    // closing genuinely Running tools below.
                                    self.fail_active_tool_calls(&mut active_tool_calls, reason).await;
                                    self.fail_active_acp_tool_calls(&mut active_acp_tool_calls, reason).await;
                                    self.fail_active_tool_groups(&mut active_tool_groups, reason).await;
                                }
                            }
                            self.finalize_active_plans(
                                &mut active_plan_ids,
                                Self::plan_terminal_status(&event),
                            )
                            .await;
                            self.finalize_active_agent_status(
                                &mut active_agent_status,
                                Self::plan_terminal_status(&event),
                            )
                            .await;
                            let outcome = self
                                .finalize(
                                    &full_text_buffer,
                                    &text_segments,
                                    text_persistence_complete,
                                    &event,
                                    terminal,
                                    emitted_response,
                                    suppress_error,
                                    &terminal_message_id,
                                    committed_artifact_count,
                                )
                                .await;
                            // Publish the terminal only after all lifecycle
                            // corrections. Strict consumers may stop reading at
                            // Error/Finish, so a receipt retraction sent after it
                            // would leave stale success visible.
                            if terminal_claimed {
                                self.forward_to_websocket_with_msg_id(&terminal_message_id, &event);
                            }
                            outcome
                            };
                            let outcome = terminal_cleanup.await;
                            if terminal_claimed
                                && let Some(cancellation) = self.cancellation.as_ref()
                            {
                                // Relay persistence/finalization is complete
                                // and the authoritative Finish is already on
                                // the wire. The stop worker may now release the
                                // exact generation and publish turn.completed.
                                cancellation.mark_terminal_observed();
                            }
                            #[cfg(test)]
                            if self.complete_turn {
                                Self::complete_conversation_with_context(
                                    &self.repo,
                                    &self.user_events,
                                    &self.user_id,
                                    &self.conversation_id,
                                    Some(self.root_turn_id.clone()),
                                    None,
                                    self.companion,
                                    self.companion_id.clone(),
                                    self.origin.clone(),
                                    self.channel_platform.clone(),
                                )
                                .await;
                            }
                            break outcome;
                        }
                        AgentStreamEvent::ToolCall(data) => {
                            // Plan D4: a forwarded/persisted tool call is an
                            // externally-visible action with a side effect — no
                            // failover after this, or the tool would re-run.
                            emitted_response = true;
                            let has_artifact_delivery =
                                data.status == ToolCallStatus::Completed && !data.artifacts.is_empty();
                            let active_contract_source = active_tool_calls.get(&data.call_id).cloned();
                            let artifact_contract_error = if data.status == ToolCallStatus::Completed {
                                let terminal_error = validate_completed_artifact_contract(data).err();
                                terminal_error.or_else(|| {
                                    active_contract_source.as_ref().and_then(|active| {
                                        let mut effective = active.clone();
                                        effective.status = ToolCallStatus::Completed;
                                        effective.artifacts = data.artifacts.clone();
                                        validate_completed_artifact_contract(&effective).err()
                                    })
                                })
                            } else {
                                None
                            };
                            let mut tracking_overflow = false;
                            match data.status {
                                ToolCallStatus::Running => {
                                    if terminal_tool_calls.contains(&data.call_id) {
                                        warn!(
                                            call_id = %data.call_id,
                                            tool = %data.name,
                                            "Ignoring late running event for terminal tool call"
                                        );
                                        continue;
                                    }
                                    tracking_overflow |= !track_bounded(
                                        &mut active_tool_calls,
                                        data.call_id.clone(),
                                        data.clone(),
                                        "tool_call",
                                    );
                                }
                                ToolCallStatus::Completed | ToolCallStatus::Error => {
                                    if terminal_tool_calls.contains(&data.call_id) {
                                        if data.status == ToolCallStatus::Error
                                            && !failed_terminal_tool_calls.contains(&data.call_id)
                                        {
                                            tracking_overflow |= !remember_bounded(
                                                &mut failed_terminal_tool_calls,
                                                data.call_id.clone(),
                                                "failed_terminal_tool_call",
                                            );
                                        } else {
                                            warn!(
                                                call_id = %data.call_id,
                                                tool = %data.name,
                                                status = ?data.status,
                                                "Ignoring duplicate or non-failing terminal event for tool call"
                                            );
                                            continue;
                                        }
                                    } else {
                                        tracking_overflow |= !remember_bounded(
                                            &mut terminal_tool_calls,
                                            data.call_id.clone(),
                                            "terminal_tool_call",
                                        );
                                        if data.status == ToolCallStatus::Error {
                                            tracking_overflow |= !remember_bounded(
                                                &mut failed_terminal_tool_calls,
                                                data.call_id.clone(),
                                                "failed_terminal_tool_call",
                                            );
                                        }
                                    }
                                    active_tool_calls.remove(&data.call_id);
                                    if has_artifact_delivery && artifact_contract_error.is_none() {
                                        tracking_overflow |= !track_bounded(
                                            &mut completed_artifact_tool_calls,
                                            data.call_id.clone(),
                                            data.clone(),
                                            "completed_artifact_tool_call",
                                        );
                                    } else {
                                        completed_artifact_tool_calls.remove(&data.call_id);
                                    }
                                }
                            }
                            if tracking_overflow {
                                active_tool_calls.remove(&data.call_id);
                                completed_artifact_tool_calls.remove(&data.call_id);
                                let mut failed = data.clone();
                                failed.status = ToolCallStatus::Error;
                                failed.artifacts.clear();
                                failed.output = Some(
                                    "The turn exceeded its safe tool-lifecycle tracking limit; artifact delivery was rejected"
                                        .to_owned(),
                                );
                                let failed_event = AgentStreamEvent::ToolCall(failed.clone());
                                self.forward_to_websocket(&failed_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_tool_tracking_overflow",
                                        self.persist_tool_call(&failed),
                                    )
                                    .await;
                                fatal_tracking_error = Some(
                                    "The agent emitted more tool lifecycle events than can be verified safely; the turn was terminated"
                                        .to_owned(),
                                );
                                continue;
                            }
                            if let Some(contract_error) = artifact_contract_error {
                                completed_artifact_tool_calls.remove(&data.call_id);
                                let mut failed = data.clone();
                                failed.status = ToolCallStatus::Error;
                                failed.artifacts.clear();
                                failed.output = Some(contract_error.clone());
                                let failed_event = AgentStreamEvent::ToolCall(failed.clone());
                                self.forward_to_websocket(&failed_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_artifact_contract_failure",
                                        self.persist_tool_call(&failed),
                                    )
                                    .await;
                                fatal_tracking_error = Some(format!(
                                    "Artifact delivery contract failed; the turn was terminated: {contract_error}"
                                ));
                                continue;
                            }
                            let _ = self
                                .ordered_event_side_effect(
                                    "complete_thinking_before_tool",
                                    self.complete_active_thinking(&mut active_thinking),
                                )
                                .await;
                            let _ = self
                                .ordered_event_side_effect(
                                    "close_text_before_tool",
                                    self.close_active_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        "finish",
                                    ),
                                )
                                .await;
                            if has_artifact_delivery {
                                let ownership_ready = self
                                    .ordered_event_side_effect(
                                        "claim_artifact_tool_recovery",
                                        self.claim_generic_artifact_recovery(data),
                                    )
                                    .await;
                                if let Err(error) = ownership_ready {
                                    error!(
                                        call_id = %data.call_id,
                                        error = %ErrorChain(&error),
                                        "Artifact delivery could not transfer its recovery journal"
                                    );
                                    completed_artifact_tool_calls.remove(&data.call_id);
                                    let mut failed = data.clone();
                                    failed.status = ToolCallStatus::Error;
                                    failed.artifacts.clear();
                                    failed.output = Some(
                                        "Artifact delivery could not claim a durable message identity"
                                            .to_owned(),
                                    );
                                    self.forward_to_websocket(&AgentStreamEvent::ToolCall(failed));
                                    fatal_tracking_error = Some(
                                        "Artifact delivery could not claim durable recovery ownership; the turn was terminated"
                                            .to_owned(),
                                    );
                                    continue;
                                }

                                // Do not expose a green receipt before the
                                // enclosing turn commits. Live clients receive
                                // the same receipt-free provisional lifecycle as
                                // history hydration; the full Completed frame is
                                // published by the terminal commit barrier.
                                let provisional = Self::provisional_artifact_tool_call(data);
                                self.forward_to_websocket(&AgentStreamEvent::ToolCall(provisional));
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_provisional_artifact_tool_call",
                                        self.persist_provisional_artifact_tool_call(data),
                                    )
                                    .await;
                            } else {
                                self.forward_to_websocket(&event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_tool_call",
                                        self.persist_tool_call(data),
                                    )
                                    .await;
                            }
                        }
                        AgentStreamEvent::AcpToolCall(data) => {
                            // Plan D4: see ToolCall — an ACP tool call is a
                            // visible, side-effecting action; block failover.
                            emitted_response = true;
                            let tool_call_id = data.update.tool_call_id.clone();
                            let effective_data = effective_acp_tool_call_projection(
                                active_acp_tool_calls.get(&tool_call_id),
                                data,
                            );
                            let has_artifact_delivery = effective_data
                                .update
                                .content
                                .as_ref()
                                .is_some_and(|items| {
                                    items.iter().any(|item| {
                                        matches!(
                                            item,
                                            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                                                | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                                        )
                                    })
                                });
                            let artifact_contract_error = if effective_data.update.status
                                == Some(AcpToolCallStatus::Completed)
                            {
                                validate_completed_acp_artifact_contract(&effective_data).err()
                            } else {
                                None
                            };
                            let mut tracking_overflow = false;
                            match effective_data.update.status {
                                Some(AcpToolCallStatus::Completed | AcpToolCallStatus::Failed) => {
                                    if terminal_acp_tool_calls.contains(&tool_call_id) {
                                        if effective_data.update.status == Some(AcpToolCallStatus::Failed)
                                            && !failed_terminal_acp_tool_calls.contains(&tool_call_id)
                                        {
                                            tracking_overflow |= !remember_bounded(
                                                &mut failed_terminal_acp_tool_calls,
                                                tool_call_id.clone(),
                                                "failed_terminal_acp_tool_call",
                                            );
                                        } else {
                                            warn!(
                                                tool_call_id,
                                                status = ?effective_data.update.status,
                                                "Ignoring duplicate or non-failing terminal ACP tool event"
                                            );
                                            continue;
                                        }
                                    } else {
                                        tracking_overflow |= !remember_bounded(
                                            &mut terminal_acp_tool_calls,
                                            tool_call_id.clone(),
                                            "terminal_acp_tool_call",
                                        );
                                        if effective_data.update.status == Some(AcpToolCallStatus::Failed) {
                                            tracking_overflow |= !remember_bounded(
                                                &mut failed_terminal_acp_tool_calls,
                                                tool_call_id.clone(),
                                                "failed_terminal_acp_tool_call",
                                            );
                                        }
                                    }
                                    active_acp_tool_calls.remove(&tool_call_id);
                                    if effective_data.update.status == Some(AcpToolCallStatus::Completed)
                                        && has_artifact_delivery
                                        && artifact_contract_error.is_none()
                                    {
                                        tracking_overflow |= !track_bounded(
                                            &mut completed_artifact_acp_tool_calls,
                                            tool_call_id.clone(),
                                            effective_data.clone(),
                                            "completed_artifact_acp_tool_call",
                                        );
                                    } else {
                                        completed_artifact_acp_tool_calls.remove(&tool_call_id);
                                    }
                                }
                                Some(AcpToolCallStatus::Pending | AcpToolCallStatus::InProgress) | None => {
                                    if terminal_acp_tool_calls.contains(&tool_call_id) {
                                        warn!(
                                            tool_call_id,
                                            "Ignoring late progress event for terminal ACP tool call"
                                        );
                                        continue;
                                    }
                                    tracking_overflow |= !track_bounded(
                                        &mut active_acp_tool_calls,
                                        tool_call_id.clone(),
                                        effective_data.clone(),
                                        "acp_tool_call",
                                    );
                                }
                            }
                            if tracking_overflow {
                                active_acp_tool_calls.remove(&tool_call_id);
                                completed_artifact_acp_tool_calls.remove(&tool_call_id);
                                let mut failed = effective_data.clone();
                                failed.update.session_update = AcpToolCallSessionUpdateKind::ToolCallUpdate;
                                failed.update.status = Some(AcpToolCallStatus::Failed);
                                failed.update.raw_output = Some(json!(
                                    "The turn exceeded its safe tool-lifecycle tracking limit; artifact delivery was rejected"
                                ));
                                if let Some(content) = failed.update.content.as_mut() {
                                    content.retain(|item| {
                                        !matches!(
                                            item,
                                            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                                                | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                                        )
                                    });
                                }
                                let failed_event = AgentStreamEvent::AcpToolCall(failed.clone());
                                self.forward_to_websocket(&failed_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_acp_tracking_overflow",
                                        self.persist_acp_tool_call(&failed),
                                    )
                                    .await;
                                fatal_tracking_error = Some(
                                    "The agent emitted more ACP tool lifecycle events than can be verified safely; the turn was terminated"
                                        .to_owned(),
                                );
                                continue;
                            }
                            if let Some(contract_error) = artifact_contract_error {
                                completed_artifact_acp_tool_calls.remove(&tool_call_id);
                                let mut failed = effective_data.clone();
                                failed.update.session_update =
                                    AcpToolCallSessionUpdateKind::ToolCallUpdate;
                                failed.update.status = Some(AcpToolCallStatus::Failed);
                                failed.update.raw_output = Some(json!(contract_error.clone()));
                                if let Some(content) = failed.update.content.as_mut() {
                                    content.retain(|item| {
                                        !matches!(
                                            item,
                                            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                                                | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                                        )
                                    });
                                }
                                let failed_event = AgentStreamEvent::AcpToolCall(failed.clone());
                                self.forward_to_websocket(&failed_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_acp_artifact_contract_failure",
                                        self.persist_acp_tool_call(&failed),
                                    )
                                    .await;
                                fatal_tracking_error = Some(format!(
                                    "ACP artifact delivery contract failed; the turn was terminated: {contract_error}"
                                ));
                                continue;
                            }
                            let _ = self
                                .ordered_event_side_effect(
                                    "complete_thinking_before_acp_tool",
                                    self.complete_active_thinking(&mut active_thinking),
                                )
                                .await;
                            let _ = self
                                .ordered_event_side_effect(
                                    "close_text_before_acp_tool",
                                    self.close_active_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        "finish",
                                    ),
                                )
                                .await;
                            if effective_data.update.status == Some(AcpToolCallStatus::Completed)
                                && has_artifact_delivery
                            {
                                let ownership_ready = self
                                    .ordered_event_side_effect(
                                        "claim_artifact_acp_tool_recovery",
                                        self.claim_acp_artifact_recovery(&effective_data),
                                    )
                                    .await;
                                if ownership_ready.is_err() {
                                    completed_artifact_acp_tool_calls.remove(&tool_call_id);
                                    let mut failed = effective_data.clone();
                                    failed.update.session_update =
                                        AcpToolCallSessionUpdateKind::ToolCallUpdate;
                                    failed.update.status = Some(AcpToolCallStatus::Failed);
                                    failed.update.raw_output = Some(json!(
                                        "Artifact delivery could not claim a durable message identity"
                                    ));
                                    if let Some(content) = failed.update.content.as_mut() {
                                        content.retain(|item| {
                                            !matches!(
                                                item,
                                                nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                                                    | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                                            )
                                        });
                                    }
                                    self.forward_to_websocket(&AgentStreamEvent::AcpToolCall(failed));
                                    fatal_tracking_error = Some(
                                        "ACP artifact delivery could not be projected durably; the turn was terminated"
                                            .to_owned(),
                                    );
                                    continue;
                                }

                                let provisional =
                                    Self::provisional_artifact_acp_tool_call(&effective_data);
                                self.forward_to_websocket(&AgentStreamEvent::AcpToolCall(provisional));
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_provisional_artifact_acp_tool_call",
                                        self.persist_provisional_artifact_acp_tool_call(
                                            &effective_data,
                                        ),
                                    )
                                    .await;
                            } else {
                                self.forward_to_websocket(&AgentStreamEvent::AcpToolCall(
                                    effective_data.clone(),
                                ));
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_acp_tool_call",
                                        self.persist_acp_tool_call(&effective_data),
                                    )
                                    .await;
                            }
                        }
                        AgentStreamEvent::ToolGroup(entries) => {
                            // Plan D4: see ToolCall — a tool group is a visible,
                            // side-effecting action; block failover.
                            emitted_response = true;
                            let artifact_contract_errors = tool_group_artifact_contract_errors(
                                entries,
                                &completed_artifact_tool_calls,
                            );
                            if !artifact_contract_errors.is_empty() {
                                let mut failed = entries.clone();
                                let mut reasons = Vec::with_capacity(artifact_contract_errors.len());
                                for (index, contract_error) in artifact_contract_errors {
                                    if let Some(entry) = failed.get_mut(index) {
                                        entry.status = ToolCallStatus::Error;
                                        entry.description = Some(contract_error.clone());
                                    }
                                    reasons.push(contract_error);
                                }
                                if let Some(group_id) = failed.first().map(|entry| &entry.call_id) {
                                    active_tool_groups.remove(group_id);
                                }
                                let failed_event = AgentStreamEvent::ToolGroup(failed.clone());
                                self.forward_to_websocket(&failed_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_tool_group_artifact_contract_failure",
                                        self.persist_tool_group(&failed),
                                    )
                                    .await;
                                fatal_tracking_error = Some(format!(
                                    "Tool-group artifact delivery contract failed; the turn was terminated: {}",
                                    reasons.join("; ")
                                ));
                                continue;
                            }
                            // ToolGroupEntry cannot carry a receipt or 2PC
                            // marker, so it can never be an authoritative
                            // artifact-success carrier. Suppress high-signal
                            // entries and rely on their detailed ToolCall row;
                            // retain unrelated summaries from a mixed group.
                            let visible_entries = entries
                                .iter()
                                .filter(|entry| !tool_group_entry_has_artifact_contract(entry))
                                .cloned()
                                .collect::<Vec<_>>();
                            let entries = visible_entries.as_slice();
                            if entries.is_empty() {
                                continue;
                            }
                            if let Some(group_id) = entries.first().map(|entry| entry.call_id.clone()) {
                                if entries.iter().any(|entry| entry.status == ToolCallStatus::Running) {
                                    let mut tracked_entries = entries.to_vec();
                                    tracked_entries.truncate(MAX_TERMINAL_ACTIVE_ITEMS);
                                    track_bounded(
                                        &mut active_tool_groups,
                                        group_id,
                                        tracked_entries,
                                        "tool_group",
                                    );
                                } else {
                                    active_tool_groups.remove(&group_id);
                                }
                            }
                            let _ = self
                                .ordered_event_side_effect(
                                    "complete_thinking_before_tool_group",
                                    self.complete_active_thinking(&mut active_thinking),
                                )
                                .await;
                            let _ = self
                                .ordered_event_side_effect(
                                    "close_text_before_tool_group",
                                    self.close_active_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        "finish",
                                    ),
                                )
                                .await;
                            self.forward_to_websocket(&AgentStreamEvent::ToolGroup(entries.to_vec()));
                            let _ = self
                                .ordered_event_side_effect(
                                    "persist_tool_group",
                                    self.persist_tool_group(entries),
                                )
                                .await;
                        }
                        AgentStreamEvent::AgentStatus(data) => {
                            self.forward_to_websocket(&event);
                            if data.backend == "nomi" && (data.status == "preparing" || data.status == "prepared") {
                                active_agent_status = Some(data.clone());
                                let persisted = self
                                    .ordered_event_side_effect(
                                        "persist_agent_status",
                                        self.persist_agent_status(data),
                                    )
                                    .await;
                                if data.status == "prepared" && persisted {
                                    active_agent_status = None;
                                }
                            }
                        }
                        AgentStreamEvent::Plan(data) => {
                            emitted_response = true;
                            let _ = self
                                .ordered_event_side_effect(
                                    "complete_thinking_before_plan",
                                    self.complete_active_thinking(&mut active_thinking),
                                )
                                .await;
                            let _ = self
                                .ordered_event_side_effect(
                                    "close_text_before_plan",
                                    self.close_active_text_segment(
                                        &mut active_text,
                                        &mut text_segments,
                                        "finish",
                                    ),
                                )
                                .await;
                            if let Some(source_call_id) = data.source_call_id.as_deref() {
                                let mut source = active_tool_calls.remove(source_call_id).unwrap_or_else(|| {
                                    ToolCallEventData {
                                        call_id: source_call_id.to_owned(),
                                        name: "update_plan".to_owned(),
                                        args: serde_json::Value::Null,
                                        status: ToolCallStatus::Running,
                                        input: None,
                                        output: None,
                                        description: None,
                                        artifacts: Vec::new(),
                                        retry: None,
                                    }
                                });
                                source.status = ToolCallStatus::Completed;
                                source.output = Some("Plan updated".to_owned());
                                remember_bounded(
                                    &mut terminal_tool_calls,
                                    source_call_id.to_owned(),
                                    "terminal_tool_call",
                                );
                                let source_event = AgentStreamEvent::ToolCall(source.clone());
                                self.forward_to_websocket_hidden(&source_event);
                                let _ = self
                                    .ordered_event_side_effect(
                                        "persist_plan_source_tool",
                                        self.persist_tool_call_with_hidden(&source, true),
                                    )
                                    .await;
                            }
                            let plan_id = self
                                .ordered_event_side_effect(
                                    "resolve_plan_message_id",
                                    self.plan_message_id(data),
                                )
                                .await;
                            if data.entries.iter().all(|entry| {
                                entry.get("status").and_then(serde_json::Value::as_str) == Some("completed")
                            }) {
                                active_plan_ids.remove(&plan_id);
                            } else {
                                remember_bounded(
                                    &mut active_plan_ids,
                                    plan_id.clone(),
                                    "active_plan",
                                );
                            }
                            self.forward_to_websocket_with_msg_id(&plan_id, &event);
                            let _ = self
                                .ordered_event_side_effect(
                                    "persist_plan",
                                    self.persist_plan(data),
                                )
                                .await;
                        }
                        AgentStreamEvent::TurnCompleted(metrics) => {
                            // Accumulate this turn's token usage into the owning
                            // execution attempt's conversation total. The caller
                            // already validated the explicit active relation.
                            // `context_tokens` is a gauge (last-request occupancy), so
                            // per-turn COST is the additive `input + output`. Recorded
                            // BEFORE the turn handle releases, so the polling attempt
                            // never races it.
                            if let Some(runtime_state) = self.runtime_state.as_ref() {
                                let turn_tokens =
                                    metrics.input_tokens.saturating_add(metrics.output_tokens);
                                runtime_state
                                    .add_turn_tokens(&self.conversation_id, turn_tokens as i64);
                            }
                            self.forward_to_websocket(&event);
                        }
                        _ => {
                            self.forward_to_websocket(&event);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // The only loop exit that never passes through the
                    // `Ok(mut event)` branch, so it owns the second (and last)
                    // release of withheld robot text — before the terminal
                    // cleanup block borrows `active_text` / `full_text_buffer`
                    // and closes the segment below.
                    if self.robot_session {
                        self.release_withheld_text(
                            &mut stage_filter,
                            &mut active_text,
                            &mut full_text_buffer,
                        );
                    }
                    let elapsed_ms = now_ms() - started_at;
                    warn!(
                        elapsed_ms,
                        text_len = full_text_buffer.len(),
                        "StreamRelay channel closed without terminal event"
                    );

                    let mut terminal_event = if self
                        .cancellation
                        .as_ref()
                        .is_some_and(AgentTurnCancellation::is_cancelled)
                    {
                        Self::cancelled_finish_event()
                    } else {
                        AgentStreamEvent::Error(
                            nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                                "Agent event channel closed before the turn completed",
                                None,
                            ),
                        )
                    };
                    if self
                        .cancellation
                        .as_ref()
                        .is_some_and(AgentTurnCancellation::is_cancelled)
                    {
                        terminal_event = Self::cancelled_finish_event();
                    }
                    let terminal_claimed = self
                        .cancellation
                        .as_ref()
                        .map(AgentTurnCancellation::try_claim_terminal_surface)
                        .unwrap_or(true);
                    let mut terminal = if Self::is_cancelled_finish(&terminal_event) {
                        RelayTerminal::Finish
                    } else {
                        RelayTerminal::ChannelClosed
                    };
                    let mut terminal_message_id = if matches!(terminal_event, AgentStreamEvent::Error(_)) {
                        ConversationService::mint_msg_id()
                    } else {
                        self.msg_id.clone()
                    };
                    if let Err(recovery_error) = self
                        .merge_prepared_generic_artifact_recoveries(
                            &mut completed_artifact_tool_calls,
                        )
                        .await
                    {
                        error!(
                            error = %ErrorChain(&recovery_error),
                            "Closed artifact stream retained an unreconciled recovery envelope"
                        );
                    }
                    if let Err(recovery_error) = self
                        .merge_prepared_acp_artifact_recoveries(
                            &mut completed_artifact_acp_tool_calls,
                        )
                        .await
                    {
                        error!(error = %ErrorChain(&recovery_error), "Closed ACP artifact stream retained an unreconciled recovery envelope");
                    }
                    let terminal_cleanup = async {
                        let incomplete_reason = if Self::is_cancelled_finish(&terminal_event) {
                            "cancelled"
                        } else {
                            "channel_closed"
                        };
                        // No artifact transaction is ever attempted on this
                        // branch, so the relay still has unambiguous ownership
                        // of every provisional snapshot.
                        self.rollback_completed_artifact_receipts(
                            &completed_artifact_tool_calls,
                            &completed_artifact_acp_tool_calls,
                        );
                        let failed_completed_tools = Self::take_failed_tool_calls(
                            &mut completed_artifact_tool_calls,
                            incomplete_reason,
                        );
                        let failed_completed_acp_tools = Self::take_failed_acp_tool_calls(
                            &mut completed_artifact_acp_tool_calls,
                            incomplete_reason,
                        );
                        self.broadcast_failed_tool_calls(&failed_completed_tools);
                        self.broadcast_failed_acp_tool_calls(&failed_completed_acp_tools);
                        let _ = tokio::join!(
                            self.persist_failed_tool_calls(&failed_completed_tools),
                            self.persist_failed_acp_tool_calls(&failed_completed_acp_tools),
                        );
                        let thinking_persistence_complete = self
                            .complete_active_thinking(&mut active_thinking)
                            .await;
                        let thinking_persistence_complete = if thinking_persistence_complete {
                            true
                        } else {
                            self.retry_terminal_thinking_segment(&mut active_thinking)
                                .await
                        };
                        self.close_active_text_segment(
                            &mut active_text,
                            &mut text_segments,
                            "error",
                        )
                        .await;
                        self.fail_active_tool_calls(&mut active_tool_calls, incomplete_reason).await;
                        self.fail_active_acp_tool_calls(&mut active_acp_tool_calls, incomplete_reason)
                            .await;
                        self.fail_active_tool_groups(&mut active_tool_groups, incomplete_reason)
                            .await;
                        self.finalize_active_plans(
                            &mut active_plan_ids,
                            Self::plan_terminal_status(&terminal_event),
                        )
                        .await;
                        self.finalize_active_agent_status(
                            &mut active_agent_status,
                            Self::plan_terminal_status(&terminal_event),
                        )
                        .await;
                        let text_persistence_complete = self
                            .retry_terminal_text_segment(
                                &mut active_text,
                                &mut text_segments,
                                "error",
                            )
                            .await;
                        if (!thinking_persistence_complete || !text_persistence_complete)
                            && matches!(terminal_event, AgentStreamEvent::Finish(_))
                        {
                            terminal_event = Self::assistant_segment_persistence_error_event();
                            terminal = Self::terminal_from_event(&terminal_event);
                            terminal_message_id = ConversationService::mint_msg_id();
                        }
                        let outcome = self
                            .finalize(
                                &full_text_buffer,
                                &text_segments,
                                text_persistence_complete,
                                &terminal_event,
                                terminal,
                                emitted_response,
                                // A channel-closed terminal is never a
                                // suppressible provider failure.
                                false,
                                &terminal_message_id,
                                0,
                            )
                            .await;
                        if terminal_claimed {
                            self.forward_to_websocket_with_msg_id(&terminal_message_id, &terminal_event);
                        }
                        outcome
                    };
                    let outcome = terminal_cleanup.await;
                    if terminal_claimed
                        && let Some(cancellation) = self.cancellation.as_ref()
                    {
                        cancellation.mark_terminal_observed();
                    }
                    #[cfg(test)]
                    if self.complete_turn {
                        Self::complete_conversation_with_context(
                            &self.repo,
                            &self.user_events,
                            &self.user_id,
                            &self.conversation_id,
                            Some(self.root_turn_id.clone()),
                            None,
                            self.companion,
                            self.companion_id.clone(),
                            self.origin.clone(),
                            self.channel_platform.clone(),
                        )
                        .await;
                    }
                    break outcome;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    unreachable!("lagged receive results are normalized to terminal errors")
                }
            }
        }
    }

    fn event_kind(event: &AgentStreamEvent) -> &'static str {
        match event {
            AgentStreamEvent::Start(_) => "Start",
            AgentStreamEvent::Text(_) => "Text",
            AgentStreamEvent::Tips(_) => "Tips",
            AgentStreamEvent::Thinking(_) => "Thinking",
            AgentStreamEvent::ToolCall(_) => "ToolCall",
            AgentStreamEvent::AcpToolCall(_) => "AcpToolCall",
            AgentStreamEvent::ToolGroup(_) => "ToolGroup",
            AgentStreamEvent::AgentStatus(_) => "AgentStatus",
            AgentStreamEvent::Plan(_) => "Plan",
            AgentStreamEvent::Permission(_) => "Permission",
            AgentStreamEvent::AcpPermission(_) => "AcpPermission",
            AgentStreamEvent::SkillSuggest(_) => "SkillSuggest",
            AgentStreamEvent::CronTrigger(_) => "CronTrigger",
            AgentStreamEvent::AcpModelInfo(_) => "AcpModelInfo",
            AgentStreamEvent::AcpModeInfo(_) => "AcpModeInfo",
            AgentStreamEvent::AcpConfigOption(_) => "AcpConfigOption",
            AgentStreamEvent::AcpSessionInfo(_) => "AcpSessionInfo",
            AgentStreamEvent::AcpContextUsage(_) => "AcpContextUsage",
            AgentStreamEvent::SlashCommandsUpdated(_) => "SlashCommandsUpdated",
            AgentStreamEvent::AvailableCommands(_) => "AvailableCommands",
            AgentStreamEvent::TurnCompleted(_) => "TurnCompleted",
            AgentStreamEvent::Finish(_) => "Finish",
            AgentStreamEvent::Error(_) => "Error",
            AgentStreamEvent::System(_) => "System",
            AgentStreamEvent::RequestTrace(_) => "RequestTrace",
            AgentStreamEvent::SessionAssigned(_) => "SessionAssigned",
        }
    }

    fn terminal_from_event(event: &AgentStreamEvent) -> RelayTerminal {
        match event {
            AgentStreamEvent::Error(data) => RelayTerminal::Error {
                code: data.code,
                retryable: data.retryable,
            },
            AgentStreamEvent::Finish(_) => RelayTerminal::Finish,
            _ => RelayTerminal::ChannelClosed,
        }
    }

    fn cancelled_finish_event() -> AgentStreamEvent {
        AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: Some(TurnStopReason::Cancelled),
        })
    }

    fn assistant_segment_persistence_error_event() -> AgentStreamEvent {
        AgentStreamEvent::Error(
            nomifun_ai_agent::protocol::events::ErrorEventData::legacy(
                "The assistant response could not be fully saved to conversation history",
                Some(AgentErrorCode::NomifunStateInconsistent),
            ),
        )
    }

    fn is_cancelled_finish(event: &AgentStreamEvent) -> bool {
        matches!(
            event,
            AgentStreamEvent::Finish(FinishEventData {
                stop_reason: Some(TurnStopReason::Cancelled),
                ..
            })
        )
    }

    /// Publish the bounded stop fallback when no backend/relay terminal was
    /// observed. The generation snapshot arbitrates the single publisher, so
    /// a late backend acknowledgement cannot duplicate the cancelled Finish.
    pub(crate) fn surface_cancelled_turn(
        &self,
        cancellation: &AgentTurnCancellation,
    ) -> bool {
        if !cancellation.try_claim_terminal_surface() {
            return false;
        }
        self.forward_to_websocket(&Self::cancelled_finish_event());
        cancellation.mark_terminal_observed();
        true
    }

    /// The canonical Conversation ID used by repository calls and events.
    fn conv_id(&self) -> &str {
        &self.conversation_id
    }

    /// Forward an agent event to connected WebSocket clients.
    #[tracing::instrument(skip_all)]
    fn forward_to_websocket(&self, event: &AgentStreamEvent) {
        self.forward_to_websocket_with_msg_id(&self.msg_id, event);
    }

    fn forward_to_websocket_hidden(&self, event: &AgentStreamEvent) {
        self.forward_to_websocket_with_msg_id_and_visibility(&self.msg_id, event, true);
    }

    #[tracing::instrument(skip_all)]
    fn forward_to_websocket_with_msg_id(&self, msg_id: &str, event: &AgentStreamEvent) {
        self.forward_to_websocket_with_msg_id_and_visibility(msg_id, event, false);
    }

    fn forward_to_websocket_with_msg_id_and_visibility(
        &self,
        msg_id: &str,
        event: &AgentStreamEvent,
        hidden: bool,
    ) {
        let mut event_data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %ErrorChain(&e), "Failed to serialize agent event for WebSocket");
                return;
            }
        };
        // Nested ACP SDK payloads serialise as camelCase on their own;
        // force every object key down the tree to snake_case so the
        // wire contract stays uniform.
        normalize_keys_to_snake_case(&mut event_data);

        let payload = json!({
            "conversation_id": self.conv_id(),
            "msg_id": msg_id,
            "type": event_data.get("type").cloned().unwrap_or(json!("unknown")),
            "data": event_data.get("data").cloned().unwrap_or(json!({})),
            "hidden": hidden,
        });

        self.broadcast_stream_payload(payload);
    }

    /// Insert a streamed assistant row, reconciling the cancellation-ambiguous case
    /// where SQLite committed the INSERT but its future returned an error (or a
    /// previous timed-out attempt was dropped before the caller observed it).
    /// We do not classify the insert error: SQLite uniqueness failures arrive as
    /// `DbError::Query`, and transport/executor errors can be ambiguous too.
    async fn insert_stream_message_with_reconciliation(
        &self,
        row: &MessageRow,
        operation: &'static str,
    ) -> bool {
        let insert_error = match self.repo.insert_message(row).await {
            Ok(()) => return true,
            Err(error) => error,
        };

        let existing = match self
            .repo
            .get_message(&row.conversation_id, &row.message_id)
            .await
        {
            Ok(Some(existing)) => existing,
            Ok(None) => {
                error!(
                    error = %ErrorChain(&insert_error),
                    operation,
                    message_id = %row.message_id,
                    "Failed to insert stream segment and no committed row was found to reconcile"
                );
                return false;
            }
            Err(reconcile_error) => {
                error!(
                    error = %ErrorChain(&insert_error),
                    reconcile_error = %ErrorChain(&reconcile_error),
                    operation,
                    message_id = %row.message_id,
                    "Failed to inspect an ambiguous stream-segment insert"
                );
                return false;
            }
        };

        // IDs are globally canonical, but still fail closed before updating an
        // existing row: a collision must never overwrite another message type
        // or turn. get_message already scopes this lookup to the conversation.
        if existing.conversation_id != row.conversation_id
            || existing.r#type != row.r#type
            || existing.msg_id != row.msg_id
        {
            error!(
                error = %ErrorChain(&insert_error),
                operation,
                message_id = %row.message_id,
                stored_type = %existing.r#type,
                expected_type = %row.r#type,
                stored_msg_id = ?existing.msg_id,
                expected_msg_id = ?row.msg_id,
                "Refusing to reconcile an ambiguous stream insert with an incompatible row"
            );
            return false;
        }

        let update = MessageRowUpdate {
            content: Some(row.content.clone()),
            status: Some(row.status.clone()),
            hidden: Some(row.hidden),
        };
        match self.repo.update_message(&row.message_id, &update).await {
            Ok(()) => {
                warn!(
                    error = %ErrorChain(&insert_error),
                    operation,
                    message_id = %row.message_id,
                    "Reconciled an ambiguous stream-segment insert against its committed row"
                );
                true
            }
            Err(reconcile_error) => {
                error!(
                    error = %ErrorChain(&insert_error),
                    reconcile_error = %ErrorChain(&reconcile_error),
                    operation,
                    message_id = %row.message_id,
                    "Failed to reconcile an ambiguous stream-segment insert"
                );
                false
            }
        }
    }

    /// Flush an active text segment to the database (create or update).
    #[tracing::instrument(skip_all)]
    async fn flush_text_segment(&self, segment: &mut TextSegmentState) {
        if segment.buffer.is_empty() {
            return;
        }

        let content = json!({
            "content": segment.buffer,
            "turn_id": &self.root_turn_id,
        })
        .to_string();

        if segment.record_created {
            let update = nomifun_db::MessageRowUpdate {
                content: Some(content),
                status: Some(Some("work".into())),
                hidden: None,
            };
            if let Err(e) = self.repo.update_message(&segment.id, &update).await {
                error!(error = %ErrorChain(&e), "Failed to update streaming text segment");
            }
        } else {
            let row = MessageRow {
                id: 0,
                message_id: segment.id.clone(),
                conversation_id: self.conversation_id.clone(),
                msg_id: Some(segment.id.clone()),
                r#type: "text".into(),
                content,
                position: Some("left".into()),
                status: Some("work".into()),
                hidden: false,
                created_at: segment.created_at,
            };
            if self
                .insert_stream_message_with_reconciliation(&row, "create_streaming_text")
                .await
            {
                segment.record_created = true;
            }
        }
    }

    #[tracing::instrument(skip_all)]
    async fn finalize_text_segment(
        &self,
        segment: &TextSegmentState,
        status: &str,
    ) -> Option<PersistedTextSegment> {
        if segment.buffer.is_empty() {
            return None;
        }

        let content = json!({
            "content": segment.buffer,
            "turn_id": &self.root_turn_id,
        })
        .to_string();
        if segment.record_created {
            let update = nomifun_db::MessageRowUpdate {
                content: Some(content),
                status: Some(Some(status.to_owned())),
                hidden: Some(false),
            };
            if let Err(e) = self.repo.update_message(&segment.id, &update).await {
                error!(error = %ErrorChain(&e), "Failed to finalize text segment");
                return None;
            }
        } else {
            let row = MessageRow {
                id: 0,
                message_id: segment.id.clone(),
                conversation_id: self.conversation_id.clone(),
                msg_id: Some(segment.id.clone()),
                r#type: "text".into(),
                content,
                position: Some("left".into()),
                status: Some(status.to_owned()),
                hidden: false,
                created_at: segment.created_at,
            };
            if !self
                .insert_stream_message_with_reconciliation(&row, "create_finalized_text")
                .await
            {
                return None;
            }
        }

        Some(PersistedTextSegment {
            id: segment.id.clone(),
        })
    }

    /// Finalize assistant text on stream end and apply middleware rewrites.
    #[tracing::instrument(skip_all)]
    async fn finalize(
        &self,
        text: &str,
        text_segments: &[PersistedTextSegment],
        text_persistence_complete: bool,
        event: &AgentStreamEvent,
        terminal: RelayTerminal,
        emitted_response: bool,
        suppress_error: bool,
        terminal_message_id: &str,
        committed_artifact_count: usize,
    ) -> RelayOutcome {
        let mut outcome = RelayOutcome {
            system_responses: Vec::new(),
            terminal,
            stop_reason: match event {
                AgentStreamEvent::Finish(data) => data.stop_reason,
                _ => None,
            },
            emitted_response,
            suppressed_error: None,
            final_text: None,
            final_text_msg_id: None,
            committed_artifact_count,
        };
        let cancelled = Self::is_cancelled_finish(event);
        let status = if matches!(event, AgentStreamEvent::Error(_)) || cancelled {
            "error"
        } else {
            "finish"
        };

        // Error is a first-class terminal record regardless of whether the
        // provider emitted partial text first. Persisting it only for empty
        // turns left the live Error frame unmatched after history hydration;
        // the renderer then carried that orphan into later turns. The error
        // message has its own canonical identity and an explicit owning turn.
        if let AgentStreamEvent::Error(data) = event
            && !suppress_error
        {
            self.persist_error_tips(terminal_message_id, data).await;
        }

        if !text.is_empty() {
            if !text_persistence_complete {
                error!(
                    conversation_id = %self.conversation_id,
                    msg_id = %self.msg_id,
                    "Assistant text terminal persistence failed after its bounded retry"
                );
                return outcome;
            }
            let processed = if cancelled {
                // A cancelled partial response is data to preserve, never a
                // completed instruction stream. In particular, do not execute
                // embedded cron commands or produce continuation responses.
                MiddlewareResult {
                    message: text.to_owned(),
                    display_message: None,
                    system_responses: Vec::new(),
                }
            } else {
                self.process_final_text(text).await
            };
            let final_text = processed.message.trim().to_owned();
            let hidden = final_text.is_empty();
            if !hidden {
                outcome.final_text = Some(final_text.clone());
            }

            if let Some(primary_segment) = text_segments.first() {
                if processed.message != text || hidden {
                    let content = json!({
                        "content": &final_text,
                        "turn_id": &self.root_turn_id,
                    })
                    .to_string();
                    let update = nomifun_db::MessageRowUpdate {
                        content: Some(content),
                        status: Some(Some(status.to_owned())),
                        hidden: Some(hidden),
                    };
                    match self.repo.update_message(&primary_segment.id, &update).await {
                        Ok(()) => {
                            self.send_final_text_override(&primary_segment.id, &final_text, hidden);

                            let mut all_superseded_hidden = true;
                            for segment in text_segments.iter().skip(1) {
                                let hide_update = nomifun_db::MessageRowUpdate {
                                    content: None,
                                    status: None,
                                    hidden: Some(true),
                                };
                                match self.repo.update_message(&segment.id, &hide_update).await {
                                    Ok(()) => self.send_final_text_override(&segment.id, "", true),
                                    Err(e) => {
                                        all_superseded_hidden = false;
                                        error!(error = %ErrorChain(&e), "Failed to hide superseded text segment");
                                    }
                                }
                            }
                            if all_superseded_hidden {
                                if !hidden {
                                    outcome.final_text_msg_id = Some(primary_segment.id.clone());
                                }
                            } else {
                                // Every emitted override now reflects an
                                // acknowledged row update, but a partial
                                // multi-row rewrite is not a coherent target
                                // for turn-final writeback.
                                outcome.final_text = None;
                            }
                        }
                        Err(e) => {
                            // The raw streamed segments are already durable.
                            // Keep the live UI on that same raw representation
                            // and do not claim that the middleware projection
                            // was persisted.
                            outcome.final_text = None;
                            error!(error = %ErrorChain(&e), "Failed to rewrite finalized text segment");
                        }
                    }
                } else {
                    outcome.final_text_msg_id = text_segments.last().map(|segment| segment.id.clone());
                    // Each segment was finalized at its own boundary. Preserve
                    // those statuses: a later provider failure belongs only to
                    // the active segment and must not rewrite earlier narration.
                }
            } else if !hidden {
                let message_id = ConversationService::mint_msg_id();
                let row = MessageRow {
                    id: 0,
                    message_id: message_id.clone(),
                    conversation_id: self.conversation_id.clone(),
                    msg_id: Some(message_id),
                    r#type: "text".into(),
                    content: json!({
                        "content": final_text,
                        "turn_id": &self.root_turn_id,
                    })
                    .to_string(),
                    position: Some("left".into()),
                    status: Some(status.to_owned()),
                    hidden: false,
                    created_at: now_ms(),
                };
                match self.repo.insert_message(&row).await {
                    Ok(()) => outcome.final_text_msg_id = Some(row.message_id.clone()),
                    Err(e) => {
                        outcome.final_text = None;
                        error!(error = %ErrorChain(&e), "Failed to create final fallback message");
                    }
                }
            }

            self.send_system_responses(&processed.system_responses);
            outcome.system_responses = processed.system_responses;
        } else if matches!(event, AgentStreamEvent::Error(_)) {
            if suppress_error {
                // review #1/#5: the send loop will (try to) fail over this
                // pre-response fault — do NOT persist the error tips row. Hand the
                // event back so the loop can re-surface it if the failover misses
                // (picker found no candidate), keeping queue-exhausted → original error.
                outcome.suppressed_error = Some(event.clone());
                return outcome;
            }
        }

        outcome
    }

    /// Persist a terminal provider error as a `tips` message row (the "no text,
    /// got error" surface). Factored out so [`Self::surface_terminal_error`] can
    /// re-persist a previously-suppressed error on a missed failover (review #1/#5).
    async fn persist_error_tips(
        &self,
        message_id: &str,
        data: &nomifun_ai_agent::protocol::events::ErrorEventData,
    ) {
        let content = json!({
            "content": &data.message,
            "type": "error",
            "error": &data,
            "turn_id": &self.root_turn_id,
        })
        .to_string();
        let row = MessageRow {
            id: 0,
            message_id: message_id.to_owned(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(message_id.to_owned()),
            r#type: "tips".into(),
            content,
            position: Some("left".into()),
            status: Some("error".into()),
            hidden: false,
            created_at: now_ms(),
        };
        if let Err(e) = self.repo.insert_message(&row).await {
            error!(error = %ErrorChain(&e), "Failed to store error message");
        }
    }

    #[tracing::instrument(skip_all)]
    async fn persist_agent_status(
        &self,
        data: &nomifun_ai_agent::protocol::events::AgentStatusEventData,
    ) -> bool {
        let id = self.agent_status_message_id().await;
        let mut content_value = serde_json::to_value(data).unwrap_or_else(|_| json!({}));
        if let Some(object) = content_value.as_object_mut() {
            object.insert("turn_id".to_owned(), json!(self.root_turn_id));
        }
        let content = content_value.to_string();
        let status = match data.status.as_str() {
            "prepared" => "finish",
            "error" => "error",
            _ => "work",
        };
        let existing = match self.repo.get_message(self.conv_id(), &id).await {
            Ok(existing) => existing,
            Err(e) => {
                error!(
                    status = %data.status,
                    error = %ErrorChain(&e),
                    "Failed to load agent_status message"
                );
                return false;
            }
        };

        if existing.is_some() {
            let update = nomifun_db::MessageRowUpdate {
                content: Some(content),
                status: Some(Some(status.to_owned())),
                hidden: Some(false),
            };
            return match self.repo.update_message(&id, &update).await {
                Ok(()) => true,
                Err(e) => {
                    error!(
                        status = %data.status,
                        error = %ErrorChain(&e),
                        "Failed to update agent_status message"
                    );
                    false
                }
            };
        }

        let row = MessageRow {
            id: 0,
            message_id: id.clone(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(self.root_turn_id.clone()),
            r#type: "agent_status".into(),
            content,
            position: Some("left".into()),
            status: Some(status.into()),
            hidden: false,
            created_at: now_ms(),
        };
        self.insert_stream_message_with_reconciliation(&row, "persist_agent_status")
            .await
    }

    async fn agent_status_message_id(&self) -> String {
        self.derived_message_id("agent_status", "model_activity").await
    }

    async fn finalize_active_agent_status(
        &self,
        active_status: &mut Option<nomifun_ai_agent::protocol::events::AgentStatusEventData>,
        terminal_status: &str,
    ) -> bool {
        let Some(current) = active_status.as_ref() else {
            return true;
        };
        let final_status = if terminal_status == "finish" {
            "prepared"
        } else {
            "error"
        };
        let should_forward = current.status != final_status;
        let mut data = current.clone();
        data.status = final_status.to_owned();

        if !self.persist_agent_status(&data).await {
            return false;
        }

        if should_forward {
            self.forward_to_websocket(&AgentStreamEvent::AgentStatus(data));
        }
        *active_status = None;
        true
    }

    fn plan_session_id(&self, data: &PlanEventData) -> String {
        data.session_id
            .as_deref()
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
            .unwrap_or(&self.root_turn_id)
            .to_owned()
    }

    async fn plan_message_id(&self, data: &PlanEventData) -> String {
        self.derived_message_id("plan", &self.plan_session_id(data)).await
    }

    #[tracing::instrument(skip_all)]
    async fn persist_plan(&self, data: &PlanEventData) {
        let plan_id = self.plan_message_id(data).await;
        let session_id = self.plan_session_id(data);
        let status = if data.entries.iter().all(|entry| {
            entry.get("status").and_then(serde_json::Value::as_str) == Some("completed")
        }) {
            "finish"
        } else {
            "work"
        };
        let content = json!({
            "session_id": session_id,
            "entries": data.entries,
        })
        .to_string();

        let existing = self
            .repo
            .get_message_by_msg_id(self.conv_id(), &plan_id, "plan")
            .await
            .unwrap_or(None);

        if existing.is_some() {
            let update = nomifun_db::MessageRowUpdate {
                content: Some(content),
                status: Some(Some(status.to_owned())),
                hidden: Some(false),
            };
            if let Err(e) = self.repo.update_message(&plan_id, &update).await {
                error!(error = %ErrorChain(&e), "Failed to update plan message");
            }
            return;
        }

        let row = MessageRow {
            id: 0,
            message_id: plan_id.clone(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(plan_id),
            r#type: "plan".into(),
            content,
            position: Some("left".into()),
            status: Some(status.to_owned()),
            hidden: false,
            created_at: now_ms(),
        };
        if let Err(e) = self.repo.insert_message(&row).await {
            error!(error = %ErrorChain(&e), "Failed to persist plan message");
        }
    }

    #[tracing::instrument(skip_all)]
    async fn complete_active_thinking(
        &self,
        active_thinking: &mut Option<ThinkingSegmentState>,
    ) -> bool {
        let Some(segment) = active_thinking.as_mut() else {
            return true;
        };

        let duration_ms = match segment.completed_duration_ms {
            Some(duration_ms) => duration_ms,
            None => {
                let duration_ms = (now_ms() - segment.started_at).max(0) as u64;
                segment.completed_duration_ms = Some(duration_ms);
                self.send_thinking_done(&segment.id, duration_ms);
                duration_ms
            }
        };
        if segment.buffer.is_empty() {
            *active_thinking = None;
            return true;
        }

        let row = MessageRow {
            id: 0,
            message_id: segment.id.clone(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(segment.id.clone()),
            r#type: "thinking".into(),
            content: json!({
                "content": segment.buffer,
                "status": "done",
                "duration_ms": duration_ms,
                "turn_id": &self.root_turn_id,
            })
            .to_string(),
            position: Some("left".into()),
            status: Some("finish".into()),
            hidden: false,
            created_at: segment.started_at,
        };
        let persisted = self
            .insert_stream_message_with_reconciliation(&row, "complete_thinking")
            .await;
        if persisted {
            *active_thinking = None;
        }
        persisted
    }

    /// Retry a terminal thinking write once. The state remains owned by
    /// `active_thinking` until the repository acknowledges it, so cancellation
    /// of either attempt cannot discard the only durable-retry copy.
    async fn retry_terminal_thinking_segment(
        &self,
        active_thinking: &mut Option<ThinkingSegmentState>,
    ) -> bool {
        if active_thinking.is_some() {
            warn!(
                conversation_id = %self.conversation_id,
                msg_id = %self.msg_id,
                "Retrying assistant thinking terminal persistence"
            );
            self.complete_active_thinking(active_thinking).await
        } else {
            true
        }
    }

    /// Release whatever the robot stage-direction filter is still withholding, as
    /// literal text, into the live stream, the active segment's buffer and the
    /// turn's full text — exactly the three sinks the `Text` arm feeds.
    ///
    /// Call this at every loop exit that can reach `close_active_text_segment`
    /// without going through the `Text` arm first, otherwise a truncated `[` at
    /// the end of a text run is silently deleted. Today that is two sites: the
    /// non-`Text` branch of the `Ok(mut event)` rewrite (which covers all six
    /// in-loop `close_active_text_segment` calls reachable from a received event,
    /// plus both `break outcome` exits) and the first statement of the
    /// `RecvError::Closed` arm.
    ///
    /// WARNING: a third loop exit — or a `close_active_text_segment` call added
    /// upstream of the rewrite — breaks this silently, with no test failure and
    /// no log line. `robot_session_releases_truncated_bracket_before_tool_call`
    /// pins the known paths only.
    fn release_withheld_text(
        &self,
        filter: &mut StageDirectionFilter,
        active_text: &mut Option<TextSegmentState>,
        full_text_buffer: &mut String,
    ) {
        let released = filter.flush();
        if released.is_empty() {
            return;
        }
        // No active segment means no text run was open, so there is nowhere the
        // bytes belong and no bubble they could have been part of.
        if let Some(segment) = active_text.as_mut() {
            self.forward_to_websocket_with_msg_id(
                &segment.id,
                &AgentStreamEvent::Text(TextEventData {
                    content: released.clone(),
                }),
            );
            segment.buffer.push_str(&released);
            full_text_buffer.push_str(&released);
        }
    }

    #[tracing::instrument(skip_all)]
    async fn close_active_text_segment(
        &self,
        active_text: &mut Option<TextSegmentState>,
        text_segments: &mut Vec<PersistedTextSegment>,
        status: &str,
    ) {
        if active_text
            .as_ref()
            .is_some_and(|segment| segment.buffer.is_empty())
        {
            *active_text = None;
            return;
        }

        // Keep the in-memory segment authoritative until the repository has
        // acknowledged the terminal write. This future is deliberately used
        // behind the non-terminal side-effect timeout: taking the segment
        // before the await would drop its only retryable copy when that timeout
        // cancels the future, leaving the later terminal cleanup with nothing
        // to persist.
        let persisted = {
            let Some(text_segment) = active_text.as_ref() else {
                return;
            };
            self.finalize_text_segment(text_segment, status).await
        };
        let Some(segment) = persisted else {
            return;
        };

        *active_text = None;
        if text_segments.len() < MAX_TERMINAL_ACTIVE_ITEMS {
            text_segments.push(segment);
        } else {
            warn!(
                max = MAX_TERMINAL_ACTIVE_ITEMS,
                "Relay finalized-text tracking limit reached"
            );
        }
    }

    /// Retry a terminal text write once after the first close attempt failed.
    /// The enclosing terminal cleanup already owns the global hard deadline, so
    /// this adds recovery for transient SQLite errors without an unbounded loop.
    async fn retry_terminal_text_segment(
        &self,
        active_text: &mut Option<TextSegmentState>,
        text_segments: &mut Vec<PersistedTextSegment>,
        status: &str,
    ) -> bool {
        if active_text.is_some() {
            warn!(
                conversation_id = %self.conversation_id,
                msg_id = %self.msg_id,
                "Retrying assistant text terminal persistence"
            );
            self.close_active_text_segment(active_text, text_segments, status)
                .await;
        }
        active_text.is_none()
    }

    /// Persist a Gemini-style tool_call event.
    #[tracing::instrument(skip_all)]
    async fn persist_tool_call(&self, data: &nomifun_ai_agent::protocol::events::tool_call::ToolCallEventData) {
        self.persist_tool_call_with_hidden(data, false).await;
    }

    async fn persist_provisional_artifact_tool_call(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::ToolCallEventData,
    ) -> bool {
        let provisional = Self::provisional_artifact_tool_call(data);
        self.persist_tool_call_projection(&provisional, false, Some(false))
            .await
    }

    fn provisional_artifact_tool_call(data: &ToolCallEventData) -> ToolCallEventData {
        let mut provisional = data.clone();
        provisional.status = ToolCallStatus::Running;
        provisional.artifacts.clear();
        provisional.output = Some(ARTIFACT_DELIVERY_PENDING_OUTPUT.to_owned());
        provisional
    }

    async fn persist_tool_call_with_hidden(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::ToolCallEventData,
        hidden: bool,
    ) {
        let _ = self.persist_tool_call_projection(data, hidden, None).await;
    }

    async fn persist_tool_call_projection(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::ToolCallEventData,
        hidden: bool,
        artifact_delivery_committed: Option<bool>,
    ) -> bool {
        if data.call_id.trim().is_empty() {
            warn!(
                tool = %data.name,
                status = ?data.status,
                "Skipping tool_call persistence because call_id is empty"
            );
            return false;
        }

        let status = match data.status {
            ToolCallStatus::Running => "work",
            ToolCallStatus::Completed => "finish",
            ToolCallStatus::Error => "error",
        };
        let message_id = self.tool_message_id(&data.call_id).await;
        let mut content_value = serde_json::to_value(data).unwrap_or_default();
        if let Some(object) = content_value.as_object_mut() {
            object.insert("turn_id".to_owned(), json!(self.root_turn_id));
            if let Some(committed) = artifact_delivery_committed {
                object.insert(ARTIFACT_DELIVERY_COMMITTED_FIELD.to_owned(), json!(committed));
            }
            if data.status != ToolCallStatus::Completed {
                // Artifact receipts are a terminal-success contract. Force an
                // explicit empty array (the wire serializer normally skips an
                // empty Vec) so merging an Error over a malformed Running row
                // cannot retain provisional/stale receipts.
                object.insert("artifacts".to_owned(), json!([]));
            }
        }
        let content = content_value.to_string();

        let existing = match self.repo.get_message(self.conv_id(), &message_id).await {
            Ok(existing) => existing,
            Err(e) => {
                error!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    status,
                    error = %ErrorChain(&e),
                    "Failed to load tool_call message before persistence"
                );
                return false;
            }
        };

        if let Some(existing_row) = existing {
            let existing_artifact_committed = serde_json::from_str::<Value>(&existing_row.content)
                .ok()
                .and_then(|value| {
                    value
                        .get(ARTIFACT_DELIVERY_COMMITTED_FIELD)
                        .and_then(Value::as_bool)
                })
                == Some(true);
            let terminal_conflict = match (existing_row.status.as_deref(), data.status) {
                (Some("finish"), ToolCallStatus::Completed | ToolCallStatus::Error)
                | (Some("error"), ToolCallStatus::Error) => false,
                // A newly verified artifact completion always starts a fresh
                // provisional projection. It may safely demote an uncommitted
                // or legacy finish row; an existing error remains absorbing.
                (Some("finish"), _)
                    if artifact_delivery_committed == Some(false)
                        && !existing_artifact_committed =>
                {
                    false
                }
                (Some("finish" | "error"), _) => true,
                _ => false,
            };
            if terminal_conflict {
                warn!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    stored_status = ?existing_row.status,
                    incoming_status = ?data.status,
                    "Ignoring tool call transition away from persisted terminal state"
                );
                return false;
            }
            let merged_content = Self::merge_json_content(&existing_row.content, &content);
            let update = nomifun_db::MessageRowUpdate {
                content: Some(merged_content),
                status: Some(Some(status.to_owned())),
                hidden: hidden.then_some(true),
            };
            if let Err(e) = self.repo.update_message(&message_id, &update).await {
                error!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    status,
                    error = %ErrorChain(&e),
                    "Failed to update tool_call message"
                );
                return false;
            } else {
                debug!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    status,
                    "Updated tool_call message"
                );
            }
        } else {
            let row = MessageRow {
                id: 0,
                message_id: message_id.clone(),
                conversation_id: self.conversation_id.clone(),
                msg_id: Some(self.root_turn_id.clone()),
                r#type: "tool_call".into(),
                content,
                position: Some("left".into()),
                status: Some(status.to_owned()),
                hidden,
                created_at: now_ms(),
            };
            if let Err(e) = self.repo.insert_message(&row).await {
                error!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    status,
                    error = %ErrorChain(&e),
                    "Failed to persist tool_call message"
                );
                return false;
            } else {
                debug!(
                    call_id = %data.call_id,
                    tool = %data.name,
                    status,
                    "Persisted tool_call message"
                );
            }
        }
        true
    }

    async fn tool_message_id(&self, call_id: &str) -> String {
        self.derived_message_id("tool_call", call_id).await
    }

    fn incomplete_tool_reason(event: &AgentStreamEvent) -> Option<&'static str> {
        match event {
            AgentStreamEvent::Error(_) => Some("error"),
            AgentStreamEvent::Finish(data) => match data.stop_reason {
                Some(nomifun_ai_agent::protocol::events::TurnStopReason::MaxTokens) => Some("max_tokens"),
                Some(nomifun_ai_agent::protocol::events::TurnStopReason::MaxTurnRequests) => {
                    Some("max_turn_requests")
                }
                Some(nomifun_ai_agent::protocol::events::TurnStopReason::Refusal) => Some("refusal"),
                Some(nomifun_ai_agent::protocol::events::TurnStopReason::Cancelled) => Some("cancelled"),
                Some(nomifun_ai_agent::protocol::events::TurnStopReason::EndTurn) => Some("end_turn"),
                None => Some("finish"),
            },
            _ => None,
        }
    }

    fn invalidates_completed_artifacts(event: &AgentStreamEvent) -> bool {
        match event {
            AgentStreamEvent::Error(_) => true,
            AgentStreamEvent::Finish(data) => !matches!(
                data.stop_reason,
                None | Some(nomifun_ai_agent::protocol::events::TurnStopReason::EndTurn)
            ),
            _ => false,
        }
    }

    fn committed_artifact_tool_content(
        &self,
        data: &ToolCallEventData,
    ) -> Result<String, nomifun_db::DbError> {
        if data.status != ToolCallStatus::Completed || data.artifacts.is_empty() {
            return Err(nomifun_db::DbError::Conflict(format!(
                "tool call '{}' is not a completed artifact delivery",
                data.call_id
            )));
        }
        let mut value = serde_json::to_value(data)
            .map_err(|error| nomifun_db::DbError::Conflict(error.to_string()))?;
        let object = value.as_object_mut().ok_or_else(|| {
            nomifun_db::DbError::Conflict(format!(
                "tool call '{}' did not serialize as an object",
                data.call_id
            ))
        })?;
        object.insert("turn_id".to_owned(), json!(self.root_turn_id));
        object.insert(ARTIFACT_DELIVERY_COMMITTED_FIELD.to_owned(), json!(true));
        Ok(value.to_string())
    }

    fn committed_artifact_acp_tool_content(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
    ) -> Result<String, nomifun_db::DbError> {
        let has_delivery = data.update.content.as_ref().is_some_and(|items| {
            items.iter().any(|item| {
                matches!(
                    item,
                    nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                        | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                )
            })
        });
        if data.update.status != Some(AcpToolCallStatus::Completed) || !has_delivery {
            return Err(nomifun_db::DbError::Conflict(format!(
                "ACP tool call '{}' is not a completed artifact delivery",
                data.update.tool_call_id
            )));
        }
        let mut value = serde_json::to_value(data)
            .map_err(|error| nomifun_db::DbError::Conflict(error.to_string()))?;
        normalize_keys_to_snake_case(&mut value);
        let object = value.as_object_mut().ok_or_else(|| {
            nomifun_db::DbError::Conflict(format!(
                "ACP tool call '{}' did not serialize as an object",
                data.update.tool_call_id
            ))
        })?;
        object.insert("turn_id".to_owned(), json!(self.root_turn_id));
        object.insert(ARTIFACT_DELIVERY_COMMITTED_FIELD.to_owned(), json!(true));
        Ok(value.to_string())
    }

    async fn claim_generic_artifact_recovery(
        &self,
        data: &ToolCallEventData,
    ) -> Result<(), DbError> {
        if data.artifacts.is_empty() {
            return Ok(());
        }
        if self.allows_legacy_unjournaled_artifacts() && self.artifact_workspace.is_none() {
            // Explicit test-only fixtures exercise relay tracking/correction
            // semantics with synthetic receipts that have no real workspace.
            // Production can never enable this branch.
            return Ok(());
        }
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return Err(DbError::Conflict(
                "artifact recovery has no canonical session workspace".to_owned(),
            ));
        };
        let store = ArtifactStore::new(workspace);
        let records = store
            .recovery_records()
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        let journaled = records
            .iter()
            .map(|record| record.receipt.id.as_str())
            .collect::<HashSet<_>>();
        let matching_records = records
            .iter()
            .filter(|record| {
                data.artifacts
                    .iter()
                    .any(|artifact| artifact.id == record.receipt.id)
            })
            .collect::<Vec<_>>();
        let matching = data
            .artifacts
            .iter()
            .filter(|artifact| journaled.contains(artifact.id.as_str()))
            .count();
        // Legacy/ACP fixtures may carry verified receipts created before the
        // recoverable sink path. Production Nomi deferred images are all-or-
        // nothing journaled; a partial journal set is a hard ownership error.
        if matching == 0 {
            if self.allows_legacy_unjournaled_artifacts() {
                return Ok(());
            }
            return Err(DbError::Conflict(format!(
                "tool call '{}' has no durable artifact recovery journal",
                data.call_id
            )));
        }
        if matching != data.artifacts.len() {
            return Err(DbError::Conflict(format!(
                "tool call '{}' has an incomplete artifact recovery batch",
                data.call_id
            )));
        }
        let prepared_for_this_wire = matching_records.len() == data.artifacts.len()
            && matching_records.iter().all(|record| {
                matches!(
                    &record.state,
                    ArtifactRecoveryState::Prepared { envelope }
                        if envelope.conversation_id == self.conv_id()
                            && envelope.wire_msg_id == self.msg_id
                )
            });
        let message_id = match self.try_derived_message_id("tool_call", &data.call_id).await {
            Ok(message_id) => message_id,
            Err(error) => {
                if matching == 0 || prepared_for_this_wire {
                    let _ = store.rollback_owned_receipts(&data.artifacts);
                }
                return Err(error);
            }
        };
        let owner = ArtifactRecoveryOwner {
            conversation_id: self.conv_id().to_owned(),
            wire_msg_id: self.msg_id.clone(),
            root_turn_id: self.root_turn_id.clone(),
            message_id,
            message_type: "tool_call".to_owned(),
            committed_content: self.committed_artifact_tool_content(data)?,
        };
        if let Err(error) = store.claim_recovery_receipts(&data.artifacts, &owner) {
            // Every matching record was still Prepared before this call, so
            // even a partial journal replacement cannot race a provisional DB
            // write. Roll back the exact batch rather than leaking a claimed
            // owner that the relay rejected.
            if matching == 0 || prepared_for_this_wire {
                let _ = store.rollback_owned_receipts(&data.artifacts);
            }
            return Err(DbError::Conflict(error.to_string()));
        }
        Ok(())
    }

    async fn claim_acp_artifact_recovery(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
    ) -> Result<(), DbError> {
        let artifacts = Self::acp_artifact_receipts(data);
        if artifacts.is_empty() {
            return Ok(());
        }
        if self.allows_legacy_unjournaled_artifacts() && self.artifact_workspace.is_none() {
            return self
                .try_derived_message_id("acp_tool_call", &data.update.tool_call_id)
                .await
                .map(|_| ());
        }
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return Err(DbError::Conflict(
                "ACP artifact recovery has no canonical session workspace".to_owned(),
            ));
        };
        let store = ArtifactStore::new(workspace);
        let records = store
            .recovery_records()
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        let matching_records = records
            .iter()
            .filter(|record| artifacts.iter().any(|artifact| artifact.id == record.receipt.id))
            .collect::<Vec<_>>();
        if !matching_records.is_empty() && matching_records.len() != artifacts.len() {
            return Err(DbError::Conflict(format!(
                "ACP tool call '{}' has an incomplete artifact recovery batch",
                data.update.tool_call_id
            )));
        }
        if matching_records.is_empty() {
            if self.allows_legacy_unjournaled_artifacts() {
                return self
                    .try_derived_message_id("acp_tool_call", &data.update.tool_call_id)
                    .await
                    .map(|_| ());
            }
            return Err(DbError::Conflict(format!(
                "ACP tool call '{}' has no durable artifact recovery journal",
                data.update.tool_call_id
            )));
        }
        let prepared_for_this_wire = matching_records.len() == artifacts.len()
            && matching_records.iter().all(|record| {
                matches!(
                    &record.state,
                    ArtifactRecoveryState::Prepared { envelope }
                        if envelope.conversation_id == self.conv_id()
                            && envelope.wire_msg_id == self.msg_id
                )
            });
        let message_id = match self
            .try_derived_message_id("acp_tool_call", &data.update.tool_call_id)
            .await
        {
            Ok(message_id) => message_id,
            Err(error) => {
                if matching_records.is_empty() || prepared_for_this_wire {
                    let _ = store.rollback_owned_receipts(&artifacts);
                }
                return Err(error);
            }
        };
        let owner = ArtifactRecoveryOwner {
            conversation_id: self.conv_id().to_owned(),
            wire_msg_id: self.msg_id.clone(),
            root_turn_id: self.root_turn_id.clone(),
            message_id,
            message_type: "acp_tool_call".to_owned(),
            committed_content: self.committed_artifact_acp_tool_content(data)?,
        };
        let terminal_envelope = ArtifactRecoveryEnvelope {
            conversation_id: self.conv_id().to_owned(),
            wire_msg_id: self.msg_id.clone(),
            event_kind: "acp_tool_call".to_owned(),
            event_json: serde_json::to_string(data)
                .map_err(|error| DbError::Conflict(error.to_string()))?,
        };
        if let Err(error) = store.claim_recovery_receipts_with_envelope(
            &artifacts,
            &owner,
            Some(&terminal_envelope),
        ) {
            if matching_records.is_empty() || prepared_for_this_wire {
                let _ = store.rollback_owned_receipts(&artifacts);
            }
            return Err(DbError::Conflict(error.to_string()));
        }
        Ok(())
    }

    fn acp_artifact_receipts(
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
    ) -> Vec<PersistedArtifact> {
        data.update
            .content
            .iter()
            .flatten()
            .filter_map(|item| match item {
                nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                    artifact,
                    ..
                } => Some(artifact.clone()),
                _ => None,
            })
            .collect()
    }

    /// Recover a receipt-bearing terminal event skipped by broadcast lag. The
    /// sink journals the complete event before `send`; matching this relay's
    /// exact conversation+wire makes reconstruction deterministic.
    async fn merge_prepared_generic_artifact_recoveries(
        &self,
        generic: &mut HashMap<String, ToolCallEventData>,
    ) -> Result<(), DbError> {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return Ok(());
        };
        let records = ArtifactStore::new(workspace)
            .recovery_records()
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        let mut recovered = HashMap::<String, ToolCallEventData>::new();
        for record in records {
            let envelope = match record.state {
                ArtifactRecoveryState::Prepared { envelope }
                | ArtifactRecoveryState::ClaimedActive { envelope, .. }
                | ArtifactRecoveryState::CommitAttempting { envelope, .. }
                | ArtifactRecoveryState::NeedsReconcile { envelope, .. } => envelope,
                ArtifactRecoveryState::Unprepared
                | ArtifactRecoveryState::PersistedUnprepared => continue,
            };
            if envelope.conversation_id != self.conv_id()
                || envelope.wire_msg_id != self.msg_id
                || envelope.event_kind != "tool_call"
            {
                continue;
            }
            let data: ToolCallEventData = serde_json::from_str(&envelope.event_json)
                .map_err(|error| DbError::Conflict(format!("invalid artifact recovery event: {error}")))?;
            if data.status != ToolCallStatus::Completed
                || !data
                    .artifacts
                    .iter()
                    .any(|artifact| artifact == &record.receipt)
            {
                return Err(DbError::Conflict(format!(
                    "artifact recovery record '{}' does not match its terminal event",
                    record.receipt.id
                )));
            }
            recovered.entry(data.call_id.clone()).or_insert(data);
        }
        for (call_id, data) in recovered {
            validate_completed_artifact_contract(&data)
                .map_err(DbError::Conflict)?;
            self.claim_generic_artifact_recovery(&data).await?;
            if !track_bounded(generic, call_id, data, "recovered_artifact_tool_call") {
                return Err(DbError::Conflict(
                    "artifact recovery exceeded the terminal tracking limit".to_owned(),
                ));
            }
        }
        Ok(())
    }

    async fn merge_prepared_acp_artifact_recoveries(
        &self,
        acp: &mut HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) -> Result<(), DbError> {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return Ok(());
        };
        let records = ArtifactStore::new(workspace)
            .recovery_records()
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        let mut recovered = HashMap::new();
        for record in records {
            let envelope = match record.state {
                ArtifactRecoveryState::Prepared { envelope }
                | ArtifactRecoveryState::ClaimedActive { envelope, .. }
                | ArtifactRecoveryState::CommitAttempting { envelope, .. }
                | ArtifactRecoveryState::NeedsReconcile { envelope, .. } => envelope,
                ArtifactRecoveryState::Unprepared
                | ArtifactRecoveryState::PersistedUnprepared => continue,
            };
            if envelope.conversation_id != self.conv_id()
                || envelope.wire_msg_id != self.msg_id
                || envelope.event_kind != "acp_tool_call"
            {
                continue;
            }
            let data: nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData =
                serde_json::from_str(&envelope.event_json).map_err(|error| {
                    DbError::Conflict(format!("invalid ACP artifact recovery event: {error}"))
                })?;
            if data.update.status != Some(AcpToolCallStatus::Completed)
                || !Self::acp_artifact_receipts(&data)
                    .iter()
                    .any(|artifact| artifact == &record.receipt)
            {
                return Err(DbError::Conflict(format!(
                    "ACP artifact recovery record '{}' does not match its terminal event",
                    record.receipt.id
                )));
            }
            recovered
                .entry(data.update.tool_call_id.clone())
                .or_insert(data);
        }
        for (tool_call_id, data) in recovered {
            validate_completed_acp_artifact_contract(&data).map_err(DbError::Conflict)?;
            self.claim_acp_artifact_recovery(&data).await?;
            if !track_bounded(acp, tool_call_id, data, "recovered_artifact_acp_tool_call") {
                return Err(DbError::Conflict(
                    "ACP artifact recovery exceeded the terminal tracking limit".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn finalize_generic_artifact_recovery(
        &self,
        generic: &HashMap<String, ToolCallEventData>,
    ) {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return;
        };
        let receipts = generic
            .values()
            .flat_map(|data| data.artifacts.iter().cloned())
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return;
        }
        if let Err(error) = ArtifactStore::new(workspace).finalize_recovery_receipts(&receipts) {
            error!(
                error = %error,
                receipt_count = receipts.len(),
                "Durable artifact rows committed but recovery journal finalization failed"
            );
            let _ = ArtifactStore::new(workspace)
                .mark_recovery_receipts_needs_reconcile(&receipts);
        }
    }

    fn finalize_acp_artifact_recovery(
        &self,
        acp: &HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return;
        };
        let receipts = acp
            .values()
            .flat_map(Self::acp_artifact_receipts)
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return;
        }
        let store = ArtifactStore::new(workspace);
        if let Err(error) = store.finalize_recovery_receipts(&receipts) {
            error!(error = %error, "Durable ACP artifact rows committed but journal finalization failed");
            let _ = store.mark_recovery_receipts_needs_reconcile(&receipts);
        }
    }

    fn mark_generic_artifact_recovery_needs_reconcile(
        &self,
        generic: &HashMap<String, ToolCallEventData>,
    ) {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return;
        };
        let receipts = generic
            .values()
            .flat_map(|data| data.artifacts.iter().cloned())
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return;
        }
        if let Err(error) =
            ArtifactStore::new(workspace).mark_recovery_receipts_needs_reconcile(&receipts)
        {
            error!(
                error = %error,
                receipt_count = receipts.len(),
                "Failed to persist indeterminate artifact recovery ownership"
            );
        }
    }

    fn mark_acp_artifact_recovery_needs_reconcile(
        &self,
        acp: &HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return;
        };
        let receipts = acp
            .values()
            .flat_map(Self::acp_artifact_receipts)
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return;
        }
        if let Err(error) = ArtifactStore::new(workspace)
            .mark_recovery_receipts_needs_reconcile(&receipts)
        {
            error!(error = %error, "Failed to persist indeterminate ACP artifact recovery ownership");
        }
    }

    /// Reconcile crash-visible recovery owners before consuming a new stream.
    /// Cross-process takeover is authorized only by the exact OS-backed
    /// receipt lease; wall-clock age and boot ids are never deletion proof.
    async fn reconcile_pending_artifact_recovery_journal(&self) {
        let Some(workspace) = self.artifact_workspace.as_ref() else {
            return;
        };
        let store = ArtifactStore::new(workspace);
        let records = match store.recovery_records() {
            Ok(records) => records,
            Err(error) => {
                error!(error = %error, "Could not read artifact recovery journal");
                return;
            }
        };
        let mut event_groups = HashMap::<(String, String), Vec<_>>::new();
        for record in records {
            if record.source.conversation_id != self.conv_id() {
                continue;
            }
            let event_key = match &record.state {
                ArtifactRecoveryState::Unprepared
                | ArtifactRecoveryState::PersistedUnprepared => {
                    format!("unprepared:{}", record.receipt.id)
                }
                ArtifactRecoveryState::Prepared { envelope }
                | ArtifactRecoveryState::ClaimedActive { envelope, .. }
                | ArtifactRecoveryState::CommitAttempting { envelope, .. }
                | ArtifactRecoveryState::NeedsReconcile { envelope, .. } => {
                    envelope.event_json.clone()
                }
            };
            event_groups
                .entry((record.source.wire_msg_id.clone(), event_key))
                .or_default()
                .push(record);
        }
        let mut owner_groups = HashMap::<(String, String), Vec<_>>::new();
        for ((wire_msg_id, _), records) in event_groups {
            let owners = records
                .iter()
                .filter_map(|record| match &record.state {
                    ArtifactRecoveryState::ClaimedActive { owner, .. }
                    | ArtifactRecoveryState::CommitAttempting { owner, .. }
                    | ArtifactRecoveryState::NeedsReconcile { owner, .. } => Some(owner.clone()),
                    ArtifactRecoveryState::Unprepared
                    | ArtifactRecoveryState::PersistedUnprepared
                    | ArtifactRecoveryState::Prepared { .. } => None,
                })
                .collect::<Vec<_>>();
            // A Prepared event for this exact live wire belongs to the running
            // relay and is reconstructed at its terminal barrier, not startup.
            if wire_msg_id == self.msg_id
                && owners.is_empty()
                && records.iter().all(|record| record.produced_by_current_boot())
            {
                continue;
            }
            let mut leases_owned = true;
            let mut newly_acquired = Vec::new();
            for record in &records {
                match store.try_acquire_recovery_lease(record) {
                    Ok(true) => newly_acquired.push(record.clone()),
                    Ok(false) => {
                        leases_owned = false;
                        break;
                    }
                    Err(error) => {
                        error!(
                            error = %error,
                            artifact_id = record.receipt.id,
                            "Could not acquire abandoned artifact recovery lease"
                        );
                        leases_owned = false;
                        break;
                    }
                }
            }
            if !leases_owned {
                for record in &newly_acquired {
                    let _ = store.release_acquired_recovery_lease(record);
                }
                continue;
            }
            let receipts = records
                .iter()
                .map(|record| record.receipt.clone())
                .collect::<Vec<_>>();
            if owners.is_empty() {
                if let Err(error) = store.rollback_owned_receipts(&receipts) {
                    error!(error = %error, "Failed to roll back abandoned pre-commit artifacts");
                    Self::release_recovery_record_leases(&store, &records);
                }
                continue;
            }
            let owner = owners[0].clone();
            if owners.iter().any(|candidate| candidate != &owner) {
                error!(wire_msg_id, "Conflicting artifact recovery owners retained for manual reconciliation");
                Self::release_recovery_record_leases(&store, &records);
                continue;
            }
            // Complete any partial journal claim before relinquishing the
            // event to the replay owner.
            if let Err(error) = store.claim_recovery_receipts(&receipts, &owner) {
                error!(error = %error, wire_msg_id, "Could not complete partial artifact recovery claim");
                Self::release_recovery_record_leases(&store, &records);
                continue;
            }
            owner_groups
                .entry((owner.conversation_id.clone(), owner.root_turn_id.clone()))
                .or_default()
                .extend(records);
        }

        for ((conversation_id, root_turn_id), records) in owner_groups {
            let mut owners = HashMap::<String, ArtifactRecoveryOwner>::new();
            let mut owner_conflict = false;
            for record in &records {
                let owner = match &record.state {
                    ArtifactRecoveryState::ClaimedActive { owner, .. }
                    | ArtifactRecoveryState::CommitAttempting { owner, .. }
                    | ArtifactRecoveryState::NeedsReconcile { owner, .. } => owner,
                    _ => continue,
                };
                if owners
                    .insert(owner.message_id.clone(), owner.clone())
                    .is_some_and(|existing| existing != *owner)
                {
                    owner_conflict = true;
                }
            }
            if owner_conflict || owners.is_empty() {
                error!(conversation_id, root_turn_id, "Conflicting artifact recovery commit identities retained");
                Self::release_recovery_record_leases(&store, &records);
                continue;
            }
            let mut commits = owners
                .values()
                .map(|owner| TurnArtifactMessageCommit {
                    message_id: owner.message_id.clone(),
                    message_type: owner.message_type.clone(),
                    content: owner.committed_content.clone(),
                })
                .collect::<Vec<_>>();
            commits.sort_by(|left, right| left.message_id.cmp(&right.message_id));
            let receipts = records
                .iter()
                .map(|record| record.receipt.clone())
                .collect::<Vec<_>>();
            // CommitAttempting is a batch fence, not a per-event permission.
            // If any call in this root turn remained pre-commit, the database
            // transaction was never entered and the whole batch rolls back.
            if records.iter().any(|record| {
                matches!(
                    record.state,
                    ArtifactRecoveryState::Unprepared
                        | ArtifactRecoveryState::PersistedUnprepared
                        | ArtifactRecoveryState::Prepared { .. }
                        | ArtifactRecoveryState::ClaimedActive { .. }
                )
            }) {
                if let Err(error) = store.rollback_owned_receipts(&receipts) {
                    error!(error = %error, conversation_id, root_turn_id, "Failed to roll back incomplete artifact commit fence");
                    Self::release_recovery_record_leases(&store, &records);
                }
                continue;
            }
            let _ = store.mark_recovery_receipts_needs_reconcile(&receipts);
            let initial = self
                .reconcile_recovery_artifact_commits(
                    &conversation_id,
                    &root_turn_id,
                    &commits,
                )
                .await;
            if initial == ArtifactCommitReconciliation::AllDurable {
                if receipts.iter().all(|receipt| store.reverify_receipt(receipt).is_ok()) {
                    if let Err(error) = store.finalize_recovery_receipts(&receipts) {
                        error!(error = %error, "Failed to finalize recovered artifact journals");
                        Self::release_recovery_record_leases(&store, &records);
                    }
                } else {
                    error!(conversation_id, root_turn_id, "Durable artifact rows reference unverifiable bytes; retaining recovery journal");
                    Self::release_recovery_record_leases(&store, &records);
                }
                continue;
            }
            if !receipts.iter().all(|receipt| store.reverify_receipt(receipt).is_ok()) {
                if initial == ArtifactCommitReconciliation::DefinitelyNotCommitted {
                    if store.rollback_owned_receipts(&receipts).is_err() {
                        Self::release_recovery_record_leases(&store, &records);
                    }
                } else {
                    Self::release_recovery_record_leases(&store, &records);
                }
                continue;
            }

            let replay = self
                .repo
                .commit_turn_artifact_messages(
                    &conversation_id,
                    &root_turn_id,
                    &commits,
                    now_ms(),
                )
                .await;
            if replay.as_ref().is_ok_and(|rows| {
                Self::returned_artifact_batch_is_exact(
                    rows,
                    &commits,
                    &conversation_id,
                    &root_turn_id,
                )
            }) {
                if let Err(error) = store.finalize_recovery_receipts(&receipts) {
                    error!(error = %error, "Failed to finalize replayed artifact journals");
                    Self::release_recovery_record_leases(&store, &records);
                }
                continue;
            }
            let after_replay = self
                .reconcile_recovery_artifact_commits(
                    &conversation_id,
                    &root_turn_id,
                    &commits,
                )
                .await;
            match after_replay {
                ArtifactCommitReconciliation::AllDurable => {
                    if receipts.iter().all(|receipt| store.reverify_receipt(receipt).is_ok()) {
                        if store.finalize_recovery_receipts(&receipts).is_err() {
                            Self::release_recovery_record_leases(&store, &records);
                        }
                    } else {
                        Self::release_recovery_record_leases(&store, &records);
                    }
                }
                ArtifactCommitReconciliation::DefinitelyNotCommitted => {
                    if store.rollback_owned_receipts(&receipts).is_err() {
                        Self::release_recovery_record_leases(&store, &records);
                    }
                }
                ArtifactCommitReconciliation::Indeterminate => {
                    warn!(conversation_id, root_turn_id, "Artifact recovery replay remains indeterminate; retaining journal");
                    let _ = store.mark_recovery_receipts_needs_reconcile(&receipts);
                    Self::release_recovery_record_leases(&store, &records);
                }
            }
        }
    }

    fn release_recovery_record_leases(
        store: &ArtifactStore,
        records: &[nomifun_ai_agent::artifact_store::ArtifactRecoveryRecord],
    ) {
        for record in records {
            if let Err(error) = store.release_acquired_recovery_lease(record) {
                warn!(
                    error = %error,
                    artifact_id = record.receipt.id,
                    "Could not relinquish retained artifact recovery lease"
                );
            }
        }
    }

    async fn reconcile_recovery_artifact_commits(
        &self,
        conversation_id: &str,
        root_turn_id: &str,
        commits: &[TurnArtifactMessageCommit],
    ) -> ArtifactCommitReconciliation {
        let mut durable = 0usize;
        let mut definitely_uncommitted = true;
        for commit in commits {
            match self.repo.get_message(conversation_id, &commit.message_id).await {
                Ok(Some(row))
                    if Self::finished_artifact_row_matches(
                        &row,
                        commit,
                        conversation_id,
                        root_turn_id,
                    ) => durable += 1,
                Ok(Some(row))
                    if Self::known_uncommitted_artifact_row(
                        &row,
                        commit,
                        conversation_id,
                        root_turn_id,
                    ) => {}
                Ok(None) => {}
                Ok(Some(_)) => definitely_uncommitted = false,
                Err(error) => {
                    error!(error = %ErrorChain(&error), conversation_id, root_turn_id, "Artifact recovery audit failed");
                    return ArtifactCommitReconciliation::Indeterminate;
                }
            }
        }
        if durable == commits.len() {
            ArtifactCommitReconciliation::AllDurable
        } else if durable == 0 && definitely_uncommitted {
            ArtifactCommitReconciliation::DefinitelyNotCommitted
        } else {
            ArtifactCommitReconciliation::Indeterminate
        }
    }

    async fn commit_pending_artifact_deliveries(
        &self,
        generic: &HashMap<String, ToolCallEventData>,
        acp: &HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) -> Result<usize, ArtifactCommitFailure> {
        let committed_artifact_count = generic
            .values()
            .map(|data| data.artifacts.len())
            .sum::<usize>()
            + acp
                .values()
                .map(|data| Self::acp_artifact_receipts(data).len())
                .sum::<usize>();
        let has_local_receipts = generic.values().any(|data| !data.artifacts.is_empty())
            || acp.values().any(|data| {
                data.update.content.as_ref().is_some_and(|items| {
                    items.iter().any(|item| {
                        matches!(
                            item,
                            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                        )
                    })
                })
            });
        if has_local_receipts {
            let workspace = self
                .artifact_workspace
                .as_ref()
                .ok_or_else(|| {
                    nomifun_db::DbError::Conflict(
                        "artifact delivery has no canonical session workspace for final verification"
                            .to_owned(),
                    )
                })
                .map_err(ArtifactCommitFailure::before_commit)?;
            let store = ArtifactStore::new(workspace);
            for data in generic.values() {
                for artifact in &data.artifacts {
                    store.reverify_receipt(artifact).map_err(|error| {
                        ArtifactCommitFailure::before_commit(nomifun_db::DbError::Conflict(
                            format!(
                                "tool call '{}' artifact '{}' failed final verification: {error}",
                                data.call_id, artifact.id
                            ),
                        ))
                    })?;
                }
            }
            for data in acp.values() {
                for item in data.update.content.iter().flatten() {
                    if let nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                        artifact,
                        ..
                    } = item
                    {
                        store.reverify_receipt(artifact).map_err(|error| {
                            ArtifactCommitFailure::before_commit(
                                nomifun_db::DbError::Conflict(format!(
                                    "ACP tool call '{}' artifact '{}' failed final verification: {error}",
                                    data.update.tool_call_id, artifact.id
                                )),
                            )
                        })?;
                    }
                }
            }
        }

        let mut generic_calls = generic.values().collect::<Vec<_>>();
        generic_calls.sort_by(|left, right| left.call_id.cmp(&right.call_id));
        let mut acp_calls = acp.values().collect::<Vec<_>>();
        acp_calls.sort_by(|left, right| {
            left.update
                .tool_call_id
                .cmp(&right.update.tool_call_id)
        });

        let mut commits = Vec::with_capacity(generic_calls.len() + acp_calls.len());
        for data in generic_calls {
            commits.push(TurnArtifactMessageCommit {
                message_id: self
                    .try_derived_message_id("tool_call", &data.call_id)
                    .await
                    .map_err(ArtifactCommitFailure::before_commit)?,
                message_type: "tool_call".to_owned(),
                content: self
                    .committed_artifact_tool_content(data)
                    .map_err(ArtifactCommitFailure::before_commit)?,
            });
        }
        for data in acp_calls {
            commits.push(TurnArtifactMessageCommit {
                message_id: self
                    .try_derived_message_id("acp_tool_call", &data.update.tool_call_id)
                    .await
                    .map_err(ArtifactCommitFailure::before_commit)?,
                message_type: "acp_tool_call".to_owned(),
                content: self
                    .committed_artifact_acp_tool_content(data)
                    .map_err(ArtifactCommitFailure::before_commit)?,
            });
        }

        // Linearize successful-turn intent before entering COMMIT. A crash in
        // ClaimedActive is still a failed/incomplete turn and must roll back;
        // only this durable fence authorizes recovery replay.
        if let Some(workspace) = self.artifact_workspace.as_ref() {
            let mut receipts = generic
                .values()
                .flat_map(|data| data.artifacts.iter().cloned())
                .collect::<Vec<_>>();
            receipts.extend(acp.values().flat_map(Self::acp_artifact_receipts));
            let store = ArtifactStore::new(workspace);
            let skip_legacy_fence = if self.allows_legacy_unjournaled_artifacts() {
                let journaled = store
                    .recovery_records()
                    .map_err(|error| {
                        ArtifactCommitFailure::before_commit(nomifun_db::DbError::Conflict(
                            error.to_string(),
                        ))
                    })?
                    .into_iter()
                    .map(|record| record.receipt.id)
                    .collect::<HashSet<_>>();
                receipts.iter().all(|receipt| !journaled.contains(&receipt.id))
            } else {
                false
            };
            if !skip_legacy_fence {
                store
                    .mark_recovery_receipts_commit_attempting(&receipts)
                    .map_err(|error| {
                        ArtifactCommitFailure::before_commit(nomifun_db::DbError::Conflict(
                            format!("artifact commit intent could not be persisted: {error}"),
                        ))
                    })?;
            }
        }

        let commit_result = self
            .repo
            .commit_turn_artifact_messages(
                self.conv_id(),
                &self.root_turn_id,
                &commits,
                now_ms(),
            )
            .await;
        let commit_error = match commit_result {
            Ok(committed)
                if Self::returned_artifact_batch_is_exact(&committed, &commits, self.conv_id(), &self.root_turn_id) =>
            {
                return Ok(committed_artifact_count);
            }
            Ok(_) => nomifun_db::DbError::Conflict(
                "artifact commit returned an incomplete or mismatched durable batch".to_owned(),
            ),
            Err(error) => error,
        };

        // A COMMIT error can be an acknowledgement failure after SQLite made
        // the transaction durable. Never delete bytes based on that ambiguous
        // return value alone: query every exact terminal row. All rows durable
        // recovers success, no durable rows with only known provisional state
        // permits rollback, and every partial/query-unknown result retains the
        // snapshots for recovery rather than risking a dangling durable receipt.
        let reconciliation = self.reconcile_artifact_commit(&commits).await;
        if reconciliation == ArtifactCommitReconciliation::AllDurable {
            warn!(
                error = %ErrorChain(&commit_error),
                "Artifact COMMIT acknowledgement was inconsistent, but every exact row is durable"
            );
            return Ok(committed_artifact_count);
        }
        Err(ArtifactCommitFailure::after_reconciliation(
            commit_error,
            reconciliation,
        ))
    }

    fn returned_artifact_batch_is_exact(
        rows: &[MessageRow],
        commits: &[TurnArtifactMessageCommit],
        conversation_id: &str,
        turn_message_id: &str,
    ) -> bool {
        rows.len() == commits.len()
            && commits.iter().all(|commit| {
                rows.iter()
                    .filter(|row| {
                        Self::finished_artifact_row_matches(
                            row,
                            commit,
                            conversation_id,
                            turn_message_id,
                        )
                    })
                    .count()
                    == 1
            })
    }

    fn finished_artifact_row_matches(
        row: &MessageRow,
        commit: &TurnArtifactMessageCommit,
        conversation_id: &str,
        turn_message_id: &str,
    ) -> bool {
        row.message_id == commit.message_id
            && row.conversation_id == conversation_id
            && row.msg_id.as_deref() == Some(turn_message_id)
            && row.r#type == commit.message_type
            && row.content == commit.content
            && row.position.as_deref() == Some("left")
            && row.status.as_deref() == Some("finish")
            && !row.hidden
    }

    fn known_uncommitted_artifact_row(
        row: &MessageRow,
        commit: &TurnArtifactMessageCommit,
        conversation_id: &str,
        turn_message_id: &str,
    ) -> bool {
        let Some(stored_content) = serde_json::from_str::<Value>(&row.content).ok() else {
            return false;
        };
        let Some(candidate_content) = serde_json::from_str::<Value>(&commit.content).ok() else {
            return false;
        };
        let Some(stored_call_id) = Self::artifact_commit_call_identity(&row.r#type, &stored_content)
        else {
            return false;
        };
        let Some(candidate_call_id) =
            Self::artifact_commit_call_identity(&commit.message_type, &candidate_content)
        else {
            return false;
        };
        row.message_id == commit.message_id
            && row.conversation_id == conversation_id
            && row.msg_id.as_deref() == Some(turn_message_id)
            && row.r#type == commit.message_type
            && row.position.as_deref() == Some("left")
            // Only the relay's exact phase-one lifecycle is proof that no
            // receipt is durable. Error/unknown/conflicting rows deliberately
            // remain indeterminate and retain the physical snapshots.
            && row.status.as_deref() == Some("work")
            && !row.hidden
            && stored_content.get("turn_id").and_then(Value::as_str) == Some(turn_message_id)
            && stored_content
                .get(ARTIFACT_DELIVERY_COMMITTED_FIELD)
                .and_then(Value::as_bool)
                != Some(true)
            && stored_call_id == candidate_call_id
    }

    fn artifact_commit_call_identity<'a>(message_type: &str, content: &'a Value) -> Option<&'a str> {
        let identity = match message_type {
            "tool_call" => content.get("call_id").and_then(Value::as_str),
            "acp_tool_call" => content
                .get("update")
                .and_then(|update| update.get("tool_call_id"))
                .and_then(Value::as_str),
            _ => None,
        }?;
        let identity = identity.trim();
        (!identity.is_empty()).then_some(identity)
    }

    async fn reconcile_artifact_commit(
        &self,
        commits: &[TurnArtifactMessageCommit],
    ) -> ArtifactCommitReconciliation {
        let mut durable = 0usize;
        let mut all_other_rows_are_known_uncommitted = true;
        for commit in commits {
            let row = match self.repo.get_message(self.conv_id(), &commit.message_id).await {
                Ok(row) => row,
                Err(error) => {
                    error!(
                        message_id = %commit.message_id,
                        error = %ErrorChain(&error),
                        "Could not reconcile an ambiguous artifact COMMIT"
                    );
                    return ArtifactCommitReconciliation::Indeterminate;
                }
            };
            match row {
                Some(row)
                    if Self::finished_artifact_row_matches(
                        &row,
                        commit,
                        self.conv_id(),
                        &self.root_turn_id,
                    ) =>
                {
                    durable += 1;
                }
                Some(row)
                    if Self::known_uncommitted_artifact_row(
                        &row,
                        commit,
                        self.conv_id(),
                        &self.root_turn_id,
                    ) => {}
                None => {}
                Some(_) => all_other_rows_are_known_uncommitted = false,
            }
        }

        if durable == commits.len() {
            ArtifactCommitReconciliation::AllDurable
        } else if durable == 0 && all_other_rows_are_known_uncommitted {
            ArtifactCommitReconciliation::DefinitelyNotCommitted
        } else {
            ArtifactCommitReconciliation::Indeterminate
        }
    }

    fn rollback_completed_artifact_receipts(
        &self,
        generic: &HashMap<String, ToolCallEventData>,
        acp: &HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) {
        let mut receipts = generic
            .values()
            .flat_map(|data| data.artifacts.iter().cloned())
            .collect::<Vec<PersistedArtifact>>();
        receipts.extend(acp.values().flat_map(|data| {
            data.update.content.iter().flatten().filter_map(|item| {
                if let nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                    artifact,
                    ..
                } = item
                {
                    Some(artifact.clone())
                } else {
                    None
                }
            })
        }));
        if receipts.is_empty() {
            return;
        }

        let Some(workspace) = self.artifact_workspace.as_ref() else {
            error!(
                receipt_count = receipts.len(),
                "Cannot safely roll back provisional artifacts without the canonical session workspace"
            );
            return;
        };
        if let Err(error) = ArtifactStore::new(workspace).rollback_owned_receipts(&receipts) {
            error!(
                receipt_count = receipts.len(),
                error = %ErrorChain(&error),
                "Strict rollback of provisional artifact snapshots failed"
            );
        }
    }

    fn broadcast_committed_artifact_tool_calls(
        &self,
        completed: &HashMap<String, ToolCallEventData>,
    ) {
        let mut completed = completed.values().collect::<Vec<_>>();
        completed.sort_by(|left, right| left.call_id.cmp(&right.call_id));
        for data in completed {
            self.forward_to_websocket(&AgentStreamEvent::ToolCall(data.clone()));
        }
    }

    fn broadcast_committed_artifact_acp_tool_calls(
        &self,
        completed: &HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
    ) {
        let mut completed = completed.values().collect::<Vec<_>>();
        completed.sort_by(|left, right| {
            left.update
                .tool_call_id
                .cmp(&right.update.tool_call_id)
        });
        for data in completed {
            self.forward_to_websocket(&AgentStreamEvent::AcpToolCall(data.clone()));
        }
    }

    fn plan_terminal_status(event: &AgentStreamEvent) -> &'static str {
        match event {
            AgentStreamEvent::Finish(data)
                if matches!(
                    data.stop_reason,
                    None | Some(nomifun_ai_agent::protocol::events::TurnStopReason::EndTurn)
                ) => "finish",
            AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_) => "error",
            _ => "error",
        }
    }

    async fn finalize_active_plans(&self, active_plan_ids: &mut HashSet<String>, status: &str) {
        if active_plan_ids.len() > MAX_TERMINAL_ACTIVE_ITEMS {
            warn!(count = active_plan_ids.len(), "Truncating active plans during terminal cleanup");
        }
        for plan_id in active_plan_ids.drain().take(MAX_TERMINAL_ACTIVE_ITEMS) {
            let update = nomifun_db::MessageRowUpdate {
                content: None,
                status: Some(Some(status.to_owned())),
                hidden: None,
            };
            if let Err(error) = self.repo.update_message(&plan_id, &update).await {
                error!(
                    plan_id,
                    status,
                    error = %ErrorChain(&error),
                    "Failed to finalize active plan"
                );
            }
        }
    }

    fn take_failed_tool_calls(
        active_tool_calls: &mut HashMap<String, ToolCallEventData>,
        reason: &str,
    ) -> Vec<ToolCallEventData> {
        if active_tool_calls.is_empty() {
            return Vec::new();
        }

        if active_tool_calls.len() > MAX_TERMINAL_ACTIVE_ITEMS {
            warn!(count = active_tool_calls.len(), "Truncating active tool calls during terminal cleanup");
        }
        active_tool_calls
            .drain()
            .take(MAX_TERMINAL_ACTIVE_ITEMS)
            .map(|(_, mut data)| {
                let output = if data.status == ToolCallStatus::Completed {
                    format!(
                        "The turn ended without a valid completed delivery for this tool: {reason}"
                    )
                } else {
                    format!("The turn ended before this tool completed: {reason}")
                };
                data.status = ToolCallStatus::Error;
                data.output = Some(output);
                data.artifacts.clear();
                data
            })
            .collect()
    }

    fn broadcast_failed_tool_calls(&self, failed: &[ToolCallEventData]) {
        for data in failed {
            let event = AgentStreamEvent::ToolCall(data.clone());
            self.forward_to_websocket(&event);
        }
    }

    async fn persist_failed_tool_calls(&self, failed: &[ToolCallEventData]) {
        for data in failed {
            self.persist_tool_call(data).await;
        }
    }

    async fn fail_active_tool_calls(
        &self,
        active_tool_calls: &mut HashMap<String, ToolCallEventData>,
        reason: &str,
    ) {
        let failed = Self::take_failed_tool_calls(active_tool_calls, reason);
        self.broadcast_failed_tool_calls(&failed);
        self.persist_failed_tool_calls(&failed).await;
    }

    fn take_failed_acp_tool_calls(
        active_tool_calls: &mut HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
        reason: &str,
    ) -> Vec<nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData> {
        if active_tool_calls.len() > MAX_TERMINAL_ACTIVE_ITEMS {
            warn!(count = active_tool_calls.len(), "Truncating active ACP tool calls during terminal cleanup");
        }
        active_tool_calls
            .drain()
            .take(MAX_TERMINAL_ACTIVE_ITEMS)
            .map(|(_, mut data)| {
                let output = if data.update.status == Some(AcpToolCallStatus::Completed) {
                    format!(
                        "The turn ended without a valid completed delivery for this tool: {reason}"
                    )
                } else {
                    format!("The turn ended before this tool completed: {reason}")
                };
                data.update.session_update = AcpToolCallSessionUpdateKind::ToolCallUpdate;
                data.update.status = Some(AcpToolCallStatus::Failed);
                data.update.raw_output = Some(json!(output));
                if let Some(content) = data.update.content.as_mut() {
                    content.retain(|item| {
                        !matches!(
                            item,
                            nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact {
                                ..
                            } | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink {
                                ..
                            }
                        )
                    });
                }
                data
            })
            .collect()
    }

    fn broadcast_failed_acp_tool_calls(
        &self,
        failed: &[nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData],
    ) {
        for data in failed {
            let event = AgentStreamEvent::AcpToolCall(data.clone());
            self.forward_to_websocket(&event);
        }
    }

    async fn persist_failed_acp_tool_calls(
        &self,
        failed: &[nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData],
    ) {
        for data in failed {
            self.persist_acp_tool_call(&data).await;
        }
    }

    async fn fail_active_acp_tool_calls(
        &self,
        active_tool_calls: &mut HashMap<
            String,
            nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        >,
        reason: &str,
    ) {
        let failed = Self::take_failed_acp_tool_calls(active_tool_calls, reason);
        self.broadcast_failed_acp_tool_calls(&failed);
        self.persist_failed_acp_tool_calls(&failed).await;
    }

    async fn fail_active_tool_groups(
        &self,
        active_tool_groups: &mut HashMap<
            String,
            Vec<nomifun_ai_agent::protocol::events::tool_call::ToolGroupEntry>,
        >,
        reason: &str,
    ) {
        if active_tool_groups.len() > MAX_TERMINAL_ACTIVE_ITEMS {
            warn!(count = active_tool_groups.len(), "Truncating active tool groups during terminal cleanup");
        }
        let failed: Vec<_> = active_tool_groups
            .drain()
            .take(MAX_TERMINAL_ACTIVE_ITEMS)
            .map(|(_, mut entries)| {
                entries.truncate(MAX_TERMINAL_ACTIVE_ITEMS);
                for entry in &mut entries {
                    if entry.status == ToolCallStatus::Running {
                        entry.status = ToolCallStatus::Error;
                        let detail = format!("The turn ended before this tool completed: {reason}");
                        entry.description = Some(match entry.description.take() {
                            Some(description) if !description.is_empty() => format!("{description}: {detail}"),
                            _ => detail,
                        });
                    }
                }
                entries
            })
            .collect();

        for entries in failed {
            let event = AgentStreamEvent::ToolGroup(entries.clone());
            self.forward_to_websocket(&event);
            self.persist_tool_group(&entries).await;
        }
    }

    /// Persist an ACP (Claude CLI) tool call event.
    /// First event (ToolCall) inserts; subsequent events (ToolCallUpdate) update.
    #[tracing::instrument(skip_all)]
    async fn persist_acp_tool_call(&self, data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData) {
        let _ = self.persist_acp_tool_call_projection(data, None).await;
    }

    async fn persist_provisional_artifact_acp_tool_call(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
    ) -> bool {
        let provisional = Self::provisional_artifact_acp_tool_call(data);
        self.persist_acp_tool_call_projection(&provisional, Some(false))
            .await
    }

    fn provisional_artifact_acp_tool_call(
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
    ) -> nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData {
        let mut provisional = data.clone();
        provisional.update.status = Some(AcpToolCallStatus::InProgress);
        provisional.update.raw_output = Some(json!(ARTIFACT_DELIVERY_PENDING_OUTPUT));
        if let Some(content) = provisional.update.content.as_mut() {
            content.retain(|item| {
                !matches!(
                    item,
                    nomifun_ai_agent::protocol::events::AcpToolCallContentItem::Artifact { .. }
                        | nomifun_ai_agent::protocol::events::AcpToolCallContentItem::ResourceLink { .. }
                )
            });
        }
        provisional
    }

    async fn persist_acp_tool_call_projection(
        &self,
        data: &nomifun_ai_agent::protocol::events::tool_call::AcpToolCallEventData,
        artifact_delivery_committed: Option<bool>,
    ) -> bool {
        let tool_call_id = &data.update.tool_call_id;
        if tool_call_id.trim().is_empty() {
            warn!("Skipping ACP tool call persistence because tool_call_id is empty");
            return false;
        }
        let message_id = self.acp_tool_message_id(tool_call_id).await;
        let status = match data.update.status {
            Some(AcpToolCallStatus::Pending) | None => "work",
            Some(AcpToolCallStatus::InProgress) => "work",
            Some(AcpToolCallStatus::Completed) => "finish",
            Some(AcpToolCallStatus::Failed) => "error",
        };

        let mut value = serde_json::to_value(data).unwrap_or_default();
        normalize_keys_to_snake_case(&mut value);
        if let Some(object) = value.as_object_mut() {
            object.insert("turn_id".to_owned(), json!(self.root_turn_id));
            if let Some(committed) = artifact_delivery_committed {
                object.insert(ARTIFACT_DELIVERY_COMMITTED_FIELD.to_owned(), json!(committed));
            }
        }
        if data.update.status != Some(AcpToolCallStatus::Completed)
            && let Some(content) = value
                .get_mut("update")
                .and_then(|update| update.as_object_mut())
                .and_then(|update| update.get_mut("content"))
                .and_then(serde_json::Value::as_array_mut)
        {
            // A progress/failed frame may contain partial bytes or a remote
            // link, but those are not successful durable output. Keep text,
            // diffs, terminal diagnostics and artifact_error items only.
            content.retain(|item| {
                !matches!(
                    item.get("type").and_then(serde_json::Value::as_str),
                    Some("artifact" | "resource_link")
                )
            });
        }
        let content = value.to_string();

        let existing = match self.repo.get_message(self.conv_id(), &message_id).await {
            Ok(existing) => existing,
            Err(e) => {
                error!(
                    tool_call_id,
                    status,
                    error = %ErrorChain(&e),
                    "Failed to load ACP tool call before persistence"
                );
                return false;
            }
        };
        if let Some(existing_row) = existing {
            let existing_artifact_committed = serde_json::from_str::<Value>(&existing_row.content)
                .ok()
                .and_then(|value| {
                    value
                        .get(ARTIFACT_DELIVERY_COMMITTED_FIELD)
                        .and_then(Value::as_bool)
                })
                == Some(true);
            let terminal_conflict = match (existing_row.status.as_deref(), status) {
                (Some("finish"), "finish" | "error") | (Some("error"), "error") => false,
                (Some("finish"), _)
                    if artifact_delivery_committed == Some(false)
                        && !existing_artifact_committed =>
                {
                    false
                }
                (Some("finish" | "error"), _) => true,
                _ => false,
            };
            if terminal_conflict {
                warn!(
                    tool_call_id,
                    stored_status = ?existing_row.status,
                    incoming_status = status,
                    "Ignoring ACP tool transition away from persisted terminal state"
                );
                return false;
            }
            let merged_content = Self::merge_acp_tool_call_content(&existing_row.content, &value);
            let update = nomifun_db::MessageRowUpdate {
                content: Some(merged_content),
                status: Some(Some(status.to_owned())),
                hidden: None,
            };
            if let Err(e) = self.repo.update_message(&message_id, &update).await {
                error!(error = %ErrorChain(&e), "Failed to update acp_tool_call message");
                return false;
            }
            return true;
        }

        let row = MessageRow {
            id: 0,
            message_id: message_id.clone(),
            conversation_id: self.conversation_id.clone(),
            msg_id: Some(self.root_turn_id.clone()),
            r#type: "acp_tool_call".into(),
            content,
            position: Some("left".into()),
            status: Some(status.to_owned()),
            hidden: false,
            created_at: now_ms(),
        };
        if let Err(e) = self.repo.insert_message(&row).await {
            error!(error = %ErrorChain(&e), "Failed to persist acp_tool_call message");
            return false;
        }
        true
    }

    async fn acp_tool_message_id(&self, tool_call_id: &str) -> String {
        self.derived_message_id("acp_tool_call", tool_call_id).await
    }

    /// Merge two JSON content strings: overlays non-null fields from `new_json`
    /// onto `existing_json`, preserving fields only present in the original.
    fn merge_json_content(existing_json: &str, new_json: &str) -> String {
        let mut base: serde_json::Value = serde_json::from_str(existing_json).unwrap_or_default();
        let new_value: serde_json::Value = serde_json::from_str(new_json).unwrap_or_default();
        if let (Some(base_obj), Some(new_obj)) = (base.as_object_mut(), new_value.as_object()) {
            for (key, val) in new_obj {
                if !val.is_null() {
                    base_obj.insert(key.clone(), val.clone());
                }
            }
        }
        base.to_string()
    }

    /// Merge an AcpToolCall update into the existing DB record.
    /// Reads the stored content, overlays non-null fields from the update,
    /// preserving fields like `raw_input` that the update event omits.
    fn merge_acp_tool_call_content(existing_content: &str, update_value: &serde_json::Value) -> String {
        let mut base: serde_json::Value = serde_json::from_str(existing_content).unwrap_or_default();
        if let (Some(base_object), Some(update_object)) = (base.as_object_mut(), update_value.as_object()) {
            for (key, value) in update_object {
                if key != "update" && !value.is_null() {
                    base_object.insert(key.clone(), value.clone());
                }
            }
        }
        if let (Some(base_update), Some(new_update)) = (
            base.get_mut("update").and_then(|v| v.as_object_mut()),
            update_value.get("update").and_then(|v| v.as_object()),
        ) {
            for (key, val) in new_update {
                if !val.is_null() {
                    base_update.insert(key.clone(), val.clone());
                }
            }
            if new_update.get("status").and_then(serde_json::Value::as_str) == Some("failed")
                && let Some(content) = base_update
                    .get_mut("content")
                    .and_then(serde_json::Value::as_array_mut)
            {
                content.retain(|item| {
                    !matches!(
                        item.get("type").and_then(serde_json::Value::as_str),
                        Some("artifact" | "resource_link")
                    )
                });
            }
        }
        base.to_string()
    }

    /// Persist a tool_group event (array of tool summaries).
    #[tracing::instrument(skip_all)]
    async fn persist_tool_group(&self, entries: &[nomifun_ai_agent::protocol::events::tool_call::ToolGroupEntry]) {
        let status = if entries.iter().any(|entry| entry.status == ToolCallStatus::Error) {
            "error"
        } else if entries.iter().all(|entry| entry.status == ToolCallStatus::Completed) {
            "finish"
        } else {
            "work"
        };
        let content = serde_json::to_string(entries).unwrap_or_default();

        let source_group_id = entries
            .first()
            .map(|e| e.call_id.clone())
            .unwrap_or_else(ConversationService::mint_msg_id);
        let group_id = self.derived_message_id("tool_group", &source_group_id).await;

        let existing = self
            .repo
            .get_message(self.conv_id(), &group_id)
            .await
            .unwrap_or(None);

        if let Some(existing_row) = existing {
            let terminal_conflict = match (existing_row.status.as_deref(), status) {
                (Some("finish"), "finish") | (Some("error"), "error") => false,
                (Some("finish" | "error"), _) => true,
                _ => false,
            };
            if terminal_conflict {
                warn!(
                    group_id,
                    stored_status = ?existing_row.status,
                    incoming_status = status,
                    "Ignoring tool group transition away from persisted terminal state"
                );
                return;
            }
            let update = nomifun_db::MessageRowUpdate {
                content: Some(content),
                status: Some(Some(status.to_owned())),
                hidden: None,
            };
            if let Err(e) = self.repo.update_message(&group_id, &update).await {
                error!(error = %ErrorChain(&e), "Failed to update tool_group message");
            }
        } else {
            let row = MessageRow {
                id: 0,
                message_id: group_id.clone(),
                conversation_id: self.conversation_id.clone(),
                msg_id: Some(self.root_turn_id.clone()),
                r#type: "tool_group".into(),
                content,
                position: Some("left".into()),
                status: Some(status.to_owned()),
                hidden: false,
                created_at: now_ms(),
            };
            if let Err(e) = self.repo.insert_message(&row).await {
                error!(error = %ErrorChain(&e), "Failed to persist tool_group message");
            }
        }
    }

    /// Send a `thinking` event with `status: "done"` to close the thinking UI.
    fn send_thinking_done(&self, msg_id: &str, duration: u64) {
        let thinking_done = AgentStreamEvent::Thinking(ThinkingEventData {
            content: String::new(),
            subject: None,
            duration: Some(duration),
            status: Some("done".into()),
        });
        self.forward_to_websocket_with_msg_id(msg_id, &thinking_done);
    }

    async fn process_final_text(&self, text: &str) -> MiddlewareResult {
        let middleware = MessageMiddleware::new(
            self.cron_service
                .as_ref()
                .map(|service| Box::new(SharedCronService(Arc::clone(service))) as Box<dyn ICronService>),
        );

        let cancellation = self
            .cancellation
            .as_ref()
            .map(AgentTurnCancellation::cancellation_token);
        middleware
            .process_with_cancellation(
                text,
                &self.user_id,
                &self.conversation_id,
                cancellation.as_ref(),
            )
            .await
    }

    fn send_final_text_override(&self, msg_id: &str, text: &str, hidden: bool) {
        self.broadcast_stream_payload(json!({
            "conversation_id": self.conv_id(),
            "msg_id": msg_id,
            "type": "content",
            "data": { "content": text },
            "hidden": hidden,
            "replace": true,
        }));
    }

    fn send_system_responses(&self, responses: &[String]) {
        for response in responses {
            self.broadcast_stream_payload(json!({
                "conversation_id": self.conv_id(),
                "msg_id": ConversationService::mint_msg_id(),
                "type": "system",
                "data": response,
                "hidden": true,
            }));
        }
    }

    fn broadcast_stream_payload(&self, mut payload: serde_json::Value) {
        // Stamp the companion-companion + origin markers on every stream fragment
        // (the websocket consumers tolerate unknown fields; the companion collector
        // keys off them).
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("turn_id".into(), json!(self.root_turn_id));
            obj.insert("companion".into(), json!(self.companion));
            obj.insert("companion_id".into(), json!(self.companion_id));
            obj.insert("origin".into(), json!(self.origin));
            obj.insert("channel_platform".into(), json!(self.channel_platform));
        }
        let msg = WebSocketMessage::new("message.stream", payload);
        // Realtime delivery is a projection, never execution authority.  A
        // custom/embedded sink panic must not unwind the relay owner and then
        // panic again in the service's terminal-error recovery path, which
        // would otherwise strand the durable Conversation in Running with an
        // accepted receipt.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.user_events.send_to_user(&self.user_id, msg);
        }))
        .is_err()
        {
            error!(
                conversation_id = %self.conversation_id,
                turn_id = %self.root_turn_id,
                "User event sink panicked while projecting an agent stream event"
            );
        }
    }

    /// Emit `turn.completed` for the conversation, with the companion-companion
    /// wire markers and the turn's `origin` marker attached to the
    /// `turn.completed` payload (see [`Self::with_companion_context`] /
    /// [`Self::with_origin`]).
    #[cfg(test)]
    #[tracing::instrument(skip_all, fields(conversation_id = %conversation_id))]
    async fn complete_conversation_with_context(
        repo: &Arc<dyn IConversationRepository>,
        user_events: &Arc<dyn UserEventSink>,
        user_id: &str,
        conversation_id: &str,
        turn_id: Option<String>,
        runtime: Option<ConversationRuntimeSummary>,
        companion: bool,
        companion_id: Option<CompanionId>,
        origin: Option<String>,
        channel_platform: Option<String>,
    ) {
        if !Self::persist_conversation_finished(repo, conversation_id).await {
            warn!(
                conversation_id,
                "Suppressing turn.completed because durable Finished persistence failed"
            );
            return;
        }
        Self::broadcast_turn_completed_with_context(
            user_events,
            user_id,
            conversation_id,
            turn_id,
            runtime,
            companion,
            companion_id,
            origin,
            channel_platform,
        );
    }

    #[cfg(test)]
    async fn persist_conversation_finished(
        repo: &Arc<dyn IConversationRepository>,
        conversation_id: &str,
    ) -> bool {
        let update = nomifun_db::ConversationRowUpdate {
            status: Some("finished".to_owned()),
            updated_at: Some(now_ms()),
            ..Default::default()
        };
        match repo.update(conversation_id, &update).await {
            Ok(()) => true,
            Err(e) => {
                error!(
                    conversation_id,
                    error = %ErrorChain(&e),
                    "Failed to persist durable Finished conversation status"
                );
                false
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn broadcast_turn_completed_with_context(
        user_events: &Arc<dyn UserEventSink>,
        user_id: &str,
        conversation_id: &str,
        turn_id: Option<String>,
        runtime: Option<ConversationRuntimeSummary>,
        companion: bool,
        companion_id: Option<CompanionId>,
        origin: Option<String>,
        channel_platform: Option<String>,
    ) {
        let payload = json!({
            "conversation_id": conversation_id,
            "turn_id": turn_id,
            "status": "finished",
            "can_send_message": true,
            "runtime": runtime,
            "companion": companion,
            "companion_id": companion_id,
            "origin": origin,
            "channel_platform": channel_platform,
        });
        let msg = WebSocketMessage::new("turn.completed", payload);
        // Finished and exact release are already durable before production
        // callers reach this projection.  Keep a sink bug observational: it
        // may lose a wake-up, but it must not unwind lifecycle cleanup.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            user_events.send_to_user(user_id, msg);
        }))
        .is_err()
        {
            error!(
                conversation_id,
                "User event sink panicked while projecting turn.completed"
            );
        }

        debug!(conversation_id, status = "finished", "Turn completed");
    }

    async fn try_derived_message_id(
        &self,
        message_type: &str,
        correlation_key: &str,
    ) -> Result<String, nomifun_db::DbError> {
        let cache_key = format!("{message_type}\0{correlation_key}");
        if let Some(id) = self
            .derived_message_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&cache_key)
            .cloned()
        {
            return Ok(id);
        }

        let id = self
            .repo
            .claim_message_correlation(
                self.conv_id(),
                // Provider call/session ids are only guaranteed unique inside
                // one wire prompt. Continuations can legitimately reuse a call
                // id, so canonical row identity remains wire-scoped even though
                // the row's ownership (`msg_id`/content.turn_id) is root-scoped.
                &self.msg_id,
                message_type,
                correlation_key,
            )
            .await?;
        self.derived_message_ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(cache_key, id.clone());
        Ok(id)
    }

    async fn derived_message_id(&self, message_type: &str, correlation_key: &str) -> String {
        match self
            .try_derived_message_id(message_type, correlation_key)
            .await
        {
            Ok(id) => id,
            Err(error) => {
                error!(
                    message_type,
                    correlation_key,
                    error = %ErrorChain(&error),
                    "Failed to claim durable streamed-message correlation"
                );
                MessageId::new().into_string()
            }
        }
    }
}

struct SharedCronService(Arc<dyn ICronService>);

#[async_trait::async_trait]
impl ICronService for SharedCronService {
    async fn create_job(
        &self,
        user_id: &str,
        conversation_id: &str,
        params: &crate::response_middleware::CronCreateParams,
    ) -> crate::response_middleware::CronCommandResult {
        self.0.create_job(user_id, conversation_id, params).await
    }

    async fn update_job(
        &self,
        user_id: &str,
        conversation_id: &str,
        params: &crate::response_middleware::CronUpdateParams,
    ) -> crate::response_middleware::CronCommandResult {
        self.0.update_job(user_id, conversation_id, params).await
    }

    async fn list_jobs(&self, user_id: &str, conversation_id: &str) -> crate::response_middleware::CronCommandResult {
        self.0.list_jobs(user_id, conversation_id).await
    }

    async fn delete_job(&self, user_id: &str, job_id: &str) -> crate::response_middleware::CronCommandResult {
        self.0.delete_job(user_id, job_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_ai_agent::protocol::events::{
        ErrorEventData, FinishEventData, PlanEventData, TextEventData, ThinkingEventData,
    };
    use nomifun_common::{ConversationId, MessageId, PersistedArtifactId};
    use nomifun_db::DbError;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering},
    };

    const TEST_ASSISTANT_MESSAGE_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678941";
    const TEST_TURN_A: &str = "0190f5fe-7c00-7a00-8abc-012345678942";
    const TEST_TURN_B: &str = "0190f5fe-7c00-7a00-8abc-012345678943";
    const ONE_PIXEL_PNG: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    const TEST_USER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678944";

    fn test_conversation_id() -> String {
        ConversationId::new().into_string()
    }

    fn test_writeback_attempt(
        repo: Arc<dyn IConversationRepository>,
        user_events: Arc<dyn UserEventSink>,
        user_id: String,
        conversation_id: String,
        msg_id: String,
    ) -> TurnWritebackAttempt {
        TurnWritebackAttempt::new(
            repo,
            user_events,
            user_id,
            conversation_id,
            msg_id,
            TEST_TURN_A.to_owned(),
            "answer".to_owned(),
            Vec::new(),
            Vec::new(),
            1,
        )
    }

    #[test]
    fn corrected_retry_path_clears_historical_failure_terminal_state() {
        let kb_id = nomifun_common::KnowledgeBaseId::new();
        let report = nomifun_knowledge::TurnWritebackReport {
            status: nomifun_knowledge::TurnWritebackStatus::Written,
            candidates: 1,
            written: vec![nomifun_knowledge::WriteOutcome {
                kb_id: kb_id.clone(),
                final_rel_path: "Foo.md".into(),
                op: nomifun_knowledge::WriteOp::Create,
            }],
            failures: Vec::new(),
        };
        let prior_failures = vec![json!({
            "kb_id": kb_id,
            "rel_path": "Foo?.md",
            "error": "path component is not portable",
        })];

        let state = turn_writeback_final_state(
            &report,
            "attempt-2",
            2,
            1,
            2,
            &[],
            &prior_failures,
        );

        assert_eq!(state["status"], "written");
        assert_eq!(state["retryable"], false);
        assert_eq!(state["failures"], json!([]));
        assert_eq!(state["written"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn retry_without_candidate_keeps_unresolved_target_retryable() {
        let kb_id = nomifun_common::KnowledgeBaseId::new();
        let report = nomifun_knowledge::TurnWritebackReport {
            status: nomifun_knowledge::TurnWritebackStatus::NoCandidate,
            candidates: 0,
            written: Vec::new(),
            failures: Vec::new(),
        };
        let prior_written = vec![json!({
            "kb_id": kb_id,
            "rel_path": "A.md",
            "staged": false,
        })];
        let prior_failures = vec![json!({
            "kb_id": kb_id,
            "rel_path": "B.md",
            "error": "temporary failure",
        })];

        let state = turn_writeback_final_state(
            &report,
            "attempt-2",
            2,
            1,
            2,
            &prior_written,
            &prior_failures,
        );

        assert_eq!(state["status"], "partial");
        assert_eq!(state["retryable"], true);
        assert_eq!(state["failures"], json!(prior_failures));
    }

    #[test]
    fn retry_success_in_another_base_does_not_clear_prior_failure() {
        let failed_kb = nomifun_common::KnowledgeBaseId::new();
        let written_kb = nomifun_common::KnowledgeBaseId::new();
        let report = nomifun_knowledge::TurnWritebackReport {
            status: nomifun_knowledge::TurnWritebackStatus::Written,
            candidates: 1,
            written: vec![nomifun_knowledge::WriteOutcome {
                kb_id: written_kb,
                final_rel_path: "Unrelated.md".into(),
                op: nomifun_knowledge::WriteOp::Create,
            }],
            failures: Vec::new(),
        };
        let prior_failures = vec![json!({
            "kb_id": failed_kb,
            "rel_path": "StillPending.md",
            "error": "temporary failure",
        })];

        let state = turn_writeback_final_state(
            &report,
            "attempt-2",
            2,
            1,
            2,
            &[],
            &prior_failures,
        );

        assert_eq!(state["status"], "partial");
        assert_eq!(state["retryable"], true);
        assert_eq!(state["failures"], json!(prior_failures));
    }

    fn test_artifact(id: &str) -> nomifun_ai_agent::artifact_store::PersistedArtifact {
        nomifun_ai_agent::artifact_store::PersistedArtifact {
            id: PersistedArtifactId::new().into_string(),
            kind: nomifun_ai_agent::artifact_store::ArtifactKind::Image,
            mime_type: "image/png".into(),
            path: format!("/workspace/{id}.png"),
            relative_path: format!("nomifun-artifacts/{id}.png"),
            size_bytes: 10,
            sha256: "a".repeat(64),
        }
    }

    fn persisted_png_artifact(
        workspace: &std::path::Path,
    ) -> nomifun_ai_agent::artifact_store::PersistedArtifact {
        ArtifactStore::new(workspace)
            .persist_inline(
                nomifun_ai_agent::artifact_store::ArtifactKind::Image,
                "image/png",
                ONE_PIXEL_PNG,
            )
            .expect("persist verified test PNG")
    }

    struct TestUserEventBus {
        sender: broadcast::Sender<WebSocketMessage<Value>>,
    }

    impl TestUserEventBus {
        fn new(capacity: usize) -> Self {
            let (sender, _) = broadcast::channel(capacity);
            Self { sender }
        }

        fn subscribe(&self) -> broadcast::Receiver<WebSocketMessage<Value>> {
            self.sender.subscribe()
        }
    }

    impl UserEventSink for TestUserEventBus {
        fn send_to_user(&self, _user_id: &str, event: WebSocketMessage<Value>) {
            let _ = self.sender.send(event);
        }
    }

    struct PanicUserEventSink {
        calls: AtomicUsize,
    }

    impl PanicUserEventSink {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl UserEventSink for PanicUserEventSink {
        fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<Value>) {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            panic!("injected write-back event sink panic");
        }
    }

    fn seed_writeback_message(
        repo: &RecordingRepo,
        conversation_id: &str,
        message_id: &str,
        content: Value,
    ) {
        repo.inserts.lock().unwrap().push(MessageRow {
            id: 0,
            message_id: message_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            msg_id: Some(message_id.to_owned()),
            r#type: "text".to_owned(),
            content: content.to_string(),
            position: Some("left".to_owned()),
            status: Some("finish".to_owned()),
            hidden: false,
            created_at: now_ms(),
        });
    }

    #[test]
    fn terminal_writeback_state_absorbs_late_running_and_interrupted_for_same_attempt() {
        let terminal = json!({
            "status": "written",
            "attempt_id": "attempt-a",
            "started_at": 100,
            "updated_at": 300,
            "finished_at": 300,
        });
        let late_running =
            turn_writeback_running_state("writing", "attempt-a", 0, 100, 400, &[], &[]);
        let late_interrupted = turn_writeback_interrupted_state(
            "attempt-a",
            0,
            100,
            400,
            "late panic finalizer",
            &[],
            &[],
        );

        assert_eq!(
            reject_turn_writeback_transition(&terminal, &late_running),
            Some(TurnWritebackPersistOutcome::IgnoredTerminalAttempt)
        );
        assert_eq!(
            reject_turn_writeback_transition(&terminal, &late_interrupted),
            Some(TurnWritebackPersistOutcome::IgnoredTerminalAttempt)
        );
    }

    #[test]
    fn writeback_transition_rejects_stale_attempt_and_running_phase_regression() {
        let existing =
            turn_writeback_running_state("writing", "attempt-new", 0, 200, 250, &[], &[]);
        let stale_attempt =
            turn_writeback_running_state("started", "attempt-old", 0, 100, 300, &[], &[]);
        let newer_attempt =
            turn_writeback_running_state("started", "attempt-next", 1, 300, 300, &[], &[]);
        let phase_regression =
            turn_writeback_running_state("extracting", "attempt-new", 0, 200, 300, &[], &[]);

        assert_eq!(
            reject_turn_writeback_transition(&existing, &stale_attempt),
            Some(TurnWritebackPersistOutcome::IgnoredStaleAttempt)
        );
        assert_eq!(
            reject_turn_writeback_transition(&existing, &phase_regression),
            Some(TurnWritebackPersistOutcome::IgnoredStaleProgress)
        );
        assert_eq!(
            reject_turn_writeback_transition(&existing, &newer_attempt),
            None
        );

        let generation_two =
            turn_writeback_running_state("writing", "attempt-g2", 2, 200, 250, &[], &[]);
        let late_generation_one =
            turn_writeback_running_state("started", "attempt-g1", 1, 500, 500, &[], &[]);
        let early_generation_three =
            turn_writeback_running_state("started", "attempt-g3", 3, 100, 100, &[], &[]);
        let duplicate_generation_two =
            turn_writeback_running_state("started", "attempt-g2-duplicate", 2, 300, 300, &[], &[]);
        assert_eq!(
            reject_turn_writeback_transition(&generation_two, &late_generation_one),
            Some(TurnWritebackPersistOutcome::IgnoredStaleAttempt),
            "durable generation must beat a later wall-clock timestamp"
        );
        assert_eq!(
            reject_turn_writeback_transition(&generation_two, &early_generation_three),
            None,
            "a newer explicit retry generation remains admissible after clock rollback"
        );
        assert_eq!(
            reject_turn_writeback_transition(&generation_two, &duplicate_generation_two),
            Some(TurnWritebackPersistOutcome::IgnoredStaleAttempt),
            "one retry generation must have exactly one side-effect owner"
        );
    }

    #[tokio::test]
    async fn failed_writeback_persistence_does_not_broadcast_projection() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({ "content": "answer" }),
        );
        repo.fail_next_message_update();
        let bus = Arc::new(TestUserEventBus::new(8));
        let mut events = bus.subscribe();
        let repo_dyn: Arc<dyn IConversationRepository> = repo;
        let bus_dyn: Arc<dyn UserEventSink> = bus;

        let result = emit_turn_writeback_state(
            &repo_dyn,
            &bus_dyn,
            TEST_USER_ID,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            turn_writeback_running_state("started", "attempt-a", 0, 100, 100, &[], &[]),
        )
        .await;

        assert!(result.is_err());
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn terminal_writeback_persistence_retries_without_rebroadcasting_failure() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({ "content": "answer" }),
        );
        repo.fail_next_message_update();
        let bus = Arc::new(TestUserEventBus::new(8));
        let mut events = bus.subscribe();
        let attempt = test_writeback_attempt(
            repo.clone(),
            bus,
            TEST_USER_ID.to_owned(),
            conversation_id,
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        let terminal = json!({
            "status": "written",
            "attempt_id": attempt.attempt_id.clone(),
            "started_at": attempt.started_at,
            "updated_at": attempt.started_at + 10,
            "finished_at": attempt.started_at + 10,
            "retryable": false,
            "candidates": 1,
            "written": [],
            "failures": [],
        });

        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(1),
                persist_terminal_writeback_until_resolved(&attempt, terminal),
            )
            .await
            .expect("transient terminal persistence recovered"),
            TurnWritebackPersistOutcome::Committed
        );
        assert_eq!(
            repo.message_update_attempts.load(AtomicOrdering::SeqCst),
            2
        );
        assert_eq!(events.try_recv().unwrap().name, "knowledge.writeback");
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn terminal_persistence_survives_event_sink_panic_and_absorbs_interrupt() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({ "content": "answer" }),
        );
        let sink = Arc::new(PanicUserEventSink::new());
        let attempt = test_writeback_attempt(
            repo.clone(),
            sink.clone(),
            TEST_USER_ID.to_owned(),
            conversation_id,
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        let terminal = json!({
            "status": "written",
            "attempt_id": attempt.attempt_id.clone(),
            "started_at": attempt.started_at,
            "updated_at": attempt.started_at + 10,
            "finished_at": attempt.started_at + 10,
            "retryable": false,
            "candidates": 1,
            "written": [],
            "failures": [],
        });

        assert_eq!(
            attempt.emit(terminal).await.unwrap(),
            TurnWritebackPersistOutcome::Committed
        );
        assert_eq!(sink.calls.load(AtomicOrdering::SeqCst), 1);

        // RecordingRepo records updates but intentionally does not mutate its
        // inserted fixture. Reflect the acknowledged write here so the next
        // read observes the same durable state as the real repository.
        let persisted_content = repo
            .updates
            .lock()
            .unwrap()
            .last()
            .and_then(|(_, update)| update.content.clone())
            .expect("terminal write-back content");
        repo.inserts.lock().unwrap()[0].content = persisted_content;
        let updates_before_interrupt = repo.updates.lock().unwrap().len();

        attempt.interrupt("panic after terminal projection").await;

        assert_eq!(
            repo.updates.lock().unwrap().len(),
            updates_before_interrupt,
            "terminal persistence must absorb a late panic finalizer"
        );
        assert_eq!(
            sink.calls.load(AtomicOrdering::SeqCst),
            1,
            "ignored state must not be broadcast"
        );
    }

    #[tokio::test]
    async fn terminal_stream_sink_panic_is_projection_only_and_relay_still_returns() {
        let repo = Arc::new(RecordingRepo::new());
        let sink = Arc::new(PanicUserEventSink::new());
        let (tx, _) = broadcast::channel(8);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo,
            sink.clone(),
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(outcome.terminal, RelayTerminal::Finish);
        assert_eq!(
            sink.calls.load(AtomicOrdering::SeqCst),
            1,
            "a failed realtime projection must not be retried as business execution"
        );
    }

    #[tokio::test]
    async fn writeback_panic_persists_interrupted_before_disarming_owner() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        let attempt = test_writeback_attempt(
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            TEST_USER_ID.to_owned(),
            conversation_id.clone(),
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({
                "content": "answer",
                "knowledge_writeback": turn_writeback_running_state(
                    "writing",
                    &attempt.attempt_id,
                    attempt.attempt_generation,
                    attempt.started_at,
                    attempt.started_at + 1,
                    &attempt.prior_written,
                    &attempt.prior_failures,
                ),
            }),
        );
        let mut owner_guard = attempt.owner_guard("guard must be disarmed by panic recovery");

        let report = await_turn_writeback_report_or_interrupt(
            &attempt,
            &mut owner_guard,
            async { panic!("injected knowledge write-back panic") },
        )
        .await;

        assert!(report.is_none());
        let update = repo
            .updates
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("panic recovery must persist a terminal state");
        let content: Value =
            serde_json::from_str(update.1.content.as_deref().expect("updated content")).unwrap();
        assert_eq!(content["knowledge_writeback"]["status"], "interrupted");
        assert_eq!(
            content["knowledge_writeback"]["commit_ambiguous"],
            true
        );
        assert_eq!(content["knowledge_writeback"]["retryable"], false);

        drop(owner_guard);
        tokio::task::yield_now().await;
        assert_eq!(
            repo.updates.lock().unwrap().len(),
            1,
            "disarmed Drop must not schedule a duplicate terminal finalizer"
        );
    }

    #[tokio::test]
    async fn aborting_outer_owner_does_not_detach_registered_writeback_from_stop_fence() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        let attempt = test_writeback_attempt(
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            TEST_USER_ID.to_owned(),
            conversation_id.clone(),
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({
                "content": "answer",
                "knowledge_writeback": turn_writeback_running_state(
                    "writing",
                    &attempt.attempt_id,
                    attempt.attempt_generation,
                    attempt.started_at,
                    attempt.started_at + 1,
                    &attempt.prior_written,
                    &attempt.prior_failures,
                ),
            }),
        );
        let work_attempt = attempt.clone();
        let work_gate = Arc::new(Notify::new());
        let work_gate_for_task = Arc::clone(&work_gate);
        let (started_tx, started_rx) = oneshot::channel();
        let outer_owner = tokio::spawn(run_registered_turn_writeback(
            attempt,
            async move {
                let _ = started_tx.send(());
                work_gate_for_task.notified().await;
                work_attempt
                    .interrupt("injected tracked write-back completion")
                    .await;
                Ok(())
            },
        ));
        started_rx.await.expect("registered worker started");

        outer_owner.abort();
        assert!(
            outer_owner.await.expect_err("outer owner must be aborted").is_cancelled()
        );

        let conversation_for_fence = conversation_id.clone();
        let writeback_fence = tokio::spawn(async move {
            await_turn_writeback_quiesced(&conversation_for_fence).await;
        });
        tokio::task::yield_now().await;
        assert!(
            !writeback_fence.is_finished(),
            "stop fence must retain authority while the detached-but-tracked worker can still publish"
        );

        work_gate.notify_one();
        tokio::time::timeout(Duration::from_secs(1), writeback_fence)
            .await
            .expect("write-back fence completed")
            .expect("write-back fence task");
        let update = repo
            .updates
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("tracked worker persisted terminal state");
        let content: Value =
            serde_json::from_str(update.1.content.as_deref().expect("updated content")).unwrap();
        assert_eq!(content["knowledge_writeback"]["status"], "interrupted");
    }

    #[tokio::test(start_paused = true)]
    async fn quiesced_writeback_reconciliation_has_no_busy_timeout_release() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        let attempt = test_writeback_attempt(
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            TEST_USER_ID.to_owned(),
            conversation_id.clone(),
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({
                "content": "answer",
                "knowledge_writeback": turn_writeback_running_state(
                    "writing",
                    &attempt.attempt_id,
                    attempt.attempt_generation,
                    attempt.started_at,
                    attempt.started_at + 1,
                    &attempt.prior_written,
                    &attempt.prior_failures,
                ),
            }),
        );
        repo.block_message_updates();
        let repo_for_reconcile: Arc<dyn IConversationRepository> = repo.clone();
        let reconciliation = tokio::spawn(async move {
            reconcile_quiesced_writebacks_until_resolved(
                repo_for_reconcile,
                None,
                TEST_USER_ID,
                &conversation_id,
            )
            .await
        });
        for _ in 0..128 {
            if repo.message_update_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(repo.message_update_attempts(), 1);

        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(
            !reconciliation.is_finished(),
            "database busy time must not release a quiesced write-back fence"
        );
        reconciliation.abort();
        let _ = reconciliation.await;
    }

    #[tokio::test]
    async fn dropping_armed_writeback_owner_schedules_interrupted_persistence() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({ "content": "answer" }),
        );
        let attempt = test_writeback_attempt(
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            TEST_USER_ID.to_owned(),
            conversation_id,
            TEST_ASSISTANT_MESSAGE_ID.to_owned(),
        );
        let (armed_tx, armed_rx) = oneshot::channel();
        let owner = tokio::spawn(async move {
            let _guard = attempt.owner_guard("injected owner abort");
            let _ = armed_tx.send(());
            std::future::pending::<()>().await;
        });
        armed_rx.await.expect("owner armed");

        owner.abort();
        let _ = owner.await;
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !repo.updates.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("abort finalizer persisted interrupted state");

        let persisted: Value = serde_json::from_str(
            repo.updates
                .lock()
                .unwrap()
                .last()
                .and_then(|(_, update)| update.content.as_deref())
                .expect("interrupted content"),
        )
        .unwrap();
        assert_eq!(
            persisted["knowledge_writeback"]["status"],
            "interrupted"
        );
        assert_eq!(
            persisted["knowledge_writeback"]["failures"][0]["error"],
            "injected owner abort"
        );
        assert_eq!(persisted["knowledge_writeback"]["retryable"], false);
        assert_eq!(
            persisted["knowledge_writeback"]["commit_ambiguous"],
            true
        );
    }

    #[tokio::test]
    async fn orphan_reconciliation_interrupts_only_persisted_running_attempts() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_ASSISTANT_MESSAGE_ID,
            json!({
                "content": "running",
                "knowledge_writeback":
                    turn_writeback_running_state(
                        "writing",
                        "attempt-running",
                        0,
                        100,
                        200,
                        &[],
                        &[],
                    ),
            }),
        );
        seed_writeback_message(
            &repo,
            &conversation_id,
            TEST_TURN_A,
            json!({
                "content": "terminal",
                "knowledge_writeback": {
                    "status": "written",
                    "attempt_id": "attempt-terminal",
                    "started_at": 100,
                    "updated_at": 200,
                    "finished_at": 200,
                },
            }),
        );
        let events = Arc::new(TestUserEventBus::new(8));
        let mut receiver = events.subscribe();
        let repo_dyn: Arc<dyn IConversationRepository> = repo.clone();
        let events_dyn: Arc<dyn UserEventSink> = events;

        assert_eq!(
            reconcile_orphaned_writebacks(
                repo_dyn,
                Some(events_dyn),
                TEST_USER_ID,
                &conversation_id,
            )
            .await
            .unwrap(),
            1
        );
        let updates = repo.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, TEST_ASSISTANT_MESSAGE_ID);
        let persisted: Value =
            serde_json::from_str(updates[0].1.content.as_deref().unwrap()).unwrap();
        assert_eq!(
            persisted["knowledge_writeback"]["status"],
            "interrupted"
        );
        assert_eq!(persisted["knowledge_writeback"]["retryable"], false);
        assert_eq!(
            persisted["knowledge_writeback"]["commit_ambiguous"],
            true
        );
        drop(updates);

        let event = receiver.try_recv().expect("committed projection");
        assert_eq!(event.name, "knowledge.writeback");
        assert_eq!(event.data["status"], "interrupted");
        assert_eq!(event.data["msg_id"], TEST_ASSISTANT_MESSAGE_ID);
        match receiver.try_recv() {
            Err(
                broadcast::error::TryRecvError::Empty
                | broadcast::error::TryRecvError::Closed,
            ) => {}
            other => panic!("unexpected second orphan reconciliation event: {other:?}"),
        }
    }

    // ── run() async tests ─────────────────────────────────────────

    #[tokio::test]
    async fn turn_root_preflight_precedes_children_and_visible_segments_use_distinct_ids() {
        use nomifun_ai_agent::protocol::events::AgentStatusEventData;

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(32));
        let (tx, _) = broadcast::channel(32);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_root_turn_id(TEST_TURN_A);

        relay
            .ensure_turn_root_persisted()
            .await
            .expect("preflight persists the logical root");
        relay
            .ensure_turn_root_persisted()
            .await
            .expect("same-relay preflight is idempotent");

        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::AgentStatus(AgentStatusEventData {
            backend: "nomi".into(),
            status: "preparing".into(),
            agent_name: Some("Nomi".into()),
            session_id: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "ready".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.terminal, RelayTerminal::Finish);
        assert_eq!(outcome.committed_artifact_count, 0);

        let rows = repo.inserts.lock().unwrap().clone();
        let root_index = rows
            .iter()
            .position(|row| row.message_id == TEST_TURN_A)
            .expect("logical root row");
        let root = &rows[root_index];
        assert_eq!(root.r#type, "turn_root");
        assert_eq!(root.msg_id.as_deref(), Some(TEST_TURN_A));
        assert!(root.hidden);
        assert_eq!(
            serde_json::from_str::<Value>(&root.content).unwrap()["kind"],
            "turn_root"
        );

        let status_index = rows
            .iter()
            .position(|row| row.r#type == "agent_status")
            .expect("agent status child row");
        assert!(root_index < status_index);
        assert_eq!(rows[status_index].msg_id.as_deref(), Some(TEST_TURN_A));

        let text = rows
            .iter()
            .find(|row| row.r#type == "text")
            .expect("visible text segment");
        assert_ne!(text.message_id, TEST_TURN_A);
        assert_ne!(text.message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(text.msg_id.as_deref(), Some(text.message_id.as_str()));
        assert_eq!(
            outcome.final_text_msg_id.as_deref(),
            Some(text.message_id.as_str())
        );
    }

    #[tokio::test]
    async fn turn_root_preflight_accepts_a_legacy_visible_root_but_rejects_wrong_ownership() {
        let conversation_id = test_conversation_id();
        let repo = Arc::new(RecordingRepo::new());
        repo.inserts.lock().unwrap().push(MessageRow {
            id: 0,
            message_id: TEST_TURN_A.to_owned(),
            conversation_id: conversation_id.clone(),
            msg_id: Some(TEST_TURN_A.to_owned()),
            r#type: "text".to_owned(),
            content: json!({ "content": "legacy", "turn_id": TEST_TURN_A }).to_string(),
            position: Some("left".to_owned()),
            status: Some("finish".to_owned()),
            hidden: false,
            created_at: now_ms(),
        });
        let relay = StreamRelay::new(
            conversation_id,
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        )
        .with_root_turn_id(TEST_TURN_A);
        relay
            .ensure_turn_root_persisted()
            .await
            .expect("pre-upgrade text root remains a valid owner");
        assert_eq!(
            repo.inserts
                .lock()
                .unwrap()
                .iter()
                .filter(|row| row.message_id == TEST_TURN_A)
                .count(),
            1
        );

        let wrong_repo = Arc::new(RecordingRepo::new());
        wrong_repo.inserts.lock().unwrap().push(MessageRow {
            id: 0,
            message_id: TEST_TURN_B.to_owned(),
            conversation_id: test_conversation_id(),
            msg_id: Some(TEST_TURN_B.to_owned()),
            r#type: "text".to_owned(),
            content: json!({ "content": "user-owned collision" }).to_string(),
            position: Some("right".to_owned()),
            status: Some("finish".to_owned()),
            hidden: false,
            created_at: now_ms(),
        });
        let wrong_bus = Arc::new(TestUserEventBus::new(8));
        let mut wrong_events = wrong_bus.subscribe();
        let conflict_relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            wrong_repo.clone(),
            wrong_bus,
            None,
        )
        .with_root_turn_id(TEST_TURN_B);
        let conflict = conflict_relay
        .ensure_turn_root_persisted()
        .await
        .expect_err("a right-side message cannot become an assistant turn root");
        assert!(matches!(conflict, DbError::Conflict(_)));
        let outcome = conflict_relay.into_turn_root_failure_outcome(conflict);
        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert_eq!(wrong_repo.inserts.lock().unwrap().len(), 1);
        let event = wrong_events.try_recv().expect("preflight failure terminal event");
        assert_eq!(event.data["type"], "error");
        assert_eq!(event.data["turn_id"], TEST_TURN_B);
    }

    #[tokio::test]
    async fn run_text_then_finish_persists_message() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let conversation_id = test_conversation_id();
        let relay = StreamRelay::new(
            conversation_id.clone(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        // Send text events then finish
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Hello ".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "World".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.system_responses.is_empty());
        assert_eq!(outcome.terminal, RelayTerminal::Finish);
        // Plan D4: a turn that streamed Text is not pre-response.
        assert!(outcome.emitted_response);

        // Should have inserted a message with accumulated text
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        let msg = &inserts[0];
        assert_eq!(msg.conversation_id, conversation_id);
        assert_ne!(msg.message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(msg.msg_id.as_deref(), Some(msg.message_id.as_str()));
        assert_eq!(outcome.final_text_msg_id.as_deref(), Some(msg.message_id.as_str()));
        assert_eq!(msg.r#type, "text");
        assert_eq!(msg.status.as_deref(), Some("finish"));

        let content: serde_json::Value = serde_json::from_str(&msg.content).unwrap();
        assert_eq!(content["content"], "Hello World");
    }

    #[tokio::test(start_paused = true)]
    async fn non_terminal_persistence_has_no_local_timeout_or_circuit_breaker() {
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            Arc::new(RecordingRepo::new()),
            Arc::new(TestUserEventBus::new(8)),
            None,
        );
        let first = relay.ordered_event_side_effect(
                "never_resolves",
                std::future::pending::<()>(),
            );
        tokio::pin!(first);
        assert!(
            tokio::time::timeout(Duration::from_secs(60), &mut first)
                .await
                .is_err(),
            "elapsed wall time must not abandon an issued repository mutation"
        );
        drop(first);

        let polls = Arc::new(AtomicUsize::new(0));
        let polls_for_future = Arc::clone(&polls);
        relay
            .ordered_event_side_effect(
                "must_not_poll",
                async move {
                    polls_for_future.fetch_add(1, AtomicOrdering::SeqCst);
                },
            )
            .await;
        assert_eq!(
            polls.load(AtomicOrdering::SeqCst),
            1,
            "a previously stalled call must not poison later ordered persistence"
        );
    }

    #[tokio::test]
    async fn failed_streaming_text_insert_is_retried_by_terminal_finalization() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_next_message_insert();
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "x".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(outcome.terminal, RelayTerminal::Finish);
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1, "the failed work insert must be retried as the terminal row");
        assert_eq!(
            outcome.final_text_msg_id.as_deref(),
            Some(inserts[0].message_id.as_str())
        );
        assert_ne!(inserts[0].message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(inserts[0].status.as_deref(), Some("finish"));
        let content: Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(content["content"], "x".repeat(FLUSH_INTERVAL as usize));
        assert!(
            repo.take_updates().is_empty(),
            "a failed insert must not make finalization update a nonexistent row"
        );
    }

    #[tokio::test]
    async fn ambiguous_streaming_insert_is_reconciled_without_a_duplicate_row() {
        let repo = Arc::new(RecordingRepo::new());
        repo.commit_next_message_insert_then_error();
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "x".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        let inserts = repo.take_inserts();
        assert_eq!(
            inserts.len(),
            1,
            "a committed-but-unacknowledged insert must be reconciled, not duplicated"
        );
        assert_eq!(
            outcome.final_text_msg_id.as_deref(),
            Some(inserts[0].message_id.as_str())
        );
        let updates = repo.take_updates();
        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates[0]
                .1
                .status
                .as_ref()
                .and_then(|status| status.as_deref()),
            Some("work"),
            "the ambiguous streaming insert is reconciled to its intended work state"
        );
        assert_eq!(
            updates[1]
                .1
                .status
                .as_ref()
                .and_then(|status| status.as_deref()),
            Some("finish")
        );
    }

    #[tokio::test]
    async fn persistent_terminal_insert_failure_surfaces_state_inconsistent_error() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_inserts();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "visible but unavailable database".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert!(outcome.terminal.is_error());
        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert!(outcome.emitted_response);
        assert!(outcome.final_text.is_none());
        assert!(outcome.final_text_msg_id.is_none());
        assert!(
            repo.take_inserts().iter().all(|row| row.r#type != "text"),
            "no text row may be claimed after every insert attempt failed"
        );

        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
            }
        }
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
    }

    #[tokio::test]
    async fn failed_text_finalization_keeps_the_segment_retryable_and_untracked() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_next_message_insert();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        );
        let mut active_text = Some(TextSegmentState {
            id: TEST_ASSISTANT_MESSAGE_ID.into(),
            buffer: "durable answer".into(),
            created_at: now_ms(),
            record_created: false,
            flush_counter: 0,
        });
        let mut text_segments = Vec::new();

        relay
            .close_active_text_segment(&mut active_text, &mut text_segments, "finish")
            .await;

        assert!(active_text.is_some(), "a failed final write must retain the retry state");
        assert!(
            text_segments.is_empty(),
            "a failed final write must not be reported as a persisted segment"
        );

        relay
            .close_active_text_segment(&mut active_text, &mut text_segments, "finish")
            .await;

        assert!(active_text.is_none());
        assert_eq!(text_segments.len(), 1);
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].status.as_deref(), Some("finish"));
    }

    #[tokio::test]
    async fn transient_terminal_update_failure_retries_the_existing_work_row() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_next_message_update();
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "x".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1, "the work row must not be inserted a second time");
        assert_eq!(
            outcome.final_text_msg_id.as_deref(),
            Some(inserts[0].message_id.as_str())
        );
        assert_eq!(inserts[0].status.as_deref(), Some("work"));
        let updates = repo.take_updates();
        assert_eq!(updates.len(), 1, "terminal finalization should retry exactly once");
        assert_eq!(updates[0].0, inserts[0].message_id);
        assert_eq!(
            updates[0].1.status.as_ref().and_then(|status| status.as_deref()),
            Some("finish")
        );
    }

    #[tokio::test]
    async fn persistent_terminal_update_failure_does_not_claim_or_insert_the_work_row() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_updates();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "x".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert!(outcome.terminal.is_error());
        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert!(outcome.emitted_response, "the visible text must continue to block failover");
        assert!(outcome.final_text.is_none());
        assert!(
            outcome.final_text_msg_id.is_none(),
            "an unfinalized work row must not be advertised as durable final text"
        );
        let inserts = repo.take_inserts();
        let text_rows: Vec<_> = inserts.iter().filter(|row| row.r#type == "text").collect();
        assert_eq!(
            text_rows.len(),
            1,
            "finalize must not fall back to a conflicting INSERT for an existing work row"
        );
        assert_eq!(text_rows[0].status.as_deref(), Some("work"));
        assert!(
            inserts.iter().any(|row| row.r#type == "tips" && row.status.as_deref() == Some("error")),
            "the state-inconsistent terminal must itself be durable"
        );
        assert!(repo.take_updates().is_empty());

        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
            }
        }
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
    }

    #[tokio::test]
    async fn text_persistence_failure_prevents_completed_artifact_commit() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_updates();
        let bus = Arc::new(TestUserEventBus::new(128));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(128);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-text-persistence-artifact-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        assert!(artifact_path.is_file());
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-before-unpersisted-text".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "x".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        let inserts = repo.take_inserts();
        let tool_row = inserts
            .iter()
            .find(|row| row.r#type == "tool_call")
            .expect("artifact tool has a provisional row");
        assert_eq!(tool_row.status.as_deref(), Some("work"));
        assert!(
            repo.take_updates().iter().all(|(id, update)| {
                id != &tool_row.message_id
                    || update.status.as_ref().and_then(|status| status.as_deref())
                        != Some("finish")
            }),
            "artifact receipt must not commit after assistant text durability fails"
        );

        let mut tool_statuses = Vec::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call"
                && let Some(status) = event.data["data"]["status"].as_str()
            {
                tool_statuses.push(status.to_owned());
            }
        }
        assert!(!tool_statuses.iter().any(|status| status == "completed"));
        assert_eq!(tool_statuses.last().map(String::as_str), Some("error"));
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
        assert!(
            !artifact_path.exists(),
            "assistant persistence failure must roll back the provisional snapshot"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_nonterminal_text_close_is_awaited_to_definitive_completion() {
        let repo = Arc::new(RecordingRepo::new());
        repo.set_block_message_inserts(true);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        );
        let mut active_text = Some(TextSegmentState {
            id: TEST_ASSISTANT_MESSAGE_ID.into(),
            buffer: "answer after a busy database".into(),
            created_at: now_ms(),
            record_created: false,
            flush_counter: 0,
        });
        let mut text_segments = Vec::new();

        {
            let mut ordered = Box::pin(relay.ordered_event_side_effect(
                "close_text_before_tool",
                relay.close_active_text_segment(
                    &mut active_text,
                    &mut text_segments,
                    "finish",
                ),
            ));
            assert!(
                tokio::time::timeout(Duration::from_secs(60), &mut ordered)
                    .await
                    .is_err(),
                "the old one-second bound must not abandon the text insert"
            );
            repo.set_block_message_inserts(false);
            ordered.await;
        }

        assert!(active_text.is_none());
        assert_eq!(text_segments.len(), 1);
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        let content: Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(content["content"], "answer after a busy database");
        assert_eq!(inserts[0].status.as_deref(), Some("finish"));
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_nonterminal_update_withholds_terminal_and_commits_before_finish() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(128));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(128);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        let relay_task = tokio::spawn(relay.consume(rx));

        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "a".into(),
            }))
            .unwrap();
        }
        for _ in 0..128 {
            if repo.message_insert_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(repo.message_insert_attempts(), 1);

        repo.set_block_message_updates(true);
        for _ in 0..FLUSH_INTERVAL {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: "b".into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        for _ in 0..128 {
            if repo.message_update_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            repo.message_update_attempts(),
            1,
            "the relay must be blocked in the nonterminal `work` update"
        );

        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(
            !relay_task.is_finished(),
            "elapsed wall time must not abandon a queued update to consume Finish"
        );
        let stream_types = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .filter(|event| event.name == "message.stream")
            .map(|event| event.data["type"].clone())
            .collect::<Vec<_>>();
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert!(!stream_types.iter().any(|kind| *kind == json!("error")));

        repo.set_block_message_updates(false);
        let outcome = tokio::time::timeout(Duration::from_secs(1), relay_task)
            .await
            .expect("relay completed after the ordered update was acknowledged")
            .expect("relay task");
        assert_eq!(outcome.terminal, RelayTerminal::Finish);

        let updates = repo.take_updates();
        assert_eq!(
            updates.len(),
            2,
            "one nonterminal update and one terminal update must commit"
        );
        assert_eq!(
            updates[0]
                .1
                .status
                .as_ref()
                .and_then(|status| status.as_deref()),
            Some("work")
        );
        assert_eq!(
            updates[1]
                .1
                .status
                .as_ref()
                .and_then(|status| status.as_deref()),
            Some("finish"),
            "terminal status must be the last physical update"
        );
        tokio::task::yield_now().await;
        assert!(
            repo.take_updates().is_empty(),
            "no abandoned nonterminal update may commit after terminal cleanup"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_terminal_assistant_insert_retains_turn_and_withholds_finish() {
        let repo = Arc::new(RecordingRepo::new());
        repo.set_block_message_inserts(true);
        let bus = Arc::new(TestUserEventBus::new(16));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(16);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "durability must precede terminal publication".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let relay_task = tokio::spawn(relay.consume(rx));
        for _ in 0..128 {
            if repo.message_insert_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            repo.message_insert_attempts(),
            1,
            "the relay must be blocked at the assistant terminal insert"
        );
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(
            !relay_task.is_finished(),
            "elapsed wall time must not turn an unacknowledged assistant insert into Finish"
        );

        let stream_types = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .filter(|event| event.name == "message.stream")
            .map(|event| event.data["type"].clone())
            .collect::<Vec<_>>();
        assert!(stream_types.iter().any(|kind| *kind == json!("content")));
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert!(!stream_types.iter().any(|kind| *kind == json!("error")));

        relay_task.abort();
        let _ = relay_task.await;
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_nonterminal_thinking_close_is_awaited_and_sends_done_once() {
        let repo = Arc::new(RecordingRepo::new());
        repo.set_block_message_inserts(true);
        let bus = Arc::new(TestUserEventBus::new(16));
        let mut ws_rx = bus.subscribe();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let mut active_thinking = Some(ThinkingSegmentState {
            id: TEST_ASSISTANT_MESSAGE_ID.into(),
            buffer: "reasoning".into(),
            started_at: now_ms(),
            completed_duration_ms: None,
        });

        {
            let mut ordered = Box::pin(relay.ordered_event_side_effect(
                "complete_thinking_before_text",
                relay.complete_active_thinking(&mut active_thinking),
            ));
            assert!(
                tokio::time::timeout(Duration::from_secs(60), &mut ordered)
                    .await
                    .is_err(),
                "the old one-second bound must not abandon the thinking insert"
            );
            repo.set_block_message_inserts(false);
            assert!(ordered.await);
        }

        assert!(active_thinking.is_none());
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].r#type, "thinking");

        let done_count = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .filter(|event| {
                event.name == "message.stream"
                    && event.data["type"] == "thinking"
                    && event.data["data"]["status"] == "done"
            })
            .count();
        assert_eq!(done_count, 1, "a persistence retry must not duplicate thinking.done");
    }

    #[tokio::test]
    async fn thinking_insert_reconcile_update_failure_remains_retryable() {
        let repo = Arc::new(RecordingRepo::new());
        repo.commit_next_message_insert_then_error();
        repo.fail_next_message_update();
        repo.reject_duplicate_message_inserts();
        let bus = Arc::new(TestUserEventBus::new(16));
        let mut ws_rx = bus.subscribe();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let mut active_thinking = Some(ThinkingSegmentState {
            id: TEST_ASSISTANT_MESSAGE_ID.into(),
            buffer: "reasoning".into(),
            started_at: now_ms(),
            completed_duration_ms: None,
        });

        assert!(!relay.complete_active_thinking(&mut active_thinking).await);
        assert!(active_thinking.is_some());
        assert!(relay.complete_active_thinking(&mut active_thinking).await);
        assert!(active_thinking.is_none());

        assert_eq!(repo.take_inserts().len(), 1);
        let updates = repo.take_updates();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(
            updates[0].1.status.as_ref().and_then(|status| status.as_deref()),
            Some("finish")
        );
        let done_count = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .filter(|event| {
                event.name == "message.stream"
                    && event.data["type"] == "thinking"
                    && event.data["data"]["status"] == "done"
            })
            .count();
        assert_eq!(done_count, 1);
    }

    #[tokio::test]
    async fn persistent_thinking_insert_failure_rejects_finish() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_inserts();
        let bus = Arc::new(TestUserEventBus::new(32));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(32);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: "visible reasoning".into(),
            subject: None,
            duration: None,
            status: Some("thinking".into()),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert!(outcome.emitted_response);
        assert!(
            repo.take_inserts().iter().all(|row| row.r#type != "thinking"),
            "failed thinking writes must not be claimed as history"
        );
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
            }
        }
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
    }

    #[tokio::test]
    async fn lagged_stream_with_live_sender_becomes_one_bounded_terminal_error() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(16));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(1);
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "overwrites the only finish".into(),
        }))
        .unwrap();

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo,
            bus,
            None,
        );
        let outcome = tokio::time::timeout(Duration::from_secs(2), relay.consume(rx))
            .await
            .expect("live sender must not keep a lagged relay pending");

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStreamBroken)
        );
        assert_eq!(tx.receiver_count(), 0, "relay receiver is released after terminal fallback");
        let mut error_events = 0;
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" && event.data["type"] == "error" {
                error_events += 1;
            }
        }
        assert_eq!(error_events, 1);
        assert!(tx.send(AgentStreamEvent::Finish(FinishEventData::default())).is_err());
    }

    // UC-2b: a relay wired with runtime state accumulates the TurnCompleted token
    // usage (input + output) into the conversation's running total — the seam the
    // owning execution attempt reads the accumulated total after settle.
    #[tokio::test]
    async fn turn_completed_accumulates_tokens_into_wired_runtime_state() {
        use nomifun_ai_agent::protocol::events::TurnCompletedEventData;

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let runtime_state = Arc::new(ConversationRuntimeStateService::default());

        let conversation_id = test_conversation_id();
        let relay = StreamRelay::new(conversation_id.clone(), TEST_ASSISTANT_MESSAGE_ID.into(), TEST_USER_ID.into(), repo, bus, None)
            .with_runtime_state(runtime_state.clone());
        let rx = tx.subscribe();

        // Two TurnCompleted events (e.g. a continuation) then Finish.
        tx.send(AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            input_tokens: 100,
            output_tokens: 40,
            ..Default::default()
        }))
        .unwrap();
        tx.send(AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            input_tokens: 30,
            output_tokens: 10,
            ..Default::default()
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let _ = relay.consume(rx).await;

        // (100+40) + (30+10) = 180, keyed by the relay's conversation id.
        assert_eq!(runtime_state.take_turn_tokens(&conversation_id), Some(180));
    }

    // Zero-regression: a relay WITHOUT runtime state wired (the default chat path)
    // records nothing — no accumulator entry for the conversation.
    #[tokio::test]
    async fn turn_completed_without_runtime_state_records_nothing() {
        use nomifun_ai_agent::protocol::events::TurnCompletedEventData;

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let observer = Arc::new(ConversationRuntimeStateService::default());

        let conversation_id = test_conversation_id();
        let relay = StreamRelay::new(conversation_id.clone(), TEST_ASSISTANT_MESSAGE_ID.into(), TEST_USER_ID.into(), repo, bus, None);
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::TurnCompleted(TurnCompletedEventData {
            input_tokens: 999,
            output_tokens: 1,
            ..Default::default()
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let _ = relay.consume(rx).await;

        // The relay was never given this runtime state, so it cannot have written.
        assert_eq!(observer.take_turn_tokens(&conversation_id), None);
    }

    #[tokio::test]
    async fn run_plan_event_persists_message_for_history_reload() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Plan(PlanEventData {
            session_id: Some("session-1".into()),
            source_call_id: None,
            entries: vec![
                json!({ "content": "Inspect current renderer path", "status": "completed" }),
                json!({ "content": "Persist plan rows", "status": "in_progress" }),
            ],
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let plan_msg = inserts.iter().find(|m| m.r#type == "plan").expect("plan message must be persisted");
        MessageId::parse(&plan_msg.message_id).expect("plan row has a canonical message ID");
        assert_eq!(plan_msg.msg_id.as_deref(), Some(plan_msg.message_id.as_str()));
        assert_eq!(plan_msg.status.as_deref(), Some("work"));

        let content: serde_json::Value = serde_json::from_str(&plan_msg.content).unwrap();
        assert_eq!(content["session_id"], "session-1");
        assert_eq!(content["entries"].as_array().unwrap().len(), 2);
        assert_eq!(content["entries"][1]["status"], "in_progress");
        let updates = repo.take_updates();
        let (_, terminal_update) = updates
            .iter()
            .find(|(id, _)| id == &plan_msg.message_id)
            .expect("incomplete plan must be closed with the turn");
        assert_eq!(
            terminal_update.status.as_ref().map(|status| status.as_deref()),
            Some(Some("finish"))
        );
        assert!(outcome.emitted_response);
    }

    #[tokio::test]
    async fn run_plan_event_completes_and_hides_its_source_tool() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-plan".into(),
            name: "update_plan".into(),
            args: json!({"plan": []}),
            status: ToolCallStatus::Running,
            input: Some(json!({"plan": []})),
            output: None,
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Plan(PlanEventData {
            session_id: Some("update_plan".into()),
            source_call_id: Some("tc-plan".into()),
            entries: vec![json!({"content": "Build game", "status": "completed"})],
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy("later provider error", None)))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.committed_artifact_count, 0);

        let source_id = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("source tool must be persisted")
            .message_id;
        MessageId::parse(&source_id).expect("tool row has a canonical message ID");
        let updates = repo.take_updates();
        let source_updates: Vec<_> = updates
            .iter()
            .filter(|(id, _)| id == &source_id)
            .collect();
        assert_eq!(source_updates.len(), 1, "source tool must settle exactly once");
        let update = &source_updates[0].1;
        assert_eq!(update.status.as_ref().map(|v| v.as_deref()), Some(Some("finish")));
        assert_eq!(update.hidden, Some(true));
        let content: serde_json::Value =
            serde_json::from_str(update.content.as_deref().expect("completed source content")).unwrap();
        assert_eq!(content["status"], "completed");
    }

    #[tokio::test]
    async fn run_terminal_error_closes_preparing_agent_status() {
        use nomifun_ai_agent::protocol::events::AgentStatusEventData;

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::AgentStatus(AgentStatusEventData {
            backend: "nomi".into(),
            status: "preparing".into(),
            agent_name: Some("Nomi".into()),
            session_id: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy("provider failed", None)))
            .unwrap();

        relay.consume(rx).await;

        let status_id = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "agent_status")
            .expect("agent status must be persisted")
            .message_id;
        MessageId::parse(&status_id).expect("agent status has a canonical message ID");
        let updates = repo.take_updates();
        let (_, update) = updates
            .iter()
            .find(|(id, _)| id == &status_id)
            .expect("preparing agent status must close on terminal error");
        assert_eq!(update.status.as_ref().map(|s| s.as_deref()), Some(Some("error")));
        let content: serde_json::Value = serde_json::from_str(update.content.as_deref().unwrap()).unwrap();
        assert_eq!(content["status"], "error");
    }

    #[tokio::test]
    async fn run_text_tool_text_splits_text_segments() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Alpha".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-001".into(),
            name: "read_file".into(),
            args: json!({"path": "a.ts"}),
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "Beta".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let text_msgs: Vec<_> = inserts.iter().filter(|msg| msg.r#type == "text").collect();
        assert_eq!(text_msgs.len(), 2, "text should split across tool boundaries");
        assert_ne!(text_msgs[0].message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_ne!(text_msgs[0].message_id, text_msgs[1].message_id);

        let mut text_event_msg_ids = Vec::new();
        while let Ok(evt) = ws_rx.try_recv() {
            if evt.name == "message.stream" && (evt.data["type"] == "text" || evt.data["type"] == "content") {
                text_event_msg_ids.push(evt.data["msg_id"].as_str().unwrap_or_default().to_owned());
            }
        }
        assert_eq!(text_event_msg_ids.len(), 2);
        assert_eq!(text_event_msg_ids[0], text_msgs[0].message_id);
        assert_eq!(text_event_msg_ids[1], text_msgs[1].message_id);
        assert_ne!(text_event_msg_ids[0], text_event_msg_ids[1]);
    }

    #[tokio::test]
    async fn terminal_error_does_not_relabel_completed_text_segments() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "Before".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-1".into(),
            name: "Read".into(),
            args: json!({}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("ok".into()),
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "After".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy("provider failed", None)))
            .unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let text_rows: Vec<_> = inserts.iter().filter(|row| row.r#type == "text").collect();
        assert_eq!(text_rows.len(), 2);
        assert_eq!(text_rows[0].status.as_deref(), Some("finish"));
        assert_eq!(text_rows[1].status.as_deref(), Some("error"));
        let updates = repo.take_updates();
        assert!(
            updates.iter().all(|(id, update)| {
                id != &text_rows[0].message_id
                    || update.status.as_ref().map(|status| status.as_deref()) != Some(Some("error"))
            }),
            "a later provider error must not corrupt an earlier completed text segment"
        );
    }

    #[tokio::test]
    async fn run_error_with_no_text_stores_tips_message() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "Something went wrong",
            None,
        )))
        .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.system_responses.is_empty());
        assert_eq!(
            outcome.terminal,
            RelayTerminal::Error {
                code: None,
                retryable: None
            }
        );
        // Plan D4: an error with no streamed Text is pre-response — the failover
        // seam is allowed to switch models on this kind of terminal error.
        assert!(!outcome.emitted_response);

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        let msg = &inserts[0];
        assert_eq!(msg.r#type, "tips");
        assert_eq!(msg.status.as_deref(), Some("error"));
        assert_eq!(msg.msg_id.as_deref(), Some(msg.message_id.as_str()));
        assert_ne!(msg.message_id, TEST_ASSISTANT_MESSAGE_ID);

        let content: serde_json::Value = serde_json::from_str(&msg.content).unwrap();
        assert_eq!(content["content"], "Something went wrong");
        assert_eq!(content["type"], "error");
        assert_eq!(content["turn_id"], TEST_ASSISTANT_MESSAGE_ID);

        let live_error = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .find(|event| event.name == "message.stream" && event.data["type"] == "error")
            .expect("terminal error must be broadcast");
        assert_eq!(live_error.data["msg_id"], msg.message_id);
        assert_eq!(live_error.data["turn_id"], TEST_ASSISTANT_MESSAGE_ID);
    }

    #[tokio::test]
    async fn partial_text_error_persists_a_distinct_canonical_error_message() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "partial answer".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy("late provider failure", None)))
            .unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let text = inserts.iter().find(|row| row.r#type == "text").expect("partial text row");
        let error = inserts.iter().find(|row| row.r#type == "tips").expect("error tips row");
        assert_eq!(text.status.as_deref(), Some("error"));
        assert_eq!(error.status.as_deref(), Some("error"));
        assert_ne!(text.message_id, error.message_id, "text and terminal error need independent identities");
        assert_eq!(error.msg_id.as_deref(), Some(error.message_id.as_str()));
        let content: serde_json::Value = serde_json::from_str(&error.content).unwrap();
        assert_eq!(content["turn_id"], TEST_ASSISTANT_MESSAGE_ID);
    }

    #[tokio::test]
    async fn run_tool_call_then_error_is_post_response() {        // Plan D4 (review #4): a turn that forwarded/persisted a ToolCall and
        // THEN hit a provider fault must report `emitted_response == true`, so
        // the failover seam refuses to switch — re-running the turn would
        // re-execute the tool's side effect (and re-bill it).
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-001".into(),
            name: "write_file".into(),
            args: json!({"path": "a.ts"}),
            status: ToolCallStatus::Completed,
            description: None,
            input: None,
            output: Some("ok".into()),
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "provider 503 after tool ran",
            None,
        )))
        .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());
        assert_eq!(outcome.committed_artifact_count, 0);
        // A tool action already ran this turn → NOT pre-response → never failed over.
        assert!(
            outcome.emitted_response,
            "a forwarded ToolCall must mark the turn as having emitted a response"
        );
    }

    #[tokio::test]
    async fn run_marks_active_tool_call_error_when_turn_hits_max_tokens() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};
        use nomifun_ai_agent::protocol::events::TurnStopReason;

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-write".into(),
            name: "Write".into(),
            args: json!({"file_path": "/tmp/index.html"}),
            status: ToolCallStatus::Running,
            description: None,
            input: Some(json!({"file_path": "/tmp/index.html"})),
            output: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: Some(TurnStopReason::MaxTokens),
        }))
        .unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.terminal, RelayTerminal::Finish);

        let tool_id = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("tool call must be persisted")
            .message_id;
        MessageId::parse(&tool_id).expect("tool row has a canonical message ID");
        let updates = repo.take_updates();
        let (_, update) = updates
            .iter()
            .find(|(id, _)| id == &tool_id)
            .expect("active tool call should be marked failed when the turn is truncated");
        assert_eq!(update.status.as_ref().map(|v| v.as_deref()), Some(Some("error")));
        let content: serde_json::Value = serde_json::from_str(update.content.as_deref().expect("updated content")).unwrap();
        assert_eq!(content["status"], "error");
        assert_eq!(content["output"], "The turn ended before this tool completed: max_tokens");
    }

    #[tokio::test]
    async fn run_scopes_tool_message_identity_to_the_turn() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        for turn_id in [TEST_TURN_A, TEST_TURN_B] {
            let bus = Arc::new(TestUserEventBus::new(64));
            let (tx, _) = broadcast::channel(64);
            let relay = StreamRelay::new(
                test_conversation_id(),
                turn_id.into(),
                TEST_USER_ID.into(),
                repo.clone(),
                bus,
                None,
            );
            let rx = tx.subscribe();
            tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: "provider-call-1".into(),
                name: "Read".into(),
                args: json!({"path": "a.txt"}),
                status: ToolCallStatus::Completed,
                input: None,
                output: Some("ok".into()),
                description: None,
                artifacts: Vec::new(),
                retry: None,
            }))
            .unwrap();
            tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
            relay.consume(rx).await;
        }

        let inserts = repo.take_inserts();
        let ids: Vec<_> = inserts
            .iter()
            .filter(|row| row.r#type == "tool_call")
            .map(|row| row.message_id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| MessageId::parse(*id).is_ok()));
        assert_ne!(ids[0], ids[1], "the same provider call key is scoped by turn");
        let turns: Vec<_> = inserts
            .iter()
            .filter(|row| row.r#type == "tool_call")
            .map(|row| serde_json::from_str::<serde_json::Value>(&row.content).unwrap()["turn_id"].clone())
            .collect();
        assert_eq!(turns, [json!(TEST_TURN_A), json!(TEST_TURN_B)]);
    }

    #[tokio::test]
    async fn run_does_not_regress_a_terminal_tool_to_running() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        let event = |status, output| {
            AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: "provider-call-1".into(),
                name: "Bash".into(),
                args: json!({"command": "true"}),
                status,
                input: None,
                output,
                description: None,
                artifacts: Vec::new(),
                retry: None,
            })
        };
        tx.send(event(ToolCallStatus::Completed, Some("ok".into()))).unwrap();
        tx.send(event(ToolCallStatus::Running, None)).unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let updates = repo.take_updates();
        assert!(
            updates.iter().all(|(_, update)| update.status.as_ref().map(|s| s.as_deref()) != Some(Some("work"))),
            "a late running event must not overwrite a terminal tool"
        );
        assert!(
            updates.iter().all(|(_, update)| update.status.as_ref().map(|s| s.as_deref()) != Some(Some("error"))),
            "a late running event must not reactivate the tool for terminal cleanup"
        );
    }

    #[tokio::test]
    async fn run_does_not_forward_late_completed_artifact_after_tool_error() {
        use nomifun_ai_agent::artifact_store::{ArtifactKind, PersistedArtifact};
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        let event = |status, artifacts| {
            AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: "provider-call-1".into(),
                name: "ImageGeneration".into(),
                args: json!({"prompt": "cat"}),
                status,
                input: None,
                output: None,
                description: None,
                artifacts,
                retry: None,
            })
        };
        tx.send(event(ToolCallStatus::Error, Vec::new())).unwrap();
        tx.send(event(
            ToolCallStatus::Completed,
            vec![PersistedArtifact {
                id: PersistedArtifactId::new().into_string(),
                kind: ArtifactKind::Image,
                mime_type: "image/png".into(),
                path: "/workspace/old.png".into(),
                relative_path: "nomifun-artifacts/old.png".into(),
                size_bytes: 10,
                sha256: "a".repeat(64),
            }],
        ))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let rows = repo.take_inserts();
        let row = rows
            .iter()
            .find(|row| row.r#type == "tool_call")
            .expect("failed tool call is persisted");
        assert_eq!(row.status.as_deref(), Some("error"));
        let content: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content["artifacts"], json!([]));

        let mut tool_events = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" && event.data["type"] == "tool_call" {
                tool_events.push(event.data);
            }
        }
        assert_eq!(tool_events.len(), 1, "late terminal success must not reach live UI");
        assert_eq!(tool_events[0]["data"]["status"], "error");
    }

    #[tokio::test]
    async fn artifact_recovery_same_process_next_turn_rolls_back_unenveloped_receipt() {
        let conversation_id = test_conversation_id();
        let old_wire_id = MessageId::new().into_string();
        let new_wire_id = MessageId::new().into_string();
        let repo = Arc::new(RecordingRepo::new());
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-recovery-handoff-unprepared-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = ArtifactStore::new(&workspace);
        let source = ArtifactRecoverySource {
            conversation_id: conversation_id.clone(),
            wire_msg_id: old_wire_id.clone(),
        };
        let artifact = store
            .persist_inline_and_existing_batch_recoverable(
                [(nomifun_ai_agent::artifact_store::ArtifactKind::Image, "image/png", ONE_PIXEL_PNG)],
                std::iter::empty::<&std::path::Path>(),
                &source,
            )
            .unwrap()
            .pop()
            .unwrap();
        let artifact_path = PathBuf::from(&artifact.path);

        let (old_tx, _) = broadcast::channel(8);
        let old_rx = old_tx.subscribe();
        old_tx
            .send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        StreamRelay::new(
            conversation_id.clone(),
            old_wire_id,
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        )
        .with_root_turn_id(TEST_TURN_A)
        .with_artifact_workspace(workspace.clone())
        .consume(old_rx)
        .await;
        assert!(artifact_path.is_file());

        let (new_tx, _) = broadcast::channel(8);
        let new_rx = new_tx.subscribe();
        new_tx
            .send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        StreamRelay::new(
            conversation_id,
            new_wire_id,
            TEST_USER_ID.into(),
            repo,
            Arc::new(TestUserEventBus::new(8)),
            None,
        )
        .with_root_turn_id(TEST_TURN_B)
        .with_artifact_workspace(workspace.clone())
        .consume(new_rx)
        .await;

        assert!(!artifact_path.exists());
        assert!(store.recovery_records().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn artifact_recovery_same_process_next_turn_reconciles_needs_state() {
        let conversation_id = test_conversation_id();
        let old_wire_id = MessageId::new().into_string();
        let new_wire_id = MessageId::new().into_string();
        let repo = Arc::new(RecordingRepo::new());
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-recovery-handoff-needs-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = ArtifactStore::new(&workspace);
        let source = ArtifactRecoverySource {
            conversation_id: conversation_id.clone(),
            wire_msg_id: old_wire_id.clone(),
        };
        let artifact = store
            .persist_inline_and_existing_batch_recoverable(
                [(nomifun_ai_agent::artifact_store::ArtifactKind::Image, "image/png", ONE_PIXEL_PNG)],
                std::iter::empty::<&std::path::Path>(),
                &source,
            )
            .unwrap()
            .pop()
            .unwrap();
        let committed_content = json!({
            "call_id": "recovered-image",
            "name": "image_gen",
            "status": "completed",
            "artifacts": [artifact.clone()],
            "turn_id": TEST_TURN_A,
            "artifact_delivery_committed": true,
        })
        .to_string();
        let envelope = ArtifactRecoveryEnvelope {
            conversation_id: conversation_id.clone(),
            wire_msg_id: old_wire_id.clone(),
            event_kind: "tool_call".to_owned(),
            event_json: committed_content.clone(),
        };
        store
            .prepare_recovery_receipts(std::slice::from_ref(&artifact), &envelope)
            .unwrap();
        store
            .claim_recovery_receipts(
                std::slice::from_ref(&artifact),
                &ArtifactRecoveryOwner {
                    conversation_id: conversation_id.clone(),
                    wire_msg_id: old_wire_id.clone(),
                    root_turn_id: TEST_TURN_A.to_owned(),
                    message_id: TEST_ASSISTANT_MESSAGE_ID.to_owned(),
                    message_type: "tool_call".to_owned(),
                    committed_content,
                },
            )
            .unwrap();
        store
            .mark_recovery_receipts_commit_attempting(std::slice::from_ref(&artifact))
            .unwrap();
        store
            .mark_recovery_receipts_needs_reconcile(std::slice::from_ref(&artifact))
            .unwrap();

        let (old_tx, _) = broadcast::channel(8);
        let old_rx = old_tx.subscribe();
        old_tx
            .send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        StreamRelay::new(
            conversation_id.clone(),
            old_wire_id,
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        )
        .with_root_turn_id(TEST_TURN_A)
        .with_artifact_workspace(workspace.clone())
        .consume(old_rx)
        .await;

        let (new_tx, _) = broadcast::channel(8);
        let new_rx = new_tx.subscribe();
        new_tx
            .send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        StreamRelay::new(
            conversation_id,
            new_wire_id,
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(8)),
            None,
        )
        .with_root_turn_id(TEST_TURN_B)
        .with_artifact_workspace(workspace.clone())
        .consume(new_rx)
        .await;

        assert!(PathBuf::from(&artifact.path).is_file());
        assert!(store.recovery_records().unwrap().is_empty());
        assert!(repo.artifact_commit_attempts() >= 1);
        assert!(repo
            .take_inserts()
            .iter()
            .any(|row| row.message_id == TEST_ASSISTANT_MESSAGE_ID
                && row.status.as_deref() == Some("finish")));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn run_keeps_completed_artifact_after_successful_turn_finish() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-success".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.committed_artifact_count, 1);

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("artifact tool gets a provisional row");
        assert_eq!(row.status.as_deref(), Some("work"));
        let provisional: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(provisional["status"], "running");
        assert_eq!(provisional["artifacts"], json!([]));
        assert_eq!(provisional[ARTIFACT_DELIVERY_COMMITTED_FIELD], false);

        let updates = repo.take_updates();
        let committed = updates
            .iter()
            .rev()
            .find(|(id, update)| {
                id == &row.message_id
                    && update.status.as_ref().map(|s| s.as_deref()) == Some(Some("finish"))
            })
            .expect("successful enclosing turn promotes the artifact receipt");
        let committed_content: serde_json::Value =
            serde_json::from_str(committed.1.content.as_deref().expect("committed content")).unwrap();
        assert_eq!(committed_content["artifacts"].as_array().map(Vec::len), Some(1));
        assert_eq!(committed_content[ARTIFACT_DELIVERY_COMMITTED_FIELD], true);

        let mut tool_statuses = Vec::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call"
                && let Some(status) = event.data["data"]["status"].as_str()
            {
                tool_statuses.push(status.to_owned());
            }
        }
        assert_eq!(tool_statuses, ["running", "completed"]);
        assert_eq!(stream_types.last(), Some(&json!("finish")));
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn atomic_artifact_commit_failure_rejects_finish_and_leaves_only_provisional_history() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.fail_artifact_commits();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-commit-failure-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        assert!(artifact_path.is_file());
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-commit-fails".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());
        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert_eq!(outcome.committed_artifact_count, 0);

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("phase one persists a fail-closed row");
        assert_eq!(row.status.as_deref(), Some("work"));
        let content: Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content["status"], "running");
        assert_eq!(content["artifacts"], json!([]));
        assert_eq!(content[ARTIFACT_DELIVERY_COMMITTED_FIELD], false);

        let mut tool_statuses = Vec::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call"
                && let Some(status) = event.data["data"]["status"].as_str()
            {
                tool_statuses.push(status.to_owned());
            }
        }
        assert_eq!(tool_statuses, ["running", "error"]);
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
        assert_eq!(repo.artifact_commit_attempts(), 1);
        assert!(
            !artifact_path.exists(),
            "a definitively failed artifact transaction must roll back the provisional snapshot"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn artifact_reverification_failure_rolls_back_before_database_commit() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-reverify-failure-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        std::fs::write(&artifact_path, b"tampered after receipt publication")
            .expect("tamper provisional artifact");
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            Arc::new(TestUserEventBus::new(64)),
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-reverify-fails".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert_eq!(repo.artifact_commit_attempts(), 0);
        assert!(
            !artifact_path.exists(),
            "pre-COMMIT verification failure must roll back the owned provisional snapshot"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn lost_artifact_commit_ack_recovers_exact_durable_rows_without_rollback() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.commit_artifact_rows_then_error();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-lost-ack-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-commit-lost-ack".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(outcome.terminal, RelayTerminal::Finish);
        assert_eq!(outcome.committed_artifact_count, 1);
        assert_eq!(repo.artifact_commit_attempts(), 1);
        assert!(
            artifact_path.is_file(),
            "an exact durable artifact row owns its snapshot even when the COMMIT acknowledgement is lost"
        );
        let mut tool_statuses = Vec::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call"
                && let Some(status) = event.data["data"]["status"].as_str()
            {
                tool_statuses.push(status.to_owned());
            }
        }
        assert_eq!(tool_statuses, ["running", "completed"]);
        assert_eq!(stream_types.last(), Some(&json!("finish")));
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn unknown_artifact_commit_state_retains_provisional_snapshot() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.fail_artifact_commit_with_unknown_reconciliation();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-unknown-commit-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo,
            Arc::new(TestUserEventBus::new(64)),
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-commit-unknown".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert_eq!(outcome.committed_artifact_count, 0);
        assert!(
            artifact_path.is_file(),
            "query-unknown COMMIT ownership must retain the snapshot for recovery"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn partial_artifact_commit_state_retains_every_snapshot() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.commit_first_artifact_row_then_error();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-partial-commit-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let first = persisted_png_artifact(&workspace);
        let second = persisted_png_artifact(&workspace);
        let paths = [PathBuf::from(&first.path), PathBuf::from(&second.path)];
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo,
            Arc::new(TestUserEventBus::new(64)),
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        for (call_id, artifact) in [("artifact-partial-a", first), ("artifact-partial-b", second)] {
            tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: call_id.into(),
                name: "ImageGeneration".into(),
                args: json!({"prompt": "cat"}),
                status: ToolCallStatus::Completed,
                input: None,
                output: Some("generated".into()),
                description: None,
                artifacts: vec![artifact],
                retry: None,
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;

        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );
        assert_eq!(outcome.committed_artifact_count, 0);
        assert!(
            paths.iter().all(|path| path.is_file()),
            "partial durable ownership must retain the entire batch for recovery"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_atomic_artifact_commit_retains_turn_and_exposes_no_terminal_receipt() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.block_artifact_commits();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-stall-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-commit-times-out".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let relay_task = tokio::spawn(relay.consume(rx));
        for _ in 0..128 {
            if repo.artifact_commit_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            repo.artifact_commit_attempts(),
            1,
            "the relay must be blocked at the exact artifact commit cutpoint"
        );
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(
            !relay_task.is_finished(),
            "elapsed wall time must not release a turn with an ambiguous artifact COMMIT"
        );
        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("the pending commit leaves the provisional row intact");
        assert_eq!(row.status.as_deref(), Some("work"));
        let content: Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content["artifacts"], json!([]));

        let mut observed_completed = false;
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            observed_completed |= event.data["type"] == "tool_call"
                && event.data["data"]["status"] == "completed";
        }
        assert!(!observed_completed);
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert!(
            !stream_types.iter().any(|kind| *kind == json!("error")),
            "a timeout must not manufacture a terminal error while COMMIT ownership is ambiguous"
        );
        relay_task.abort();
        let _ = relay_task.await;
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn artifact_delivery_never_uses_random_message_identity_after_correlation_failure() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_correlations();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-without-durable-id".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![test_artifact("identity-failure")],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());
        assert!(repo.take_inserts().iter().all(|row| {
            row.r#type != "tool_call" || row.status.as_deref() != Some("finish")
        }));

        let mut saw_tool_error = false;
        let mut saw_tool_completed = false;
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call" {
                saw_tool_error |= event.data["data"]["status"] == "error";
                saw_tool_completed |= event.data["data"]["status"] == "completed";
            }
        }
        assert!(saw_tool_error);
        assert!(!saw_tool_completed);
        assert!(!stream_types.iter().any(|kind| *kind == json!("finish")));
        assert_eq!(stream_types.last(), Some(&json!("error")));
    }

    #[tokio::test]
    async fn run_retracts_completed_artifact_when_enclosing_turn_errors() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-artifact-terminal-error-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        assert!(artifact_path.is_file());
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-then-error".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "provider failed after artifact delivery",
            None,
        )))
        .unwrap();

        relay.consume(rx).await;

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("completed artifact tool is persisted provisionally");
        assert_eq!(row.status.as_deref(), Some("work"));
        let provisional: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(provisional["artifacts"], json!([]));
        assert_eq!(provisional[ARTIFACT_DELIVERY_COMMITTED_FIELD], false);
        let updates = repo.take_updates();
        let correction = updates
            .iter()
            .rev()
            .find(|(id, _)| id == &row.message_id)
            .expect("global turn error must correct the completed artifact row");
        assert_eq!(
            correction.1.status.as_ref().map(|status| status.as_deref()),
            Some(Some("error"))
        );
        let content: serde_json::Value =
            serde_json::from_str(correction.1.content.as_deref().expect("corrected content")).unwrap();
        assert_eq!(content["status"], "error");
        assert_eq!(content["artifacts"], json!([]));

        let mut last_tool = None;
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
                if event.data["type"] == "tool_call" {
                    last_tool = Some(event.data);
                }
            }
        }
        let last_tool = last_tool.expect("live UI receives the terminal artifact correction");
        assert_eq!(last_tool["data"]["status"], "error");
        assert_eq!(last_tool["data"]["artifacts"], json!([]));
        assert_eq!(
            stream_types.last(),
            Some(&json!("error")),
            "the enclosing terminal must be published after artifact retraction"
        );
        assert!(
            !artifact_path.exists(),
            "an unsuccessful enclosing turn must roll back its provisional snapshot"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_terminal_artifact_correction_withholds_enclosing_terminal() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "artifact-before-wedged-db".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![test_artifact("wedged-db")],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "provider failed after artifact delivery",
            None,
        )))
        .unwrap();
        // The completed row above can be inserted, but its terminal correction
        // now wedges forever. The exact turn owner must remain live instead of
        // converting elapsed wall time into permission to finalize.
        repo.block_message_updates();

        let relay_task = tokio::spawn(relay.consume(rx));
        for _ in 0..128 {
            if repo.message_update_attempts() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            repo.message_update_attempts(),
            1,
            "the relay must be blocked at the exact terminal correction cutpoint"
        );
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert!(
            !relay_task.is_finished(),
            "elapsed wall time must not release terminal cleanup authority"
        );

        let provisional = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_call")
            .expect("the pre-terminal artifact projection is durable");
        assert_eq!(provisional.status.as_deref(), Some("work"));
        let content: serde_json::Value = serde_json::from_str(&provisional.content).unwrap();
        assert_eq!(content["status"], "running");
        assert_eq!(content["artifacts"], json!([]));
        assert_eq!(content[ARTIFACT_DELIVERY_COMMITTED_FIELD], false);

        let mut final_tool_status = None;
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call" {
                final_tool_status = event.data["data"]["status"].as_str().map(str::to_owned);
            }
        }
        assert_eq!(final_tool_status.as_deref(), Some("error"));
        assert!(
            !stream_types.iter().any(|kind| *kind == json!("error")),
            "the enclosing terminal must remain withheld until the durable correction returns"
        );
        relay_task.abort();
        let _ = relay_task.await;
    }

    #[tokio::test]
    async fn run_retracts_completed_acp_artifact_when_enclosing_turn_errors() {
        use nomifun_ai_agent::protocol::events::{
            AcpToolCallContentItem,
            tool_call::{
                AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
                AcpToolCallUpdateData,
            },
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-conversation-acp-artifact-terminal-error-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let artifact_path = PathBuf::from(&artifact.path);
        assert!(artifact_path.is_file());
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "session-artifact".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "acp-artifact-then-error".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: Some("Generate image".into()),
                kind: None,
                raw_input: None,
                raw_output: Some(json!("generated")),
                content: Some(vec![AcpToolCallContentItem::Artifact {
                    artifact,
                    source_uri: None,
                }]),
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "provider failed after ACP artifact delivery",
            None,
        )))
        .unwrap();

        relay.consume(rx).await;

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("completed ACP artifact tool is persisted provisionally");
        assert_eq!(row.status.as_deref(), Some("work"));
        let provisional: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(provisional["update"]["status"], "in_progress");
        assert!(
            provisional["update"]["content"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert_eq!(provisional[ARTIFACT_DELIVERY_COMMITTED_FIELD], false);
        let updates = repo.take_updates();
        let correction = updates
            .iter()
            .rev()
            .find(|(id, _)| id == &row.message_id)
            .expect("global turn error must correct the completed ACP artifact row");
        assert_eq!(
            correction.1.status.as_ref().map(|status| status.as_deref()),
            Some(Some("error"))
        );
        let content: serde_json::Value =
            serde_json::from_str(correction.1.content.as_deref().expect("corrected content")).unwrap();
        assert_eq!(content["update"]["status"], "failed");
        assert!(
            content["update"]["content"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "failed ACP projection must remove artifact/resource-link delivery blocks"
        );

        let mut last_acp = None;
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
                if event.data["type"] == "acp_tool_call" {
                    last_acp = Some(event.data);
                }
            }
        }
        let last_acp = last_acp.expect("live UI receives the terminal ACP artifact correction");
        assert_eq!(last_acp["data"]["update"]["status"], "failed");
        assert!(
            last_acp["data"]["update"]["content"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert_eq!(stream_types.last(), Some(&json!("error")));
        assert!(
            !artifact_path.exists(),
            "an unsuccessful ACP turn must roll back its provisional snapshot"
        );
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn channel_close_retracts_completed_generic_and_acp_artifacts_before_terminal() {
        use nomifun_ai_agent::protocol::events::{
            AcpToolCallContentItem,
            tool_call::{
                AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
                AcpToolCallUpdateData, ToolCallEventData, ToolCallStatus,
            },
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "generic-before-close".into(),
            name: "ImageGeneration".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![test_artifact("generic-close")],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "session-close".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "acp-before-close".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: Some("Generate image".into()),
                kind: None,
                raw_input: None,
                raw_output: Some(json!("generated")),
                content: Some(vec![AcpToolCallContentItem::Artifact {
                    artifact: test_artifact("acp-close"),
                    source_uri: None,
                }]),
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        drop(tx);

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.terminal, RelayTerminal::ChannelClosed);

        let rows = repo.take_inserts();
        let generic_id = rows
            .iter()
            .find(|row| row.r#type == "tool_call")
            .expect("generic artifact row")
            .message_id
            .clone();
        let acp_id = rows
            .iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("ACP artifact row")
            .message_id
            .clone();
        let updates = repo.take_updates();
        for id in [generic_id, acp_id] {
            let update = updates
                .iter()
                .rev()
                .find(|(updated_id, _)| updated_id == &id)
                .expect("closed stream must retract every completed artifact lifecycle");
            assert_eq!(
                update.1.status.as_ref().map(|status| status.as_deref()),
                Some(Some("error"))
            );
        }

        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream" {
                stream_types.push(event.data["type"].clone());
            }
        }
        assert_eq!(stream_types.last(), Some(&json!("error")));
        assert_eq!(
            stream_types
                .iter()
                .filter(|event_type| **event_type == json!("tool_call"))
                .count(),
            2,
            "completed generic tool plus its error correction are both visible"
        );
        assert_eq!(
            stream_types
                .iter()
                .filter(|event_type| **event_type == json!("acp_tool_call"))
                .count(),
            2,
            "completed ACP tool plus its error correction are both visible"
        );
    }

    #[tokio::test]
    async fn generic_artifact_tracking_limit_fails_closed_without_an_untracked_success() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(4096));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(1024);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        for index in 0..=MAX_TERMINAL_ACTIVE_ITEMS {
            tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: format!("artifact-{index}"),
                name: "ImageGeneration".into(),
                args: json!({"prompt": "cat"}),
                status: ToolCallStatus::Completed,
                input: None,
                output: Some("generated".into()),
                description: None,
                artifacts: vec![test_artifact(&format!("artifact-{index}"))],
                retry: None,
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());

        let mut final_statuses = HashMap::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "tool_call"
                && let (Some(call_id), Some(status)) = (
                    event.data["data"]["call_id"].as_str(),
                    event.data["data"]["status"].as_str(),
                )
            {
                final_statuses.insert(call_id.to_owned(), status.to_owned());
            }
        }
        assert_eq!(final_statuses.len(), MAX_TERMINAL_ACTIVE_ITEMS + 1);
        assert!(final_statuses.values().all(|status| status == "error"));
        assert_eq!(stream_types.last(), Some(&json!("error")));

        let rows = repo.take_inserts();
        assert_eq!(
            rows.iter().filter(|row| row.r#type == "tool_call").count(),
            MAX_TERMINAL_ACTIVE_ITEMS + 1
        );
        assert_eq!(
            repo.take_updates()
                .iter()
                .filter(|(_, update)| {
                    update.status.as_ref().map(|status| status.as_deref()) == Some(Some("error"))
                })
                .count(),
            MAX_TERMINAL_ACTIVE_ITEMS
        );
    }

    #[tokio::test]
    async fn acp_artifact_tracking_limit_fails_closed_without_an_untracked_success() {
        use nomifun_ai_agent::protocol::events::{
            AcpToolCallContentItem,
            tool_call::{
                AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
                AcpToolCallUpdateData,
            },
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(4096));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(1024);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo,
            bus,
            None,
        )
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();
        for index in 0..=MAX_TERMINAL_ACTIVE_ITEMS {
            tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
                session_id: "session-overflow".into(),
                update: AcpToolCallUpdateData {
                    session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                    tool_call_id: format!("acp-artifact-{index}"),
                    status: Some(AcpToolCallStatus::Completed),
                    title: Some("Generate image".into()),
                    kind: None,
                    raw_input: None,
                    raw_output: Some(json!("generated")),
                    content: Some(vec![AcpToolCallContentItem::Artifact {
                        artifact: test_artifact(&format!("acp-artifact-{index}")),
                        source_uri: None,
                    }]),
                    locations: None,
                },
                meta: None,
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());

        let mut final_statuses = HashMap::new();
        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            if event.data["type"] == "acp_tool_call"
                && let (Some(call_id), Some(status)) = (
                    event.data["data"]["update"]["tool_call_id"].as_str(),
                    event.data["data"]["update"]["status"].as_str(),
                )
            {
                final_statuses.insert(call_id.to_owned(), status.to_owned());
            }
        }
        assert_eq!(final_statuses.len(), MAX_TERMINAL_ACTIVE_ITEMS + 1);
        assert!(final_statuses.values().all(|status| status == "failed"));
        assert_eq!(stream_types.last(), Some(&json!("error")));
    }

    #[tokio::test]
    async fn persistence_does_not_regress_a_terminal_tool_after_relay_restart() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        for status in [ToolCallStatus::Completed, ToolCallStatus::Running] {
            let bus = Arc::new(TestUserEventBus::new(64));
            let (tx, _) = broadcast::channel(64);
            let relay = StreamRelay::new(
                test_conversation_id(),
                TEST_TURN_A.into(),
                TEST_USER_ID.into(),
                repo.clone(),
                bus,
                None,
            );
            let rx = tx.subscribe();
            tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: "provider-call-1".into(),
                name: "Bash".into(),
                args: json!({"command": "true"}),
                status,
                input: None,
                output: (status == ToolCallStatus::Completed).then(|| "ok".into()),
                description: None,
                artifacts: Vec::new(),
                retry: None,
            }))
            .unwrap();
            tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
            relay.consume(rx).await;
        }

        let updates = repo.take_updates();
        assert!(
            updates.iter().all(|(_, update)| update.status.as_ref().map(|s| s.as_deref()) != Some(Some("work"))),
            "stored terminal state must reject a late running update after relay restart"
        );
    }

    #[tokio::test]
    async fn run_suppresses_pre_response_error_when_failover_will_fire() {
        // review #1/#5: with a suppressor that accepts the fault's code, a
        // pre-response (no text) provider error must NOT broadcast a WS error
        // event NOR persist an error `tips` row — the user only ever sees the
        // backup model's turn. The swallowed event is handed back for re-surface.
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        // Always-suppress predicate (the send loop passes is_provider_fault).
        .with_failover_suppressor(Arc::new(|_code| true));

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "provider 503 pre-response",
            Some(nomifun_api_types::AgentErrorCode::UserLlmProviderGatewayError),
        )))
        .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());
        // No error tips row persisted.
        let inserts = repo.take_inserts();
        assert!(
            !inserts.iter().any(|m| m.r#type == "tips"),
            "a suppressed pre-response error must not persist a tips row"
        );
        // No WS error event broadcast.
        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        assert!(
            !ws_events
                .iter()
                .any(|evt| evt.name == "message.stream" && evt.data["type"] == "error"),
            "a suppressed pre-response error must not broadcast a WS error event"
        );
        // The swallowed event is handed back so the loop can re-surface on a miss.
        assert!(matches!(outcome.suppressed_error, Some(AgentStreamEvent::Error(_))));
    }

    #[tokio::test]
    async fn run_does_not_suppress_when_response_already_emitted() {
        // The suppressor must NOT fire post-response: a Text then a fault keeps
        // the error visible (failover would duplicate the streamed text).
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_failover_suppressor(Arc::new(|_code| true));

        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "partial".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy("fault after text", None)))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.emitted_response);
        assert!(
            outcome.suppressed_error.is_none(),
            "a post-response fault must never be suppressed"
        );
    }

    #[tokio::test]
    async fn run_send_error_injects_error_and_completes_turn() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion();

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        let (send_error_tx, send_error_rx) = tokio::sync::oneshot::channel();
        send_error_tx
            .send(Err(AgentSendError::from_app_error(nomifun_common::AppError::BadGateway(
                "provider returned 401 invalid api key".into(),
            ))))
            .unwrap();

        let outcome = relay.consume_with_send_error(rx, send_error_rx).await;
        assert!(outcome.system_responses.is_empty());
        assert_eq!(
            outcome.terminal,
            RelayTerminal::Error {
                code: Some(nomifun_api_types::AgentErrorCode::UserLlmProviderAuthFailed),
                retryable: Some(false)
            }
        );

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].r#type, "tips");
        assert_eq!(inserts[0].status.as_deref(), Some("error"));
        let content: serde_json::Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(content["content"], "The model provider rejected the request");
        assert_eq!(content["type"], "error");
        assert_eq!(content["error"]["code"], "USER_LLM_PROVIDER_AUTH_FAILED");
        assert_eq!(content["error"]["ownership"], "user_llm_provider");
        assert_eq!(content["error"]["retryable"], false);
        assert_eq!(content["error"]["feedback_recommended"], false);
        assert_eq!(content["error"]["detail"], "provider returned 401 invalid api key");
        assert_eq!(content["error"]["resolution"]["kind"], "check_provider_credentials");
        assert_eq!(content["error"]["resolution"]["target"], "provider_settings");

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }

        let error_event = ws_events
            .iter()
            .find(|evt| evt.name == "message.stream" && evt.data["type"] == "error")
            .expect("send error should be forwarded as message.stream error");
        assert_eq!(error_event.data["data"]["code"], "USER_LLM_PROVIDER_AUTH_FAILED");
        assert_eq!(error_event.data["data"]["ownership"], "user_llm_provider");
        assert!(ws_events.iter().any(|evt| evt.name == "turn.completed"));
    }

    #[tokio::test]
    async fn run_send_error_keeps_existing_stream_error_when_it_arrives_first() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();
        let send_error = AgentSendError::from_app_error(nomifun_common::AppError::BadGateway(
            "provider returned 401 invalid api key".into(),
        ));
        tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
            "stream already emitted",
            None,
        )))
        .unwrap();
        let (send_error_tx, send_error_rx) = tokio::sync::oneshot::channel();
        let delayed_send_error = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = send_error_tx.send(Err(send_error));
        });

        let outcome = relay.consume_with_send_error(rx, send_error_rx).await;
        delayed_send_error.await.unwrap();
        assert!(outcome.system_responses.is_empty());

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].r#type, "tips");
        let content: serde_json::Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(content["content"], "stream already emitted");
        assert_eq!(content["type"], "error");
    }

    #[tokio::test]
    async fn run_send_error_uses_send_error_when_it_arrives_first() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();
        let (send_error_tx, send_error_rx) = tokio::sync::oneshot::channel();
        send_error_tx
            .send(Err(AgentSendError::from_app_error(nomifun_common::AppError::BadGateway(
                "provider returned 401 invalid api key".into(),
            ))))
            .unwrap();
        let delayed_stream_error = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = tx.send(AgentStreamEvent::Error(ErrorEventData::legacy(
                "stream already emitted",
                None,
            )));
        });

        let outcome = relay.consume_with_send_error(rx, send_error_rx).await;
        delayed_stream_error.await.unwrap();
        assert!(outcome.system_responses.is_empty());
        assert_eq!(
            outcome.terminal,
            RelayTerminal::Error {
                code: Some(nomifun_api_types::AgentErrorCode::UserLlmProviderAuthFailed),
                retryable: Some(false)
            }
        );

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].r#type, "tips");
        let content: serde_json::Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(content["content"], "The model provider rejected the request");
        assert_eq!(content["type"], "error");
        assert_eq!(content["error"]["resolution"]["kind"], "check_provider_credentials");
        assert_eq!(content["error"]["resolution"]["target"], "provider_settings");
    }

    #[tokio::test]
    async fn closed_send_task_signal_is_a_bounded_terminal_error() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        let (send_tx, send_rx) = tokio::sync::oneshot::channel::<Result<(), AgentSendError>>();
        drop(send_tx);

        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            relay.consume_with_send_error(rx, send_rx),
        )
        .await
        .expect("closed send task signal must not leave the relay waiting");
        assert!(outcome.terminal.is_error());
        let inserts = repo.take_inserts();
        let tips = inserts
            .iter()
            .find(|row| row.r#type == "tips")
            .expect("abnormal send task exit must be persisted as an error");
        assert!(tips.content.contains("exited before reporting acceptance"));
    }

    #[tokio::test]
    async fn run_thinking_tool_thinking_splits_thinking_segments() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: "Plan A".into(),
            subject: None,
            duration: None,
            status: Some("thinking".into()),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-001".into(),
            name: "read_file".into(),
            args: json!({"path": "a.ts"}),
            status: ToolCallStatus::Running,
            description: None,
            input: None,
            output: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: "Plan B".into(),
            subject: None,
            duration: None,
            status: Some("thinking".into()),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let thinking_msgs: Vec<_> = inserts.iter().filter(|msg| msg.r#type == "thinking").collect();
        assert_eq!(thinking_msgs.len(), 2, "thinking should split across tool boundaries");
        assert_ne!(thinking_msgs[0].message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(thinking_msgs[0].msg_id.as_deref(), Some(thinking_msgs[0].message_id.as_str()));
        assert_ne!(thinking_msgs[0].msg_id, thinking_msgs[1].msg_id);

        let mut done_msg_ids = Vec::new();
        while let Ok(evt) = ws_rx.try_recv() {
            if evt.name == "message.stream" && evt.data["type"] == "thinking" && evt.data["data"]["status"] == "done" {
                done_msg_ids.push(evt.data["msg_id"].as_str().unwrap_or_default().to_owned());
            }
        }
        assert_eq!(done_msg_ids.len(), 2);
        assert_eq!(done_msg_ids[0], thinking_msgs[0].message_id);
        assert_eq!(done_msg_ids[1], thinking_msgs[1].message_id);
        assert_ne!(done_msg_ids[0], done_msg_ids[1]);
    }

    #[tokio::test]
    async fn run_thinking_then_text_uses_distinct_segment_ids() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: "Plan first".into(),
            subject: None,
            duration: None,
            status: Some("thinking".into()),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Final answer".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let thinking_msgs: Vec<_> = inserts.iter().filter(|msg| msg.r#type == "thinking").collect();
        let text_msgs: Vec<_> = inserts.iter().filter(|msg| msg.r#type == "text").collect();

        assert_eq!(thinking_msgs.len(), 1);
        assert_eq!(text_msgs.len(), 1);
        assert_ne!(thinking_msgs[0].message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_ne!(text_msgs[0].message_id, TEST_ASSISTANT_MESSAGE_ID);
        assert_ne!(thinking_msgs[0].message_id, text_msgs[0].message_id);

        let mut text_msg_ids = Vec::new();
        let mut thinking_done_ids = Vec::new();
        while let Ok(evt) = ws_rx.try_recv() {
            if evt.name != "message.stream" {
                continue;
            }
            if evt.data["type"] == "text" || evt.data["type"] == "content" {
                text_msg_ids.push(evt.data["msg_id"].as_str().unwrap_or_default().to_owned());
            }
            if evt.data["type"] == "thinking" && evt.data["data"]["status"] == "done" {
                thinking_done_ids.push(evt.data["msg_id"].as_str().unwrap_or_default().to_owned());
            }
        }

        assert_eq!(thinking_done_ids, vec![thinking_msgs[0].message_id.clone()]);
        assert_eq!(text_msg_ids.len(), 1);
        assert_ne!(text_msg_ids[0], TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(
            outcome.final_text_msg_id.as_deref(),
            Some(text_msg_ids[0].as_str()),
            "turn-final post-processing should target the final assistant text segment, not the thinking segment"
        );
    }

    #[tokio::test]
    async fn run_channel_closed_finalizes() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        // Send text then drop sender (channel closes without Finish)
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "partial".into(),
        }))
        .unwrap();
        drop(tx);

        let outcome = relay.consume(rx).await;
        assert!(outcome.system_responses.is_empty());

        // Preserve both pieces of terminal evidence: the partial assistant
        // text and a first-class canonical error row for the broken channel.
        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 2);
        let text = inserts.iter().find(|row| row.r#type == "text").expect("partial text row");
        let error = inserts.iter().find(|row| row.r#type == "tips").expect("channel error row");
        assert_eq!(text.status.as_deref(), Some("error"));
        assert_eq!(error.status.as_deref(), Some("error"));
        let text_content: serde_json::Value = serde_json::from_str(&text.content).unwrap();
        assert_eq!(text_content["content"], "partial");
        assert_eq!(error.msg_id.as_deref(), Some(error.message_id.as_str()));
        let mut ws_events = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            ws_events.push(event);
        }
        let live_error = ws_events
            .iter()
            .find(|event| event.name == "message.stream" && event.data["type"] == "error")
            .expect("unexpected channel closure must be visible as a terminal error");
        assert_eq!(live_error.data["msg_id"], error.message_id);
    }

    #[tokio::test]
    async fn test_only_completion_opt_in_broadcasts_turn_completed() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let conversation_id = test_conversation_id();
        let relay = StreamRelay::new(
            conversation_id.clone(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion();

        // Subscribe to the bus before relay runs
        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.system_responses.is_empty());

        // Collect WebSocket events
        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }

        // Should have turn.completed event
        let turn_event = ws_events.iter().find(|e| e.name == "turn.completed");
        assert!(turn_event.is_some());
        let data = &turn_event.unwrap().data;
        assert_eq!(data["conversation_id"], conversation_id);
        assert_eq!(data["turn_id"], TEST_ASSISTANT_MESSAGE_ID);
        assert_eq!(data["status"], "finished");
        assert_eq!(data["can_send_message"], true);
    }

    #[tokio::test]
    async fn completion_event_requires_a_durable_finished_commit() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_conversation_updates();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let conversation_id = test_conversation_id();

        StreamRelay::complete_conversation_with_context(
            &(repo as Arc<dyn IConversationRepository>),
            &(bus as Arc<dyn UserEventSink>),
            TEST_USER_ID,
            &conversation_id,
            Some(TEST_ASSISTANT_MESSAGE_ID.to_owned()),
            None,
            false,
            None,
            None,
            None,
        )
        .await;

        assert!(
            ws_rx.try_recv().is_err(),
            "turn.completed must not be published when durable Finished persistence failed"
        );
    }

    #[tokio::test]
    async fn cancellation_token_injects_terminal_finish_without_backend_ack() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (_tx, rx) = broadcast::channel(64);
        let runtime_state = Arc::new(ConversationRuntimeStateService::default());
        let turn_handle = runtime_state
            .try_acquire_turn_with_wire_id(
                &test_conversation_id(),
                Some(TEST_ASSISTANT_MESSAGE_ID.to_owned()),
            )
            .expect("turn handle");
        let cancellation = turn_handle.turn_cancellation();
        cancellation.cancel();

        let mut ws_rx = bus.subscribe();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo,
            bus,
            None,
        )
        .with_cancellation(cancellation);

        let outcome = tokio::time::timeout(Duration::from_millis(250), relay.consume(rx))
            .await
            .expect("cancelled relay must not wait for the backend channel");
        assert_eq!(outcome.terminal, RelayTerminal::Finish);

        let mut ws_events = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            ws_events.push(event);
        }
        let finish = ws_events
            .iter()
            .find(|event| event.name == "message.stream" && event.data["type"] == "finish")
            .expect("cancel must surface a terminal stream event");
        assert_eq!(finish.data["data"]["stop_reason"], "cancelled");
        assert!(
            ws_events
                .iter()
                .all(|event| event.name != "turn.completed"),
            "default relay must leave durable completion to the service lifecycle owner"
        );
    }

    #[tokio::test]
    async fn cancellation_marks_streamed_partial_text_as_error() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, rx) = broadcast::channel(64);
        let runtime_state = Arc::new(ConversationRuntimeStateService::default());
        let turn_handle = runtime_state
            .try_acquire_turn_with_wire_id(
                &test_conversation_id(),
                Some(TEST_ASSISTANT_MESSAGE_ID.to_owned()),
            )
            .expect("turn handle");
        let cancellation = turn_handle.turn_cancellation();
        let mut ws_rx = bus.subscribe();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_cancellation(cancellation.clone());
        let relay_task = tokio::spawn(relay.consume(rx));
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "partial before stop".into(),
        }))
        .unwrap();
        tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                let event = ws_rx.recv().await.expect("stream event");
                if event.name == "message.stream" && event.data["type"] == "content" {
                    break;
                }
            }
        })
        .await
        .expect("partial text reached relay");
        cancellation.cancel();
        relay_task.await.expect("relay task");

        let text = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "text")
            .expect("partial text persisted");
        assert_eq!(text.status.as_deref(), Some("error"));
    }

    #[tokio::test]
    async fn fallback_cancel_winner_suppresses_late_ordinary_terminal() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, rx) = broadcast::channel(64);
        let runtime_state = Arc::new(ConversationRuntimeStateService::default());
        let turn_handle = runtime_state
            .try_acquire_turn_with_wire_id(
                &test_conversation_id(),
                Some(TEST_ASSISTANT_MESSAGE_ID.to_owned()),
            )
            .expect("turn handle");
        let cancellation = turn_handle.turn_cancellation();
        let mut ws_rx = bus.subscribe();

        let fallback = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );
        assert!(fallback.surface_cancelled_turn(&cancellation));

        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo,
            bus,
            None,
        )
        .with_cancellation(cancellation);
        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.stop_reason, Some(TurnStopReason::Cancelled));

        let mut terminal_count = 0;
        while let Ok(event) = ws_rx.try_recv() {
            if event.name == "message.stream"
                && matches!(event.data["type"].as_str(), Some("finish" | "error"))
            {
                terminal_count += 1;
            }
        }
        assert_eq!(terminal_count, 1, "one wire segment has one terminal publisher");
    }

    #[tokio::test]
    async fn run_with_companion_context_stamps_markers_on_stream_and_turn() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion()
        .with_companion_context(
            true,
            Some(
                CompanionId::parse("0190f5fe-7c00-7a00-8abc-012345678942")
                    .unwrap(),
            ),
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "喵".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events
            .iter()
            .find(|e| e.name == "message.stream")
            .expect("stream event broadcast");
        assert_eq!(stream_evt.data["companion"], true);
        assert_eq!(
            stream_evt.data["companion_id"],
            "0190f5fe-7c00-7a00-8abc-012345678942"
        );
        let turn_evt = ws_events
            .iter()
            .find(|e| e.name == "turn.completed")
            .expect("turn.completed broadcast");
        assert_eq!(turn_evt.data["companion"], true);
        assert_eq!(
            turn_evt.data["companion_id"],
            "0190f5fe-7c00-7a00-8abc-012345678942"
        );
    }

    #[tokio::test]
    async fn run_with_channel_platform_stamps_platform_on_stream_and_turn() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            "3".into(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion()
        .with_companion_context(
            true,
            Some(
                CompanionId::parse("0190f5fe-7c00-7a00-8abc-012345678942")
                    .unwrap(),
            ),
        )
        .with_channel_platform(Some("telegram".into()));

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "喵".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events
            .iter()
            .find(|e| e.name == "message.stream")
            .expect("stream event broadcast");
        assert_eq!(stream_evt.data["channel_platform"], "telegram");
        let turn_evt = ws_events
            .iter()
            .find(|e| e.name == "turn.completed")
            .expect("turn.completed broadcast");
        assert_eq!(turn_evt.data["channel_platform"], "telegram");
    }

    #[tokio::test]
    async fn run_with_blank_channel_platform_normalizes_to_null() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_channel_platform(Some("   ".into()));

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "hi".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events.iter().find(|e| e.name == "message.stream").unwrap();
        assert!(stream_evt.data["channel_platform"].is_null());
    }

    #[tokio::test]
    async fn run_without_companion_context_keeps_markers_off() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion();

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "hi".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events.iter().find(|e| e.name == "message.stream").unwrap();
        assert_eq!(stream_evt.data["companion"], false);
        assert!(stream_evt.data["companion_id"].is_null());
        assert!(stream_evt.data["channel_platform"].is_null());
        let turn_evt = ws_events.iter().find(|e| e.name == "turn.completed").unwrap();
        assert_eq!(turn_evt.data["companion"], false);
        assert!(turn_evt.data["companion_id"].is_null());
        assert!(turn_evt.data["channel_platform"].is_null());
    }

    // ── Robot-session stage-direction guard tests ─────────────────

    /// Every `type == "content"` fragment the relay broadcast, concatenated.
    fn streamed_content(events: &[WebSocketMessage<Value>]) -> String {
        events
            .iter()
            .filter(|e| e.name == "message.stream")
            .filter(|e| e.data["type"] == "content")
            .filter_map(|e| e.data["data"]["content"].as_str())
            .collect()
    }

    /// The `$.content` of every persisted `text` row, in insertion order.
    fn persisted_text(inserts: &[MessageRow]) -> Vec<String> {
        inserts
            .iter()
            .filter(|row| row.r#type == "text")
            .map(|row| {
                serde_json::from_str::<Value>(&row.content).unwrap()["content"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    /// The deltas a robot turn actually produces: the bare bracketed name the
    /// model really emits, split across a token boundary — exactly the case a
    /// per-delta strip cannot handle.
    const ROBOT_STAGE_DIRECTION_DELTAS: [&str; 3] = ["[wink", "ing]你好，", "[laughs]再见。"];

    #[tokio::test]
    async fn robot_session_strips_stage_directions_from_stream_and_row() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_robot_session(true);

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        for delta in ROBOT_STAGE_DIRECTION_DELTAS {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: delta.into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        assert_eq!(
            streamed_content(&ws_events),
            "你好，再见。",
            "the live stream never carries a stage direction, split across deltas or not"
        );
        assert!(
            !ws_events
                .iter()
                .any(|e| e.name == "message.stream" && e.data["replace"] == true),
            "the strip happens per delta, so no end-of-turn rewrite flickers the bubble"
        );
        assert_eq!(persisted_text(&repo.take_inserts()), vec!["你好，再见。"]);
    }

    /// Real bracketed content is not a stage direction, in a robot thread as much
    /// as anywhere else. `[附录2]` is the case the user named: the guard exists so
    /// the transcript shows normal content, and deleting a footnote reference
    /// would be the same bug in the other direction.
    #[tokio::test]
    async fn robot_session_keeps_real_bracketed_content() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_robot_session(true);

        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "见附录[1]和[附录2]".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        assert_eq!(persisted_text(&repo.take_inserts()), vec!["见附录[1]和[附录2]"]);
    }

    /// The blast-radius test: the exact same stream through an ordinary
    /// conversation must be byte-identical. Every other conversation kind —
    /// chat, customer service, channels, ACP transcripts — takes this path.
    #[tokio::test]
    async fn non_robot_session_preserves_stage_directions() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        for delta in ROBOT_STAGE_DIRECTION_DELTAS {
            tx.send(AgentStreamEvent::Text(TextEventData {
                content: delta.into(),
            }))
            .unwrap();
        }
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let raw = ROBOT_STAGE_DIRECTION_DELTAS.concat();
        assert_eq!(
            streamed_content(&ws_events),
            raw,
            "without the gate the relay is a byte-for-byte pass-through"
        );
        assert_eq!(persisted_text(&repo.take_inserts()), vec![raw]);
    }

    /// A withheld `[wink` is text, not a stage direction, once the text run ends.
    /// Pins the `release_withheld_text` site in the non-`Text` branch of the
    /// rewrite.
    #[tokio::test]
    async fn robot_session_releases_truncated_bracket_before_tool_call() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_robot_session(true);

        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "hi[wink".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-stage".into(),
            name: "robot_look".into(),
            args: json!({"yaw": 0}),
            status: ToolCallStatus::Running,
            input: Some(json!({"yaw": 0})),
            output: None,
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        assert_eq!(
            persisted_text(&repo.take_inserts()),
            vec!["hi[wink"],
            "a bracket that never closed is literal text and must not be dropped"
        );
    }

    /// A robot turn interleaves text with `robot_*` tool calls, so it has more
    /// than one text segment. `finalize`'s middleware-rewrite branch collapses
    /// segment[0] and hides the rest; because the strip already happened per
    /// delta, `processed.message == text` and that branch stays dormant.
    #[tokio::test]
    async fn robot_session_does_not_collapse_multi_segment_turn() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_robot_session(true);

        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "[thinking]我看看。".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-turn".into(),
            name: "robot_look".into(),
            args: json!({"yaw": 30}),
            status: ToolCallStatus::Completed,
            input: Some(json!({"yaw": 30})),
            output: Some("ok".into()),
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "【happy】转好了。".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        assert_eq!(
            persisted_text(&inserts),
            vec!["我看看。", "转好了。"],
            "both narration segments survive as their own clean rows"
        );
        assert!(
            inserts.iter().filter(|row| row.r#type == "text").all(|row| !row.hidden),
            "neither text row is hidden"
        );
        assert!(
            repo.take_updates()
                .iter()
                .all(|(_, update)| update.hidden != Some(true)),
            "the finalize collapse branch must stay dormant"
        );
    }

    #[tokio::test]
    async fn run_with_origin_stamps_origin_on_stream_and_turn() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion()
        .with_origin(Some("companion".into()));

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "正在创建报表任务".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events
            .iter()
            .find(|e| e.name == "message.stream")
            .expect("stream event broadcast");
        assert_eq!(stream_evt.data["origin"], "companion");
        let turn_evt = ws_events
            .iter()
            .find(|e| e.name == "turn.completed")
            .expect("turn.completed broadcast");
        assert_eq!(turn_evt.data["origin"], "companion");
    }

    #[tokio::test]
    async fn run_without_origin_keeps_origin_null_and_blank_normalizes() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        // Blank origin must normalize to None (owner speech).
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        )
        .with_test_turn_completion()
        .with_origin(Some("   ".into()));

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData { content: "hi".into() }))
            .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();
        relay.consume(rx).await;

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }
        let stream_evt = ws_events.iter().find(|e| e.name == "message.stream").unwrap();
        assert!(stream_evt.data["origin"].is_null());
        let turn_evt = ws_events.iter().find(|e| e.name == "turn.completed").unwrap();
        assert!(turn_evt.data["origin"].is_null());
    }

    #[tokio::test]
    async fn run_finalizes_with_cleaned_replacement_event() {
        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            Some(Arc::new(MockCronService)),
        );

        let mut ws_rx = bus.subscribe();
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Hello [CRON_LIST]".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(outcome.system_responses, vec!["[System: listed]".to_string()]);

        let inserts = repo.take_inserts();
        assert_eq!(inserts.len(), 1);
        let text_message_id = &inserts[0].message_id;
        assert_ne!(text_message_id, TEST_ASSISTANT_MESSAGE_ID);
        let updates = repo.take_updates();
        let final_update = updates
            .iter()
            .find(|(id, update)| id == text_message_id && update.content.is_some())
            .expect("expected cleaned final text update");
        let content: serde_json::Value = serde_json::from_str(final_update.1.content.as_deref().unwrap()).unwrap();
        assert_eq!(content["content"].as_str().map(str::trim), Some("Hello"));

        let mut ws_events = vec![];
        while let Ok(evt) = ws_rx.try_recv() {
            ws_events.push(evt);
        }

        let replacement = ws_events
            .iter()
            .find(|evt| evt.name == "message.stream" && evt.data["type"] == "content" && evt.data["replace"] == true);
        assert!(replacement.is_some());
        assert_eq!(
            replacement.unwrap().data["data"]["content"].as_str().map(str::trim),
            Some("Hello")
        );
    }

    #[tokio::test]
    async fn failed_final_rewrite_emits_no_unacknowledged_override_or_outcome() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_next_message_update();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            Some(Arc::new(MockCronService)),
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Hello [CRON_LIST]".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert!(outcome.final_text.is_none());
        assert!(outcome.final_text_msg_id.is_none());
        assert_eq!(outcome.system_responses, vec!["[System: listed]".to_string()]);
        assert!(repo.take_updates().is_empty());
        let inserts = repo.take_inserts();
        let raw: Value = serde_json::from_str(&inserts[0].content).unwrap();
        assert_eq!(raw["content"], "Hello [CRON_LIST]");
        assert!(
            std::iter::from_fn(|| ws_rx.try_recv().ok()).all(|event| {
                event.name != "message.stream" || event.data["replace"] != true
            }),
            "live replacement must wait for the database rewrite acknowledgement"
        );
    }

    #[tokio::test]
    async fn failed_superseded_hide_emits_only_acknowledged_overrides() {
        let repo = Arc::new(RecordingRepo::new());
        repo.fail_message_update_attempt(2);
        let bus = Arc::new(TestUserEventBus::new(128));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(128);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            Some(Arc::new(MockCronService)),
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Alpha ".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: String::new(),
            subject: None,
            duration: None,
            status: Some("thinking".into()),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Text(TextEventData {
            content: "Beta [CRON_LIST]".into(),
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        let outcome = relay.consume(rx).await;

        assert!(outcome.final_text.is_none());
        assert!(outcome.final_text_msg_id.is_none());
        let inserts = repo.take_inserts();
        let text_rows: Vec<_> = inserts.iter().filter(|row| row.r#type == "text").collect();
        assert_eq!(text_rows.len(), 2);
        let updates = repo.take_updates();
        assert_eq!(updates.len(), 1, "only the acknowledged primary rewrite is recorded");
        assert_eq!(updates[0].0, text_rows[0].message_id);

        let replacements: Vec<_> = std::iter::from_fn(|| ws_rx.try_recv().ok())
            .filter(|event| {
                event.name == "message.stream"
                    && event.data["type"] == "content"
                    && event.data["replace"] == true
            })
            .collect();
        assert_eq!(replacements.len(), 1);
        assert_eq!(replacements[0].data["msg_id"], text_rows[0].message_id);
        assert!(
            replacements
                .iter()
                .all(|event| event.data["msg_id"] != text_rows[1].message_id),
            "a failed hide must remain visible both live and after reload"
        );
    }

    // ── Tool persistence tests ────────────────────────────────────

    #[tokio::test]
    async fn run_tool_call_persists_message() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        // First event: Running with input but no output
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-001".into(),
            name: "read_file".into(),
            args: json!({"path": "notes.txt"}),
            status: ToolCallStatus::Running,
            input: Some(json!({"path": "notes.txt"})),
            output: None,
            description: Some("Read file".into()),
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        // Second event: Completed with output but no input
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "tc-001".into(),
            name: "read_file".into(),
            args: json!({"path": "notes.txt"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("contents".into()),
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let tool_msg = inserts.iter().find(|m| m.r#type == "tool_call");
        assert!(tool_msg.is_some());
        let msg = tool_msg.unwrap();
        MessageId::parse(&msg.message_id).expect("tool row has a canonical message ID");
        assert_eq!(msg.msg_id.as_deref(), Some(TEST_ASSISTANT_MESSAGE_ID));
        assert_eq!(msg.status.as_deref(), Some("work"));

        let updates = repo.take_updates();
        let tool_update = updates.iter().find(|(id, _)| id == &msg.message_id);
        assert!(tool_update.is_some());
        let (_, upd) = tool_update.unwrap();
        assert_eq!(upd.status, Some(Some("finish".to_owned())));

        // Verify merge: input from first event preserved, output from second event added
        let merged: serde_json::Value = serde_json::from_str(upd.content.as_deref().unwrap()).unwrap();
        assert_eq!(merged["name"], "read_file");
        assert_eq!(merged["status"], "completed");
        assert!(
            merged.get("input").is_some() && !merged["input"].is_null(),
            "input must be preserved after merge"
        );
        assert_eq!(merged["input"]["path"], "notes.txt");
        assert_eq!(merged["output"], "contents");
        assert_eq!(merged["description"], "Read file");
    }

    #[tokio::test]
    async fn completed_image_tool_without_receipt_fails_the_enclosing_turn() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallEventData, ToolCallStatus};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "empty-image-result".into(),
            name: "image_gen".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Running,
            input: Some(json!({"prompt": "cat"})),
            output: None,
            description: Some("Generate image".into()),
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "empty-image-result".into(),
            name: "tool_result".into(),
            args: json!({}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("success".into()),
            description: None,
            artifacts: Vec::new(),
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let tool_row = inserts
            .iter()
            .find(|row| row.r#type == "tool_call")
            .expect("failed image result is persisted");
        let updates = repo.take_updates();
        let final_tool_update = updates
            .iter()
            .rev()
            .find(|(id, _)| id == &tool_row.message_id)
            .expect("tool terminal update");
        assert_eq!(final_tool_update.1.status.as_ref().and_then(|s| s.as_deref()), Some("error"));
        let content: serde_json::Value =
            serde_json::from_str(final_tool_update.1.content.as_deref().expect("tool content")).unwrap();
        assert_eq!(content["artifacts"], json!([]));
        assert_eq!(content["status"], "error");

        let mut saw_successful_finish = false;
        while let Ok(event) = ws_rx.try_recv() {
            saw_successful_finish |= event.name == "message.stream" && event.data["type"] == "finish";
        }
        assert!(!saw_successful_finish, "a receipt-less image result must not finish successfully");
    }

    #[tokio::test]
    async fn run_acp_tool_call_inserts_then_updates() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallUpdateData,
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCall,
                tool_call_id: "atc-001".into(),
                status: Some(AcpToolCallStatus::InProgress),
                title: Some("Bash".into()),
                kind: None,
                raw_input: Some(json!({"command": "mv /tmp/a /tmp/b", "description": "Move file"})),
                raw_output: None,
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();

        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "atc-001".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: None,
                kind: None,
                raw_input: None,
                raw_output: Some(json!("Exit code: 0\nSTDOUT:\nSTDERR:")),
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();

        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let acp_msg = inserts.iter().find(|m| m.r#type == "acp_tool_call");
        assert!(acp_msg.is_some());
        let msg = acp_msg.unwrap();
        MessageId::parse(&msg.message_id).expect("ACP tool row has a canonical message ID");
        assert_eq!(msg.msg_id.as_deref(), Some(TEST_ASSISTANT_MESSAGE_ID));
        assert_eq!(msg.status.as_deref(), Some("work"));

        let updates = repo.take_updates();
        let acp_update = updates
            .iter()
            .find(|(id, _)| id == &msg.message_id);
        assert!(acp_update.is_some());
        let (_, upd) = acp_update.unwrap();
        assert_eq!(upd.status, Some(Some("finish".to_owned())));

        // Verify merge: raw_input from ToolCall is preserved, raw_output from ToolCallUpdate is added
        let merged: serde_json::Value = serde_json::from_str(upd.content.as_deref().unwrap()).unwrap();
        let update_obj = merged.get("update").unwrap();
        assert!(
            update_obj.get("raw_input").is_some(),
            "raw_input must be preserved after merge"
        );
        assert_eq!(
            update_obj
                .get("raw_input")
                .unwrap()
                .get("command")
                .unwrap()
                .as_str()
                .unwrap(),
            "mv /tmp/a /tmp/b"
        );
        assert!(
            update_obj.get("raw_output").is_some(),
            "raw_output must be present after merge"
        );
    }

    #[tokio::test]
    async fn external_acp_export_title_cannot_complete_without_a_verified_artifact() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
            AcpToolCallUpdateData,
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "external-session".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCall,
                tool_call_id: "external-export".into(),
                status: Some(AcpToolCallStatus::InProgress),
                title: Some("export_pdf".into()),
                kind: None,
                raw_input: Some(json!({"output_path": "report.pdf"})),
                raw_output: None,
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        // External runtimes commonly omit repeated title/input metadata on the
        // terminal delta. The active identity must remain authoritative.
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "external-session".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "external-export".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: None,
                kind: None,
                raw_input: None,
                raw_output: Some(json!({"ok": true})),
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("external ACP tool row");
        let updates = repo.take_updates();
        let (_, terminal) = updates
            .iter()
            .rev()
            .find(|(id, _)| id == &row.message_id)
            .expect("external ACP terminal correction");
        assert_eq!(
            terminal.status.as_ref().and_then(|status| status.as_deref()),
            Some("error")
        );
        let content: Value =
            serde_json::from_str(terminal.content.as_deref().expect("ACP correction content"))
                .unwrap();
        assert_eq!(content["update"]["status"], "failed");
        assert!(content["update"]["raw_output"]
            .as_str()
            .is_some_and(|message| message.contains("required verified artifacts")));

        let mut saw_finish = false;
        while let Ok(event) = ws_rx.try_recv() {
            saw_finish |= event.name == "message.stream" && event.data["type"] == "finish";
        }
        assert!(!saw_finish);
    }

    #[tokio::test]
    async fn external_acp_duplicate_receipt_cannot_satisfy_requested_image_count() {
        use nomifun_ai_agent::protocol::events::{
            AcpToolCallContentItem,
            tool_call::{
                AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
                AcpToolCallUpdateData,
            },
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        let first = test_artifact("external-duplicate");
        let mut duplicate = first.clone();
        duplicate.id = PersistedArtifactId::new().into_string();
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "external-session".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "external-image-count".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: Some("image_gen".into()),
                kind: None,
                raw_input: Some(json!({"prompt": "two cats", "count": 2})),
                raw_output: Some(json!({"ok": true})),
                content: Some(vec![
                    AcpToolCallContentItem::Artifact {
                        artifact: first,
                        source_uri: None,
                    },
                    AcpToolCallContentItem::Artifact {
                        artifact: duplicate,
                        source_uri: None,
                    },
                ]),
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("failed external ACP count row");
        assert_eq!(row.status.as_deref(), Some("error"));
        let content: Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content["update"]["status"], "failed");
        assert_eq!(content["update"]["content"], json!([]));
        assert!(content["update"]["raw_output"]
            .as_str()
            .is_some_and(|message| message.contains("same canonical artifact path")));

        let mut saw_completed = false;
        let mut saw_finish = false;
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            saw_completed |= event.data["type"] == "acp_tool_call"
                && event.data["data"]["update"]["status"] == "completed";
            saw_finish |= event.data["type"] == "finish";
        }
        assert!(!saw_completed);
        assert!(!saw_finish);
    }

    #[test]
    fn external_acp_receipt_ids_are_validated_without_tool_identity() {
        use nomifun_ai_agent::protocol::events::{
            AcpToolCallContentItem,
            tool_call::{
                AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus,
                AcpToolCallUpdateData,
            },
        };

        let first = test_artifact("identity-free-first");
        let mut duplicate_id = test_artifact("identity-free-second");
        duplicate_id.id = first.id.clone();
        let result = validate_completed_acp_artifact_contract(&AcpToolCallEventData {
            session_id: "external-session".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "identity-free-receipts".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: None,
                kind: None,
                raw_input: None,
                raw_output: None,
                content: Some(vec![
                    AcpToolCallContentItem::Artifact {
                        artifact: first,
                        source_uri: None,
                    },
                    AcpToolCallContentItem::Artifact {
                        artifact: duplicate_id,
                        source_uri: None,
                    },
                ]),
                locations: None,
            },
            meta: None,
        });

        assert!(result.unwrap_err().contains("same artifact id more than once"));
    }

    #[tokio::test]
    async fn run_acp_terminal_update_without_start_is_upserted() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallUpdateData,
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                tool_call_id: "atc-001".into(),
                status: Some(AcpToolCallStatus::Completed),
                title: Some("Bash".into()),
                kind: None,
                raw_input: None,
                raw_output: Some(json!("Exit code: 0")),
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let row = inserts
            .iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("terminal ACP update must survive a missing start event");
        MessageId::parse(&row.message_id).expect("ACP tool row has a canonical message ID");
        assert_eq!(row.status.as_deref(), Some("finish"));
        let content: serde_json::Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content["turn_id"], TEST_TURN_A);
    }

    #[tokio::test]
    async fn run_marks_active_acp_tool_failed_when_turn_is_truncated() {
        use nomifun_ai_agent::protocol::events::{TurnStopReason, tool_call::{
            AcpToolCallEventData, AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallUpdateData,
        }};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
            session_id: "sess-1".into(),
            update: AcpToolCallUpdateData {
                session_update: AcpToolCallSessionUpdateKind::ToolCall,
                tool_call_id: "atc-001".into(),
                status: Some(AcpToolCallStatus::InProgress),
                title: Some("Bash".into()),
                kind: None,
                raw_input: Some(json!({"command": "sleep 10"})),
                raw_output: None,
                content: None,
                locations: None,
            },
            meta: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData {
            session_id: None,
            stop_reason: Some(TurnStopReason::MaxTokens),
        }))
        .unwrap();

        relay.consume(rx).await;

        let tool_message_id = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "acp_tool_call")
            .expect("ACP tool must be persisted")
            .message_id;
        MessageId::parse(&tool_message_id).expect("ACP tool row has a canonical message ID");
        let updates = repo.take_updates();
        let (_, update) = updates
            .iter()
            .find(|(message_id, _)| message_id == &tool_message_id)
            .expect("active ACP tool must be terminalized");
        assert_eq!(update.status.as_ref().map(|s| s.as_deref()), Some(Some("error")));
        let content: serde_json::Value = serde_json::from_str(update.content.as_deref().unwrap()).unwrap();
        assert_eq!(content["update"]["status"], "failed");
        assert_eq!(
            content["update"]["raw_output"],
            "The turn ended before this tool completed: max_tokens"
        );
    }

    #[tokio::test]
    async fn run_tool_group_persists_message() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallStatus, ToolGroupEntry};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);

        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_ASSISTANT_MESSAGE_ID.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus.clone(),
            None,
        );

        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolGroup(vec![
            ToolGroupEntry {
                call_id: "tg-001".into(),
                name: "search".into(),
                status: ToolCallStatus::Completed,
                description: Some("Web search".into()),
            },
            ToolGroupEntry {
                call_id: "tg-002".into(),
                name: "read_file".into(),
                status: ToolCallStatus::Completed,
                description: None,
            },
        ]))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let group_msg = inserts.iter().find(|m| m.r#type == "tool_group");
        assert!(group_msg.is_some());
        let msg = group_msg.unwrap();
        MessageId::parse(&msg.message_id).expect("tool-group row has a canonical message ID");
        assert_eq!(msg.msg_id.as_deref(), Some(TEST_ASSISTANT_MESSAGE_ID));
        assert_eq!(msg.status.as_deref(), Some("finish"));

        let content: serde_json::Value = serde_json::from_str(&msg.content).unwrap();
        assert!(content.is_array());
        assert_eq!(content.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn completed_artifact_tool_group_without_receipts_fails_the_enclosing_turn() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            ToolCallStatus, ToolGroupEntry,
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolGroup(vec![
            ToolGroupEntry {
                call_id: "group-image".into(),
                name: "image_gen".into(),
                status: ToolCallStatus::Completed,
                description: Some("generated".into()),
            },
            ToolGroupEntry {
                call_id: "group-export".into(),
                name: "export_pdf".into(),
                status: ToolCallStatus::Completed,
                description: Some("exported".into()),
            },
        ]))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(outcome.terminal.is_error());

        let row = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_group")
            .expect("failed artifact tool group row");
        assert_eq!(row.status.as_deref(), Some("error"));
        let content: Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(content[0]["status"], "error");
        assert_eq!(content[1]["status"], "error");
        assert!(content[0]["description"]
            .as_str()
            .is_some_and(|message| message.contains("required verified artifacts")));
        assert!(content[1]["description"]
            .as_str()
            .is_some_and(|message| message.contains("required verified artifacts")));

        let mut saw_finish = false;
        while let Ok(event) = ws_rx.try_recv() {
            saw_finish |= event.name == "message.stream" && event.data["type"] == "finish";
        }
        assert!(!saw_finish);
    }

    #[test]
    fn tool_group_count_contract_rejects_duplicate_paired_receipts() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            ToolCallEventData, ToolCallStatus, ToolGroupEntry,
        };

        let first = test_artifact("group-count-duplicate");
        let mut duplicate = first.clone();
        duplicate.id = PersistedArtifactId::new().into_string();
        let paired = ToolCallEventData {
            call_id: "group-count".into(),
            name: "image_gen".into(),
            args: json!({"prompt": "two cats", "count": 2}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![first, duplicate],
            retry: None,
        };
        let completed = HashMap::from([(paired.call_id.clone(), paired)]);
        let entries = vec![ToolGroupEntry {
            call_id: "group-count".into(),
            name: "image_gen".into(),
            status: ToolCallStatus::Completed,
            description: Some("generated two images".into()),
        }];

        let errors = tool_group_artifact_contract_errors(&entries, &completed);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("same canonical artifact path"));
    }

    #[tokio::test]
    async fn artifact_tool_group_is_suppressed_when_receipt_commit_fails() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            ToolCallEventData, ToolCallStatus, ToolGroupEntry,
        };

        let repo = Arc::new(RecordingRepo::new());
        repo.fail_artifact_commits();
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-tool-group-2pc-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "group-2pc-image".into(),
            name: "image_gen".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolGroup(vec![ToolGroupEntry {
            call_id: "group-2pc-image".into(),
            name: "image_gen".into(),
            status: ToolCallStatus::Completed,
            description: Some("generated".into()),
        }]))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert_eq!(
            outcome.terminal.code(),
            Some(AgentErrorCode::NomifunStateInconsistent)
        );

        assert!(
            repo.take_inserts()
                .iter()
                .all(|row| row.r#type != "tool_group"),
            "receipt-less artifact summaries must never enter durable history"
        );
        assert!(
            repo.take_updates().iter().all(|(_, update)| {
                update
                    .content
                    .as_deref()
                    .and_then(|content| serde_json::from_str::<Value>(content).ok())
                    .is_none_or(|content| !content.is_array())
            }),
            "a suppressed artifact summary must not acquire an update row"
        );

        let mut saw_finish = false;
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            assert_ne!(event.data["type"], "tool_group");
            saw_finish |= event.data["type"] == "finish";
        }
        assert!(!saw_finish);
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn artifact_tool_group_is_suppressed_after_receipt_commit_succeeds() {
        use nomifun_ai_agent::protocol::events::tool_call::{
            ToolCallEventData, ToolCallStatus, ToolGroupEntry,
        };

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let mut ws_rx = bus.subscribe();
        let (tx, _) = broadcast::channel(64);
        let workspace = std::env::temp_dir().join(format!(
            "nomifun-tool-group-2pc-success-test-{}",
            MessageId::new().into_string()
        ));
        std::fs::create_dir_all(&workspace).expect("create test workspace");
        let artifact = persisted_png_artifact(&workspace);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        )
        .with_artifact_workspace(workspace.clone())
        .with_test_legacy_unjournaled_artifacts();
        let rx = tx.subscribe();

        tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: "group-2pc-success".into(),
            name: "image_gen".into(),
            args: json!({"prompt": "cat"}),
            status: ToolCallStatus::Completed,
            input: None,
            output: Some("generated".into()),
            description: None,
            artifacts: vec![artifact],
            retry: None,
        }))
        .unwrap();
        tx.send(AgentStreamEvent::ToolGroup(vec![ToolGroupEntry {
            call_id: "group-2pc-success".into(),
            name: "image_gen".into(),
            status: ToolCallStatus::Completed,
            description: Some("generated".into()),
        }]))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default()))
            .unwrap();

        let outcome = relay.consume(rx).await;
        assert!(matches!(outcome.terminal, RelayTerminal::Finish));

        assert!(
            repo.take_inserts()
                .iter()
                .all(|row| row.r#type != "tool_group"),
            "receipt-less artifact summaries must never enter durable history"
        );
        assert!(
            repo.take_updates().iter().all(|(_, update)| {
                update
                    .content
                    .as_deref()
                    .and_then(|content| serde_json::from_str::<Value>(content).ok())
                    .is_none_or(|content| !content.is_array())
            }),
            "a suppressed artifact summary must not acquire an update row"
        );

        let mut stream_types = Vec::new();
        while let Ok(event) = ws_rx.try_recv() {
            if event.name != "message.stream" {
                continue;
            }
            stream_types.push(event.data["type"].clone());
            assert_ne!(event.data["type"], "tool_group");
        }
        assert_eq!(stream_types.last(), Some(&json!("finish")));
        std::fs::remove_dir_all(workspace).expect("remove test workspace");
    }

    #[tokio::test]
    async fn run_tool_group_with_failed_entry_persists_error() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallStatus, ToolGroupEntry};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolGroup(vec![
            ToolGroupEntry {
                call_id: "tg-001".into(),
                name: "read_file".into(),
                status: ToolCallStatus::Completed,
                description: None,
            },
            ToolGroupEntry {
                call_id: "tg-002".into(),
                name: "write_file".into(),
                status: ToolCallStatus::Error,
                description: Some("permission denied".into()),
            },
        ]))
        .unwrap();
        tx.send(AgentStreamEvent::Finish(FinishEventData::default())).unwrap();

        relay.consume(rx).await;

        let inserts = repo.take_inserts();
        let row = inserts.iter().find(|row| row.r#type == "tool_group").unwrap();
        MessageId::parse(&row.message_id).expect("tool-group row has a canonical message ID");
        assert_eq!(row.msg_id.as_deref(), Some(TEST_TURN_A));
        assert_eq!(row.status.as_deref(), Some("error"));
    }

    #[tokio::test]
    async fn run_marks_active_tool_group_failed_when_channel_closes() {
        use nomifun_ai_agent::protocol::events::tool_call::{ToolCallStatus, ToolGroupEntry};

        let repo = Arc::new(RecordingRepo::new());
        let bus = Arc::new(TestUserEventBus::new(64));
        let (tx, _) = broadcast::channel(64);
        let relay = StreamRelay::new(
            test_conversation_id(),
            TEST_TURN_A.into(),
            TEST_USER_ID.into(),
            repo.clone(),
            bus,
            None,
        );
        let rx = tx.subscribe();
        tx.send(AgentStreamEvent::ToolGroup(vec![ToolGroupEntry {
            call_id: "tg-001".into(),
            name: "Bash".into(),
            status: ToolCallStatus::Running,
            description: Some("build".into()),
        }]))
        .unwrap();
        drop(tx);

        relay.consume(rx).await;

        let group_id = repo
            .take_inserts()
            .into_iter()
            .find(|row| row.r#type == "tool_group")
            .expect("tool group must be persisted")
            .message_id;
        MessageId::parse(&group_id).expect("tool-group row has a canonical message ID");
        let updates = repo.take_updates();
        let (_, update) = updates
            .iter()
            .find(|(id, _)| id == &group_id)
            .expect("active tool group must be terminalized on channel close");
        assert_eq!(update.status.as_ref().map(|s| s.as_deref()), Some(Some("error")));
        let content: serde_json::Value = serde_json::from_str(update.content.as_deref().unwrap()).unwrap();
        assert_eq!(content[0]["status"], "error");
        assert!(content[0]["description"].as_str().unwrap().contains("channel_closed"));
    }

    // ── Helpers ──────────────────────────────────────────────────

    struct MockCronService;

    #[async_trait::async_trait]
    impl ICronService for MockCronService {
        async fn create_job(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _params: &crate::response_middleware::CronCreateParams,
        ) -> crate::response_middleware::CronCommandResult {
            crate::response_middleware::CronCommandResult {
                success: true,
                message: "created".into(),
            }
        }

        async fn update_job(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _params: &crate::response_middleware::CronUpdateParams,
        ) -> crate::response_middleware::CronCommandResult {
            crate::response_middleware::CronCommandResult {
                success: true,
                message: "updated".into(),
            }
        }

        async fn list_jobs(
            &self,
            _user_id: &str,
            _conversation_id: &str,
        ) -> crate::response_middleware::CronCommandResult {
            crate::response_middleware::CronCommandResult {
                success: true,
                message: "listed".into(),
            }
        }

        async fn delete_job(&self, _user_id: &str, _job_id: &str) -> crate::response_middleware::CronCommandResult {
            crate::response_middleware::CronCommandResult {
                success: true,
                message: "deleted".into(),
            }
        }
    }

    /// Recording repo that captures insert/update calls for assertions.
    struct RecordingRepo {
        inserts: Mutex<Vec<MessageRow>>,
        updates: Mutex<Vec<(String, nomifun_db::MessageRowUpdate)>>,
        correlations: Mutex<HashMap<(String, String, String, String), String>>,
        fail_next_message_insert: AtomicBool,
        commit_next_message_insert_then_error: AtomicBool,
        fail_message_inserts: AtomicBool,
        reject_duplicate_message_inserts: AtomicBool,
        block_message_inserts: AtomicBool,
        message_insert_notify: Notify,
        message_insert_attempts: AtomicUsize,
        fail_next_message_update: AtomicBool,
        fail_message_updates: AtomicBool,
        message_update_attempts: AtomicUsize,
        fail_message_update_attempt: AtomicUsize,
        block_message_updates: AtomicBool,
        message_update_notify: Notify,
        fail_conversation_updates: AtomicBool,
        fail_message_correlations: AtomicBool,
        fail_artifact_commits: AtomicBool,
        fail_artifact_reconciliation_read: AtomicBool,
        fail_next_message_read: AtomicBool,
        commit_artifact_rows_then_error: AtomicBool,
        commit_first_artifact_row_then_error: AtomicBool,
        block_artifact_commits: AtomicBool,
        artifact_commit_attempts: AtomicUsize,
    }

    impl RecordingRepo {
        fn new() -> Self {
            Self {
                inserts: Mutex::new(vec![]),
                updates: Mutex::new(vec![]),
                correlations: Mutex::new(HashMap::new()),
                fail_next_message_insert: AtomicBool::new(false),
                commit_next_message_insert_then_error: AtomicBool::new(false),
                fail_message_inserts: AtomicBool::new(false),
                reject_duplicate_message_inserts: AtomicBool::new(false),
                block_message_inserts: AtomicBool::new(false),
                message_insert_notify: Notify::new(),
                message_insert_attempts: AtomicUsize::new(0),
                fail_next_message_update: AtomicBool::new(false),
                fail_message_updates: AtomicBool::new(false),
                message_update_attempts: AtomicUsize::new(0),
                fail_message_update_attempt: AtomicUsize::new(0),
                block_message_updates: AtomicBool::new(false),
                message_update_notify: Notify::new(),
                fail_conversation_updates: AtomicBool::new(false),
                fail_message_correlations: AtomicBool::new(false),
                fail_artifact_commits: AtomicBool::new(false),
                fail_artifact_reconciliation_read: AtomicBool::new(false),
                fail_next_message_read: AtomicBool::new(false),
                commit_artifact_rows_then_error: AtomicBool::new(false),
                commit_first_artifact_row_then_error: AtomicBool::new(false),
                block_artifact_commits: AtomicBool::new(false),
                artifact_commit_attempts: AtomicUsize::new(0),
            }
        }

        fn fail_next_message_insert(&self) {
            self.fail_next_message_insert.store(true, AtomicOrdering::SeqCst);
        }

        fn commit_next_message_insert_then_error(&self) {
            self.commit_next_message_insert_then_error
                .store(true, AtomicOrdering::SeqCst);
        }

        fn fail_message_inserts(&self) {
            self.fail_message_inserts.store(true, AtomicOrdering::SeqCst);
        }

        fn reject_duplicate_message_inserts(&self) {
            self.reject_duplicate_message_inserts
                .store(true, AtomicOrdering::SeqCst);
        }

        fn set_block_message_inserts(&self, block: bool) {
            self.block_message_inserts.store(block, AtomicOrdering::SeqCst);
            if !block {
                self.message_insert_notify.notify_waiters();
            }
        }

        fn fail_next_message_update(&self) {
            self.fail_next_message_update.store(true, AtomicOrdering::SeqCst);
        }

        fn fail_message_updates(&self) {
            self.fail_message_updates.store(true, AtomicOrdering::SeqCst);
        }

        fn fail_message_update_attempt(&self, attempt: usize) {
            self.fail_message_update_attempt
                .store(attempt, AtomicOrdering::SeqCst);
        }

        fn block_message_updates(&self) {
            self.block_message_updates.store(true, AtomicOrdering::SeqCst);
        }

        fn set_block_message_updates(&self, block: bool) {
            self.block_message_updates.store(block, AtomicOrdering::SeqCst);
            if !block {
                self.message_update_notify.notify_waiters();
            }
        }

        fn fail_conversation_updates(&self) {
            self.fail_conversation_updates
                .store(true, AtomicOrdering::SeqCst);
        }

        fn fail_message_correlations(&self) {
            self.fail_message_correlations
                .store(true, AtomicOrdering::SeqCst);
        }

        fn fail_artifact_commits(&self) {
            self.fail_artifact_commits
                .store(true, AtomicOrdering::SeqCst);
        }

        fn fail_artifact_commit_with_unknown_reconciliation(&self) {
            self.fail_artifact_reconciliation_read
                .store(true, AtomicOrdering::SeqCst);
            self.fail_artifact_commits
                .store(true, AtomicOrdering::SeqCst);
        }

        fn commit_artifact_rows_then_error(&self) {
            self.commit_artifact_rows_then_error
                .store(true, AtomicOrdering::SeqCst);
        }

        fn commit_first_artifact_row_then_error(&self) {
            self.commit_first_artifact_row_then_error
                .store(true, AtomicOrdering::SeqCst);
        }

        fn block_artifact_commits(&self) {
            self.block_artifact_commits
                .store(true, AtomicOrdering::SeqCst);
        }

        fn message_insert_attempts(&self) -> usize {
            self.message_insert_attempts.load(AtomicOrdering::SeqCst)
        }

        fn message_update_attempts(&self) -> usize {
            self.message_update_attempts.load(AtomicOrdering::SeqCst)
        }

        fn artifact_commit_attempts(&self) -> usize {
            self.artifact_commit_attempts
                .load(AtomicOrdering::SeqCst)
        }

        fn take_inserts(&self) -> Vec<MessageRow> {
            let mut inserts = self.inserts.lock().unwrap();
            std::mem::take(&mut *inserts)
                .into_iter()
                .filter(|row| !(matches!(row.r#type.as_str(), "turn_root" | "system") && row.hidden))
                .collect()
        }

        #[allow(dead_code)]
        fn take_updates(&self) -> Vec<(String, nomifun_db::MessageRowUpdate)> {
            std::mem::take(&mut self.updates.lock().unwrap())
        }
    }

    #[async_trait::async_trait]
    impl IConversationRepository for RecordingRepo {
        async fn get(&self, _id: &str) -> Result<Option<nomifun_db::models::ConversationRow>, DbError> {
            Ok(None)
        }
        async fn create(&self, row: &nomifun_db::models::ConversationRow) -> Result<String, DbError> {
            Ok(row.conversation_id.clone())
        }
        async fn update(&self, _id: &str, _updates: &nomifun_db::ConversationRowUpdate) -> Result<(), DbError> {
            if self.fail_conversation_updates.load(AtomicOrdering::SeqCst) {
                return Err(DbError::Init(
                    "injected conversation status update failure".to_owned(),
                ));
            }
            Ok(())
        }
        async fn delete(&self, _id: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn list_paginated(
            &self,
            _user_id: &str,
            _filters: &nomifun_db::ConversationFilters,
        ) -> Result<nomifun_common::PaginatedResult<nomifun_db::models::ConversationRow>, DbError> {
            Ok(nomifun_common::PaginatedResult {
                items: vec![],
                total: 0,
                has_more: false,
            })
        }
        async fn find_by_source_and_chat(
            &self,
            _user_id: &str,
            _source: &str,
            _chat_id: &str,
            _agent_type: &str,
        ) -> Result<Option<nomifun_db::models::ConversationRow>, DbError> {
            Ok(None)
        }
        async fn list_by_cron_job(
            &self,
            _user_id: &str,
            _cron_job_id: &str,
        ) -> Result<Vec<nomifun_db::models::ConversationRow>, DbError> {
            Ok(vec![])
        }
        async fn list_associated(
            &self,
            _user_id: &str,
            _conversation_id: &str,
        ) -> Result<Vec<nomifun_db::models::ConversationRow>, DbError> {
            Ok(vec![])
        }
        async fn get_messages(
            &self,
            conv_id: &str,
            page: u32,
            page_size: u32,
            _order: nomifun_db::SortOrder,
        ) -> Result<nomifun_common::PaginatedResult<MessageRow>, DbError> {
            let rows = self
                .inserts
                .lock()
                .unwrap()
                .iter()
                .filter(|row| row.conversation_id == conv_id)
                .cloned()
                .collect::<Vec<_>>();
            let total = rows.len() as u64;
            let start = page.saturating_sub(1) as usize * page_size as usize;
            let items = rows
                .into_iter()
                .skip(start)
                .take(page_size as usize)
                .collect::<Vec<_>>();
            Ok(nomifun_common::PaginatedResult {
                has_more: start.saturating_add(items.len()) < total as usize,
                items,
                total,
            })
        }
        async fn get_message(&self, _conv_id: &str, message_id: &str) -> Result<Option<MessageRow>, DbError> {
            if self
                .fail_next_message_read
                .swap(false, AtomicOrdering::SeqCst)
            {
                return Err(DbError::Init(
                    "injected artifact reconciliation read failure".to_owned(),
                ));
            }
            Ok(self
                .inserts
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.message_id == message_id)
                .cloned())
        }
        async fn insert_message(&self, row: &MessageRow) -> Result<(), DbError> {
            let structural_turn_root = matches!(row.r#type.as_str(), "turn_root" | "system")
                && row.hidden
                && row.msg_id.as_deref() == Some(row.message_id.as_str());
            if !structural_turn_root {
                self.message_insert_attempts
                    .fetch_add(1, AtomicOrdering::SeqCst);
                while self.block_message_inserts.load(AtomicOrdering::SeqCst) {
                    let notified = self.message_insert_notify.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if !self.block_message_inserts.load(AtomicOrdering::SeqCst) {
                        break;
                    }
                    notified.await;
                }
                if self
                    .commit_next_message_insert_then_error
                    .swap(false, AtomicOrdering::SeqCst)
                {
                    self.inserts.lock().unwrap().push(row.clone());
                    return Err(DbError::Init(
                        "injected committed-but-unacknowledged message insert".to_owned(),
                    ));
                }
                if self.fail_message_inserts.load(AtomicOrdering::SeqCst) {
                    return Err(DbError::Conflict("injected message insert failure".to_owned()));
                }
                if self.fail_next_message_insert.swap(false, AtomicOrdering::SeqCst) {
                    return Err(DbError::Conflict("injected message insert failure".to_owned()));
                }
            }
            if (structural_turn_root
                || self
                    .reject_duplicate_message_inserts
                    .load(AtomicOrdering::SeqCst))
                && self
                    .inserts
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|existing| existing.message_id == row.message_id)
            {
                return Err(DbError::Conflict("injected duplicate message insert".to_owned()));
            }
            self.inserts.lock().unwrap().push(row.clone());
            Ok(())
        }
        async fn commit_turn_artifact_messages(
            &self,
            conversation_id: &str,
            turn_message_id: &str,
            messages: &[TurnArtifactMessageCommit],
            committed_at: i64,
        ) -> Result<Vec<MessageRow>, DbError> {
            self.artifact_commit_attempts
                .fetch_add(1, AtomicOrdering::SeqCst);
            if self.block_artifact_commits.load(AtomicOrdering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail_artifact_commits.load(AtomicOrdering::SeqCst) {
                if self
                    .fail_artifact_reconciliation_read
                    .load(AtomicOrdering::SeqCst)
                {
                    self.fail_next_message_read
                        .store(true, AtomicOrdering::SeqCst);
                }
                return Err(DbError::Conflict(
                    "injected atomic artifact commit failure".to_owned(),
                ));
            }
            let commit_then_error = self
                .commit_artifact_rows_then_error
                .swap(false, AtomicOrdering::SeqCst);
            let partial_commit_then_error = self
                .commit_first_artifact_row_then_error
                .swap(false, AtomicOrdering::SeqCst);

            let mut inserts = self.inserts.lock().unwrap();
            let mut updates = self.updates.lock().unwrap();
            for message in messages {
                if let Some(existing) = inserts.iter().find(|row| row.message_id == message.message_id)
                    && (existing.conversation_id != conversation_id
                        || existing.msg_id.as_deref() != Some(turn_message_id)
                        || existing.r#type != message.message_type
                        || existing.status.as_deref() != Some("work"))
                {
                    return Err(DbError::Conflict(
                        "injected repository found an incompatible provisional artifact row"
                            .to_owned(),
                    ));
                }
            }
            if partial_commit_then_error {
                let message = messages.first().expect("artifact commit batch is not empty");
                let row = inserts
                    .iter_mut()
                    .find(|row| row.message_id == message.message_id)
                    .expect("provisional artifact row exists");
                row.content.clone_from(&message.content);
                row.status = Some("finish".to_owned());
                return Err(DbError::Init(
                    "injected partial artifact commit with lost acknowledgement".to_owned(),
                ));
            }
            let mut committed = Vec::with_capacity(messages.len());
            for message in messages {
                if let Some(existing) = inserts.iter().find(|row| row.message_id == message.message_id) {
                    updates.push((
                        message.message_id.clone(),
                        nomifun_db::MessageRowUpdate {
                            content: Some(message.content.clone()),
                            status: Some(Some("finish".to_owned())),
                            hidden: None,
                        },
                    ));
                    let mut row = existing.clone();
                    row.content = message.content.clone();
                    row.status = Some("finish".to_owned());
                    committed.push(row);
                } else {
                    let row = MessageRow {
                        id: 0,
                        message_id: message.message_id.clone(),
                        conversation_id: conversation_id.to_owned(),
                        msg_id: Some(turn_message_id.to_owned()),
                        r#type: message.message_type.clone(),
                        content: message.content.clone(),
                        position: Some("left".to_owned()),
                        status: Some("finish".to_owned()),
                        hidden: false,
                        created_at: committed_at,
                    };
                    inserts.push(row.clone());
                    committed.push(row);
                }
            }
            if commit_then_error {
                // Model a transaction that became durable before its caller
                // received an acknowledgement. The relay must recover this as
                // success by querying every exact finished row and must retain
                // the physical artifact snapshots.
                for message in messages {
                    let row = inserts
                        .iter_mut()
                        .find(|row| row.message_id == message.message_id)
                        .expect("committed artifact row exists");
                    row.content.clone_from(&message.content);
                    row.status = Some("finish".to_owned());
                }
                return Err(DbError::Init(
                    "injected durable artifact commit with lost acknowledgement".to_owned(),
                ));
            }
            Ok(committed)
        }
        async fn claim_message_correlation(
            &self,
            conversation_id: &str,
            turn_message_id: &str,
            message_type: &str,
            correlation_key: &str,
        ) -> Result<String, DbError> {
            if self.fail_message_correlations.load(AtomicOrdering::SeqCst) {
                return Err(DbError::Conflict(
                    "injected message correlation failure".to_owned(),
                ));
            }
            let key = (
                conversation_id.to_owned(),
                turn_message_id.to_owned(),
                message_type.to_owned(),
                correlation_key.to_owned(),
            );
            Ok(self
                .correlations
                .lock()
                .unwrap()
                .entry(key)
                .or_insert_with(|| MessageId::new().into_string())
                .clone())
        }
        async fn update_message(&self, id: &str, updates: &nomifun_db::MessageRowUpdate) -> Result<(), DbError> {
            let attempt = self
                .message_update_attempts
                .fetch_add(1, AtomicOrdering::SeqCst)
                + 1;
            while self.block_message_updates.load(AtomicOrdering::SeqCst) {
                let notified = self.message_update_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !self.block_message_updates.load(AtomicOrdering::SeqCst) {
                    break;
                }
                notified.await;
            }
            if self.fail_message_updates.load(AtomicOrdering::SeqCst)
                || self.fail_next_message_update.swap(false, AtomicOrdering::SeqCst)
                || self.fail_message_update_attempt.load(AtomicOrdering::SeqCst) == attempt
            {
                return Err(DbError::Conflict("injected message update failure".to_owned()));
            }
            self.updates.lock().unwrap().push((id.to_owned(), updates.clone()));
            Ok(())
        }
        async fn delete_messages_by_conversation(&self, _conv_id: &str) -> Result<(), DbError> {
            Ok(())
        }
        async fn get_message_by_msg_id(
            &self,
            _conv_id: &str,
            msg_id: &str,
            msg_type: &str,
        ) -> Result<Option<MessageRow>, DbError> {
            let inserts = self.inserts.lock().unwrap();
            Ok(inserts
                .iter()
                .find(|m| m.msg_id.as_deref() == Some(msg_id) && m.r#type == msg_type)
                .cloned())
        }
        async fn search_messages(
            &self,
            _user_id: &str,
            _keyword: &str,
            _page: u32,
            _page_size: u32,
        ) -> Result<nomifun_common::PaginatedResult<nomifun_db::MessageSearchRow>, DbError> {
            Ok(nomifun_common::PaginatedResult {
                items: vec![],
                total: 0,
                has_more: false,
            })
        }
    }
}
