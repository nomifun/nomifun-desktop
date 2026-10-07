//! Exact Turn output from canonical Event/Payload/Effect facts.
//!
//! Reading output never changes facts or discovers files. Each output path has
//! a successful owner receipt, an admitted tool call, a successful Runtime
//! result, and a terminal Turn boundary. Current bytes are verified separately.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path};

use nomifun_agent_contracts::{SessionEventRecord, digest_bytes, digest_payload};
use nomifun_agent_runtime::{AgentEngineEvent, AgentReconciliationSource};
use nomifun_agent_session::{ChatCausalityFacts, TurnReceipt, TurnReceiptStatus};
use nomifun_common::AppError;
use nomifun_file::{AgentSessionPatchResult, PublishedWorkspaceArtifact, WorkspacePathObservation};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::delivery::AgentExecutionDelivery;

const MAX_VERIFIED_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Conflict(format!("canonical Agent Turn output is invalid: {}", message.into()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, AppError> {
    serde_json::from_value(value.clone()).map_err(|error| invalid(error.to_string()))
}

fn payload<'a>(facts: &'a ChatCausalityFacts, event: &SessionEventRecord) -> Result<&'a Value, AppError> {
    if event.agent_session_id != facts.session.agent_session_id || event.kind_version != 1 {
        return Err(invalid("event identity or version differs from the selected Session"));
    }
    facts.event_payloads.get(event.event_id.as_ref())
        .ok_or_else(|| invalid("event has no resolved canonical payload"))
}

#[derive(Deserialize)]
struct TurnStart { source_message_id: String }
#[derive(Default, Deserialize)]
struct TerminalError { code: Option<String>, message: Option<String>, retryable: Option<bool> }
#[derive(Default, Deserialize)]
struct TerminalPayload { message: Option<String>, code: Option<String>, error: Option<TerminalError> }
#[derive(Deserialize)]
struct AssistantPart { content: String, turn_id: String }
#[derive(Deserialize)]
struct AssistantCompleted { part_count: u64, content_digest: String }

fn boundaries<'a>(facts: &ChatCausalityFacts, receipt: &'a TurnReceipt)
    -> Result<(&'a SessionEventRecord, Option<&'a SessionEventRecord>), AppError> {
    if receipt.agent_session_id != facts.session.agent_session_id {
        return Err(invalid("receipt belongs to another Session"));
    }
    let start = receipt.started_event.as_ref().ok_or_else(|| invalid("Turn has no start fact"))?;
    if start.kind.0 != "turn/started" || start.correlation_id.as_ref() != receipt.operation_id.as_ref() {
        return Err(invalid("Turn start does not match its operation"));
    }
    payload(facts, start)?;
    let terminal = receipt.terminal_event.as_ref();
    if let Some(terminal) = terminal {
        let expected = match receipt.status {
            TurnReceiptStatus::Completed => "turn/completed",
            TurnReceiptStatus::Failed => "turn/failed",
            TurnReceiptStatus::Cancelled => "turn/cancelled",
            _ => return Err(invalid("nonterminal receipt has a terminal event")),
        };
        if terminal.seq <= start.seq || terminal.kind.0 != expected
            || terminal.correlation_id.as_ref() != receipt.operation_id.as_ref() {
            return Err(invalid("terminal event does not match the exact Turn"));
        }
        payload(facts, terminal)?;
    } else if receipt.status != TurnReceiptStatus::Running {
        return Err(invalid("closed Turn has no terminal fact"));
    }
    Ok((start, terminal))
}

/// Shared delivery adapter for Channel and AgentExecution consumers. Only
/// completed public assistant parts inside this Turn can become `result_text`.
/// Failed Turn metadata preserves the canonical typed error classification.
pub fn canonical_turn_delivery(
    facts: &ChatCausalityFacts, receipt: &TurnReceipt, replayed: bool,
) -> Result<AgentExecutionDelivery, AppError> {
    let (start, terminal) = boundaries(facts, receipt)?;
    let start_payload: TurnStart = decode(payload(facts, start)?)?;
    let mut delivery = AgentExecutionDelivery {
        message_id: start_payload.source_message_id.clone(), replayed,
        completed: terminal.is_some(), paused_reason: None,
        result_ok: None, result_text: None, result_error: None,
        result_error_code: None, result_error_retryable: None,
    };
    let Some(terminal) = terminal else { return Ok(delivery); };
    let mut parts: HashMap<String, (String, u64)> = HashMap::new();
    let mut latest_completed: Option<(u64, String)> = None;
    for event in facts.events.iter().filter(|event| event.seq > start.seq && event.seq < terminal.seq) {
        match event.kind.0.as_str() {
            "message/content-part" => {
                let part: AssistantPart = decode(payload(facts, event)?)?;
                if part.turn_id != start_payload.source_message_id { continue; }
                let current = parts.entry(event.correlation_id.as_ref().to_owned()).or_default();
                if current.0.len().saturating_add(part.content.len()) > 8 * 1024 * 1024 {
                    return Err(invalid("assistant output exceeds the canonical content budget"));
                }
                current.0.push_str(&part.content);
                current.1 += 1;
            }
            "message/completed" => {
                let completed: AssistantCompleted = decode(payload(facts, event)?)?;
                let content = match parts.remove(event.correlation_id.as_ref()) {
                    Some((text, count)) if count == completed.part_count
                        && digest_bytes(text.as_bytes()).as_ref() == completed.content_digest => text,
                    None if completed.part_count == 0
                        && digest_bytes(b"").as_ref() == completed.content_digest => String::new(),
                    None => continue, // Another Turn's message cannot supply output.
                    _ => return Err(invalid("assistant completion differs from its content parts")),
                };
                if latest_completed.as_ref().is_none_or(|(seq, _)| *seq < event.seq) {
                    latest_completed = Some((event.seq, content));
                }
            }
            _ => {}
        }
    }
    if !parts.is_empty() && receipt.status == TurnReceiptStatus::Completed {
        return Err(invalid("completed Turn has unfinished assistant content"));
    }
    delivery.result_text = latest_completed.map(|(_, text)| text).filter(|text| !text.trim().is_empty());
    delivery.result_error_retryable = Some(false);
    match receipt.status {
        TurnReceiptStatus::Completed => delivery.result_ok = Some(true),
        TurnReceiptStatus::Failed => {
            let terminal: TerminalPayload = decode(payload(facts, terminal)?)?;
            let error = terminal.error.unwrap_or_default();
            delivery.result_ok = Some(false);
            delivery.result_error = error.message.or(terminal.message);
            delivery.result_error_code = error.code.or(terminal.code).or_else(|| Some("turn_failed".into()));
            // Unknown failures never grant permission to replay effects.
            delivery.result_error_retryable = Some(error.retryable.unwrap_or(false));
        }
        TurnReceiptStatus::Cancelled => {
            delivery.result_ok = Some(false);
            delivery.result_error = Some("Turn cancelled".into());
            delivery.result_error_code = Some("cancelled".into());
        }
        _ => unreachable!(),
    }
    Ok(delivery)
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
struct EffectIdentity {
    effect_id: String, turn_id: String, operation_id: String,
    capability_module: String, action_id: String, input_digest: String,
    resource_binding_id: Option<String>, resource_key: Option<String>,
    owner_domain: String, strategy: String,
}
#[derive(Deserialize)]
struct ToolAdmission { operation_id: String, call_id: String, capability_id: String, action_id: String }
#[derive(Deserialize)]
struct EffectOutcome {
    result: Option<Value>, observation_truncated: Option<bool>, observation_digest: Option<String>,
}
#[derive(Deserialize)]
struct WriteReceipt {
    path: String, written: bool, workspace_path: WorkspacePathObservation, bytes: u64, sha256: String,
}
#[derive(Deserialize)]
struct DeleteReceipt { path: String, deleted: bool, workspace_path: WorkspacePathObservation }

#[derive(Clone)]
struct FileVersion { bytes: u64, sha256: String }

fn mutation_action(action: &str) -> bool {
    matches!(action, "workspace.files/write" | "workspace.files/patch" | "workspace.files/delete" | "workspace.artifacts/publish")
}

/// Verify the final successful version of every path affected by this exact
/// Turn. Effects are read directly from the canonical snapshot, without adding
/// a second output ledger. Failed calls never overwrite successful versions.
pub fn canonical_turn_output_files(
    facts: &ChatCausalityFacts, receipt: &TurnReceipt, workspace: Option<&Path>,
) -> Result<Vec<String>, AppError> {
    let (start, terminal) = boundaries(facts, receipt)?;
    let terminal = terminal.ok_or_else(|| invalid("output verification requires a closed Turn"))?;
    let selected: Vec<_> = facts.events.iter()
        .filter(|event| event.seq > start.seq && event.seq < terminal.seq).collect();
    let mut runtime_starts = HashMap::new();
    let mut successes = HashMap::new();
    let mut reconciled_successes = HashMap::new();
    for event in &selected {
        if event.kind.0 != "runtime/progress-recorded" || event.correlation_id.as_ref() != receipt.operation_id.as_ref() { continue; }
        let value = payload(facts, event)?.get("event").ok_or_else(|| invalid("Runtime record has no event"))?;
        if value.get("event").and_then(Value::as_str).is_some_and(|kind| kind.starts_with("host_")) { continue; }
        match decode::<AgentEngineEvent>(value)? {
            AgentEngineEvent::ToolStarted { step, call_id, capability_id, action_id } if mutation_action(action_id.as_ref()) => {
                if runtime_starts.insert(call_id.as_ref().to_owned(), (step, capability_id, action_id, event.seq)).is_some() {
                    return Err(invalid("duplicate mutation call admission"));
                }
            }
            AgentEngineEvent::ToolCompleted { step, result } if !result.is_error => {
                if let Some((started_step, capability, action, admitted_seq)) = runtime_starts.get(result.call_id.as_ref()) {
                    if *started_step != step || *admitted_seq >= event.seq { return Err(invalid("mutation result does not match its admission")); }
                    let output: Value = serde_json::from_str(&result.output_text()).map_err(|error| invalid(format!("mutation result is not a receipt: {error}")))?;
                    if successes.insert(result.call_id.as_ref().to_owned(), (capability.clone(), action.clone(), output, event.seq)).is_some() {
                        return Err(invalid("duplicate successful mutation result"));
                    }
                }
            }
            AgentEngineEvent::ToolOutcomeReconciled { step, result, source: AgentReconciliationSource::OwnerReceipt,
                evidence_event_id: Some(evidence), owner_operation_id: Some(operation),
            } if !result.is_error => {
                if let Some((started_step, capability, action, admitted_seq)) = runtime_starts.get(result.call_id.as_ref()) {
                    if *started_step != step || *admitted_seq >= event.seq {
                        return Err(invalid("reconciled mutation does not match its admission"));
                    }
                    if reconciled_successes.insert(result.call_id.as_ref().to_owned(),
                        (capability.clone(), action.clone(), evidence, operation, event.seq)).is_some() {
                        return Err(invalid("duplicate reconciled mutation result"));
                    }
                }
            }
            _ => {}
        }
    }
    let mut versions: BTreeMap<String, FileVersion> = BTreeMap::new();
    let mut confirmed_calls = HashSet::new();
    for event in &selected {
        if event.kind.0 != "effect/succeeded" { continue; }
        let value = payload(facts, event)?;
        let identity: EffectIdentity = decode(value)?;
        if identity.turn_id != receipt.operation_id.as_ref() || !mutation_action(&identity.action_id) { continue; }
        if event.correlation_id.as_ref() != identity.effect_id { return Err(invalid("effect identity differs from its event")); }
        let effect_start = event.causation_event_id.as_ref().and_then(|cause|
            selected.iter().copied().find(|candidate| &candidate.event_id == cause))
            .ok_or_else(|| invalid("successful mutation has no in-Turn effect start"))?;
        if effect_start.kind.0 != "effect/started" || effect_start.seq >= event.seq
            || decode::<EffectIdentity>(payload(facts, effect_start)?)? != identity {
            return Err(invalid("mutation effect settlement differs from its start"));
        }
        let call = effect_start.causation_event_id.as_ref().and_then(|cause|
            selected.iter().copied().find(|candidate| &candidate.event_id == cause))
            .ok_or_else(|| invalid("mutation has no canonical tool admission"))?;
        if call.kind.0 != "tool/call-started" || call.causation_event_id.as_ref() != Some(&start.event_id)
            || call.seq >= effect_start.seq { return Err(invalid("mutation call belongs to another Turn")); }
        let admission: ToolAdmission = decode(payload(facts, call)?)?;
        let outcome: EffectOutcome = decode(value)?;
        let (capability, action, output, completed_seq) = if let Some((capability, action, output, seq)) = successes.get(&admission.call_id) {
            let exact = outcome.result.as_ref() == Some(output)
                || (outcome.observation_truncated == Some(true)
                    && outcome.observation_digest.as_deref() == Some(digest_payload(&serde_json::json!({"result": output}))
                        .map_err(|error| invalid(error.to_string()))?.as_ref()));
            if !exact { return Err(invalid("tool result differs from the durable owner receipt")); }
            (capability, action, output, seq)
        } else if let Some((capability, action, evidence, operation, seq)) = reconciled_successes.get(&admission.call_id) {
            // An owner-backed reconciliation observes the original succeeded
            // effect. The notice text is not a new receipt or current output.
            if evidence != event.event_id.as_ref() || operation.as_ref() != identity.operation_id {
                return Err(invalid("reconciliation witness belongs to another effect"));
            }
            (capability, action, outcome.result.as_ref().ok_or_else(||
                invalid("reconciled mutation has no exact original owner receipt"))?, seq)
        } else { return Err(invalid("successful effect has no successful Runtime tool result")); };
        if admission.operation_id != identity.operation_id || admission.action_id != identity.action_id
            || admission.capability_id != identity.capability_module || action.as_ref() != identity.action_id
            || capability.as_ref() != identity.capability_module || *completed_seq <= event.seq {
            return Err(invalid("tool result belongs to another effect or action"));
        }
        if !confirmed_calls.insert(admission.call_id) { return Err(invalid("mutation call has duplicate effect receipts")); }
        let root = workspace.ok_or_else(|| invalid("successful mutation has no workspace"))?;
        let root = std::fs::canonicalize(root).map_err(|error| invalid(error.to_string()))?;
        match identity.action_id.as_str() {
            "workspace.files/write" => {
                let written: WriteReceipt = decode(output)?;
                if !written.written || written.path.trim().is_empty() { return Err(invalid("write receipt is not successful")); }
                let path = observation_path(&root, &written.workspace_path)?;
                versions.insert(path, FileVersion { bytes: written.bytes, sha256: written.sha256 });
            }
            "workspace.files/patch" => {
                let patched: AgentSessionPatchResult = decode(output)?;
                if patched.file_count != patched.files.len() { return Err(invalid("patch receipt file count differs")); }
                for file in patched.files {
                    let observation = file.workspace_path.ok_or_else(|| invalid("patch receipt has no workspace identity"))?;
                    let path = observation_path(&root, &observation)?;
                    versions.insert(path, FileVersion { bytes: file.bytes_after,
                        sha256: file.written_sha256.ok_or_else(|| invalid("patch receipt has no written digest"))? });
                }
            }
            "workspace.files/delete" => {
                let deleted: DeleteReceipt = decode(output)?;
                if !deleted.deleted || deleted.path.trim().is_empty() { return Err(invalid("delete receipt is not successful")); }
                let path = observation_path(&root, &deleted.workspace_path)?;
                versions.retain(|candidate, _| candidate != &path && !candidate.starts_with(&format!("{path}/")));
            }
            "workspace.artifacts/publish" => {
                let published: PublishedWorkspaceArtifact = decode(output)?;
                if published.workspace_root_sha256.as_deref() != Some(digest_bytes(root.to_str().ok_or_else(|| invalid("non-UTF8 workspace"))?.as_bytes()).as_ref())
                    || published.artifact_id != published.sha256
                    || published.relative_path != format!("{}/{}", nomifun_file::ARTIFACT_RELATIVE_ROOT, published.artifact_id) {
                    return Err(invalid("published artifact identity differs from its receipt"));
                }
                relative_path(&published.relative_path)?;
                versions.insert(published.relative_path, FileVersion { bytes: published.size_bytes, sha256: published.sha256 });
            }
            _ => unreachable!(),
        }
    }
    if successes.keys().any(|call| !confirmed_calls.contains(call)) {
        return Err(invalid("successful mutation result has no durable effect receipt"));
    }
    if versions.is_empty() { return Ok(Vec::new()); }
    let root = std::fs::canonicalize(workspace.ok_or_else(|| invalid("output has no workspace"))?)
        .map_err(|error| invalid(error.to_string()))?;
    versions.into_iter().map(|(relative, version)| verify_file(&root, &relative, &version)).collect()
}

fn relative_path(value: &str) -> Result<&Path, AppError> {
    let path = Path::new(value);
    if value.is_empty() || value.contains(['\\', ':', '\0']) || path.is_absolute()
        || value.split('/').any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
        || path.components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(invalid("receipt path is not workspace relative"));
    }
    Ok(path)
}

fn observation_path(root: &Path, observation: &WorkspacePathObservation) -> Result<String, AppError> {
    if observation.root_sha256 != digest_bytes(root.to_str().ok_or_else(|| invalid("non-UTF8 workspace"))?.as_bytes()).as_ref() {
        return Err(invalid("receipt belongs to another workspace"));
    }
    relative_path(&observation.path)?;
    Ok(observation.path.clone())
}

fn verify_file(root: &Path, relative: &str, version: &FileVersion) -> Result<String, AppError> {
    let path = root.join(relative_path(relative)?);
    if version.bytes > MAX_VERIFIED_OUTPUT_BYTES || version.sha256.len() != 64
        || !version.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("file receipt size or digest is invalid"));
    }
    let canonical = std::fs::canonicalize(&path).map_err(|error| invalid(error.to_string()))?;
    if canonical != path || !canonical.starts_with(root) { return Err(invalid("output escaped its observed workspace path")); }
    let mut file = std::fs::File::open(&canonical).map_err(|error| invalid(error.to_string()))?;
    let metadata = file.metadata().map_err(|error| invalid(error.to_string()))?;
    if !metadata.is_file() || metadata.len() != version.bytes { return Err(invalid("output bytes differ from the final receipt")); }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| invalid(error.to_string()))?;
        if read == 0 { break; }
        bytes += read as u64;
        if bytes > version.bytes { return Err(invalid("output grew after its receipt")); }
        hasher.update(&buffer[..read]);
    }
    if bytes != version.bytes || format!("{:x}", hasher.finalize()) != version.sha256.to_ascii_lowercase()
        || std::fs::canonicalize(&path).ok().as_ref() != Some(&canonical) {
        return Err(invalid("output hash differs from the final successful receipt"));
    }
    canonical.to_str().map(str::to_owned).ok_or_else(|| invalid("non-UTF8 output path"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentSessionId, CorrelationId, EventId, EventProducerId, IdempotencyKey,
        OperationId, SessionEventKind, SessionEventPayloadRef, SessionPayloadId, StrictJsonValue,
    };
    use nomifun_agent_runtime::AgentToolResult;
    use serde_json::json;
    use std::collections::BTreeSet;

    const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000201";
    const TURN: &str = "turn:user:owner:session:initial-turn";
    const ROOT: &str = "0190f5fe-7c00-7a00-8000-000000000202";

    struct Fixture { facts: ChatCausalityFacts, receipt: TurnReceipt, next: u64 }

    impl Fixture {
        fn new() -> Self {
            let session = decode(&json!({
                "agent_session_id": SESSION,
                "owner_ref":{"principal_kind":"user","principal_id":"owner"},
                "metadata":{"title":null,"archived":false,"pinned":false},
                "agent_binding": {
                    "preset_revision_ref":{"preset_id":"preset","revision":1,"revision_digest":"a".repeat(64)},
                    "resolved_snapshot_ref":{"snapshot_id":"snapshot","snapshot_digest":"b".repeat(64)},
                    "typed_resource_bindings":[],"binding_version":1,
                },
                "next_seq":1001,
            })).unwrap();
            let facts = ChatCausalityFacts {
                session,
                head: nomifun_agent_session::SessionHeadProjection {
                    session_id: AgentSessionId::from(SESSION), status: "completed".into(), active_turn_id: None,
                    active_set_generation: 1, last_seq: 1000, unread_count: 0,
                },
                events: Vec::new(), event_payloads: BTreeMap::new(), operation_ids: BTreeSet::new(),
                turn_route_identities: BTreeSet::new(), execution_generation: 1, execution_fence: 1,
                fork_context: None,
            };
            let mut fixture = Self { facts, receipt: TurnReceipt {
                agent_session_id: SESSION.into(), operation_id: TURN.into(), status: TurnReceiptStatus::Completed,
                started_event: None, terminal_event: None,
            }, next: 1 };
            let start = fixture.push("turn/started", TURN, None, json!({"source_message_id":ROOT}));
            fixture.receipt.started_event = Some(start);
            fixture.next = 1000;
            let terminal = fixture.push("turn/completed", TURN, None, json!({"model_steps":1,"finish_reason":"stop"}));
            fixture.receipt.terminal_event = Some(terminal);
            fixture.next = 2;
            fixture
        }
        fn push(&mut self, kind: &str, correlation: &str, cause: Option<&str>, value: Value) -> SessionEventRecord {
            let event = SessionEventRecord {
                agent_session_id: SESSION.into(), seq: self.next,
                event_id: EventId::from(format!("event:{}", self.next)),
                producer_id: EventProducerId::from("test"),
                idempotency_key: IdempotencyKey::from(format!("event:{}", self.next)),
                kind: SessionEventKind(kind.into()), kind_version: 1,
                correlation_id: CorrelationId::from(correlation), causation_event_id: cause.map(EventId::from),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(value.clone())),
            };
            self.next += 1;
            self.facts.event_payloads.insert(event.event_id.as_ref().into(), value);
            self.facts.events.push(event.clone());
            self.facts.events.sort_by_key(|event| event.seq);
            event
        }
        fn runtime(&mut self, event: AgentEngineEvent) -> SessionEventRecord {
            self.push("runtime/progress-recorded", TURN, None, json!({"event":event}))
        }
        fn mutation(&mut self, action: &str, output: Value) {
            let suffix = self.next.to_string();
            let call_id = format!("call-{suffix}");
            let op_id = format!("tool-{suffix}");
            let effect_id = format!("effect-{suffix}");
            let capability = action.split('/').next().unwrap();
            self.runtime(AgentEngineEvent::ToolStarted {
                step: 1, call_id: call_id.clone().into(), capability_id: capability.into(), action_id: action.into(),
            });
            let root = self.receipt.started_event.as_ref().unwrap().event_id.as_ref().to_owned();
            let call = self.push("tool/call-started", &call_id, Some(&root), json!({
                "operation_id":op_id,"call_id":call_id,"capability_id":capability,"action_id":action,"name":"tool"
            }));
            let identity = json!({
                "effect_id": effect_id,"turn_id":TURN,"operation_id":op_id,
                "capability_module":capability,"action_id":action,"input_digest":"c".repeat(64),
                "owner_domain":"workspace","strategy":"managed_effect",
            });
            let start = self.push("effect/started", &effect_id, Some(call.event_id.as_ref()), identity.clone());
            let mut settled = identity;
            settled["result"] = output.clone();
            self.push("effect/succeeded", &effect_id, Some(start.event_id.as_ref()), settled);
            self.runtime(AgentEngineEvent::ToolCompleted {
                step: 1, result: AgentToolResult::text(call_id.into(), output.to_string(), false),
            });
        }
        fn write(&mut self, root: &Path, relative: &str, bytes: &[u8]) {
            std::fs::write(root.join(relative), bytes).unwrap();
            self.mutation("workspace.files/write", json!({
                "path":relative,"written":true,"created":true,"bytes":bytes.len(),
                "workspace_path": observation(root,relative),"sha256":digest_bytes(bytes),
            }));
        }
        fn assistant(&mut self, root: &str, text: &str) {
            let message = format!("assistant:{}", self.next);
            self.push("message/content-part", &message, None, json!({"turn_id":root,"content":text}));
            self.push("message/completed", &message, None, json!({"part_count":1,"content_digest":digest_bytes(text.as_bytes())}));
        }
        fn files(&self, root: &Path) -> Result<Vec<String>, AppError> {
            canonical_turn_output_files(&self.facts, &self.receipt, Some(root))
        }
    }

    fn observation(root: &Path, relative: &str) -> Value {
        let root = std::fs::canonicalize(root).unwrap();
        json!({"root_sha256":digest_bytes(root.to_str().unwrap().as_bytes()),"path":relative,"case_resolved":false})
    }

    #[test]
    fn writes_use_the_final_successful_version_and_do_not_discover_files() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        fixture.write(root.path(), "report.md", b"final");
        std::fs::write(root.path().join("unclaimed.txt"), "existing").unwrap();
        assert_eq!(fixture.files(root.path()).unwrap(), [std::fs::canonicalize(root.path().join("report.md")).unwrap().to_str().unwrap()]);
    }

    #[test]
    fn patch_receipts_supersede_writes_and_deleted_outputs_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        std::fs::write(root.path().join("report.md"), b"patched").unwrap();
        fixture.mutation("workspace.files/patch", json!({
            "files":[{"path":"report.md","workspace_path":observation(root.path(),"report.md"),
                "bytes_before":5,"bytes_after":7,"hunks_applied":1,"created":false,
                "source_sha256":digest_bytes(b"first"),"written_sha256":digest_bytes(b"patched")}],
            "file_count":1,"total_bytes_before":5,"total_bytes_after":7,
        }));
        assert_eq!(fixture.files(root.path()).unwrap().len(), 1);
        std::fs::remove_file(root.path().join("report.md")).unwrap();
        fixture.mutation("workspace.files/delete", json!({
            "path":"report.md","deleted":true,"workspace_path":observation(root.path(),"report.md")
        }));
        assert!(fixture.files(root.path()).unwrap().is_empty());
        fixture.write(root.path(), "report.md", b"recreated");
        assert_eq!(fixture.files(root.path()).unwrap().len(), 1);
    }

    #[test]
    fn published_artifacts_verify_the_immutable_bytes_and_workspace_identity() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.md"), "published").unwrap();
        let store = nomifun_file::WorkspaceArtifactStore::new(root.path()).unwrap();
        let artifact = store.publish("source.md", None).unwrap();
        let mut fixture = Fixture::new();
        fixture.mutation("workspace.artifacts/publish", serde_json::to_value(&artifact).unwrap());
        std::fs::write(root.path().join("source.md"), "changed source").unwrap();
        assert_eq!(fixture.files(root.path()).unwrap(), [std::fs::canonicalize(root.path().join(&artifact.relative_path)).unwrap().to_str().unwrap()]);
        let outside = tempfile::tempdir().unwrap();
        assert!(fixture.files(outside.path()).is_err());
    }

    #[test]
    fn altered_bytes_forged_hash_size_and_path_escape_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        std::fs::write(root.path().join("report.md"), b"other").unwrap();
        assert!(fixture.files(root.path()).is_err());
        assert!(verify_file(root.path(), "../escaped", &FileVersion { bytes:0,sha256:digest_bytes(b"").as_ref().to_owned() }).is_err());
        assert!(verify_file(root.path(), "report.md", &FileVersion { bytes:MAX_VERIFIED_OUTPUT_BYTES+1,sha256:"0".repeat(64) }).is_err());
        assert!(verify_file(root.path(), "report.md", &FileVersion { bytes:99,sha256:digest_bytes(b"other").as_ref().to_owned() }).is_err());
        assert!(verify_file(root.path(), "report.md", &FileVersion { bytes:5,sha256:"0".repeat(64) }).is_err());
    }

    #[test]
    fn failed_or_unsettled_tool_results_cannot_supply_outputs() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        fixture.facts.events.retain(|event| event.kind.0 != "effect/succeeded");
        assert!(fixture.files(root.path()).is_err());
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        let result = fixture.facts.events.iter().find(|event| event.kind.0 == "runtime/progress-recorded"
            && fixture.facts.event_payloads[event.event_id.as_ref()].pointer("/event/event").and_then(Value::as_str) == Some("tool_completed")).unwrap();
        fixture.facts.event_payloads.get_mut(result.event_id.as_ref()).unwrap()["event"]["result"]["is_error"] = json!(true);
        assert!(fixture.files(root.path()).is_err());
    }

    #[test]
    fn output_requires_an_exact_admitted_action_and_preterminal_receipt() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        let admission = fixture.facts.events.iter().find(|event| event.kind.0 == "tool/call-started").unwrap();
        fixture.facts.event_payloads.get_mut(admission.event_id.as_ref()).unwrap()["action_id"] = json!("workspace.files/read");
        assert!(fixture.files(root.path()).is_err());
        let mut fixture = Fixture::new();
        fixture.next = 1001;
        fixture.write(root.path(), "later.md", b"later");
        assert!(fixture.files(root.path()).unwrap().is_empty());
        fixture.receipt.terminal_event = None;
        assert!(fixture.files(root.path()).is_err());
    }

    #[test]
    fn public_text_uses_resolved_payloads_and_excludes_reasoning_other_turns_and_future_text() {
        let mut fixture = Fixture::new();
        fixture.push("thinking/content-part", "thinking", None, json!({"turn_id":ROOT,"content":"secret reasoning"}));
        fixture.assistant("another-root", "other turn");
        fixture.assistant(ROOT, "final answer");
        // A stored payload must be resolved through canonical facts, never
        // guessed from an inline event or reconstructed Message projection.
        for event in fixture.facts.events.iter_mut().filter(|event| event.kind.0 == "message/content-part") {
            event.payload = SessionEventPayloadRef::Stored(SessionPayloadId::from("offloaded"));
        }
        fixture.next = 1001;
        fixture.assistant(ROOT, "future answer");
        let result = canonical_turn_delivery(&fixture.facts, &fixture.receipt, true).unwrap();
        assert_eq!(result.result_text.as_deref(), Some("final answer"));
        assert_eq!(result.result_ok, Some(true));
        let mut reasoning = Fixture::new();
        reasoning.push("thinking/content-part", "thinking", None, json!({"turn_id":ROOT,"content":"reasoning only"}));
        assert!(canonical_turn_delivery(&reasoning.facts, &reasoning.receipt, true).unwrap().result_text.is_none());
    }

    #[test]
    fn public_text_completion_digest_is_required_and_errors_preserve_retry_authority() {
        let mut fixture = Fixture::new();
        fixture.assistant(ROOT, "answer");
        let completed = fixture.facts.events.iter().find(|event| event.kind.0 == "message/completed").unwrap();
        fixture.facts.event_payloads.get_mut(completed.event_id.as_ref()).unwrap()["content_digest"] = json!("0".repeat(64));
        assert!(canonical_turn_delivery(&fixture.facts, &fixture.receipt, true).is_err());
        let mut fixture = Fixture::new();
        fixture.receipt.status = TurnReceiptStatus::Failed;
        let mut terminal = fixture.receipt.terminal_event.take().unwrap();
        terminal.kind = SessionEventKind("turn/failed".into());
        fixture.facts.event_payloads.insert(terminal.event_id.as_ref().into(), json!({
            "message":"failure", "error":{"code":"USER_LLM_PROVIDER_RATE_LIMITED","message":"rate limited","retryable":true}
        }));
        fixture.receipt.terminal_event = Some(terminal.clone());
        let error = canonical_turn_delivery(&fixture.facts, &fixture.receipt, true).unwrap();
        assert_eq!(error.result_error_code.as_deref(), Some("USER_LLM_PROVIDER_RATE_LIMITED"));
        assert_eq!(error.result_error_retryable, Some(true));
        fixture.facts.event_payloads.insert(terminal.event_id.as_ref().into(), json!({"message":"unclassified"}));
        assert_eq!(canonical_turn_delivery(&fixture.facts, &fixture.receipt, true).unwrap().result_error_retryable, Some(false));
    }
    #[test]
    fn resumed_original_owner_receipt_is_verified_without_replaying_the_mutation() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        let effect = fixture.facts.events.iter().find(|event| event.kind.0 == "effect/succeeded").unwrap();
        let evidence = effect.event_id.as_ref().to_owned();
        let effect_payload = &fixture.facts.event_payloads[effect.event_id.as_ref()];
        let operation = effect_payload["operation_id"].as_str().unwrap().to_owned();
        let completed = fixture.facts.events.iter().find(|event| event.kind.0 == "runtime/progress-recorded"
            && fixture.facts.event_payloads[event.event_id.as_ref()].pointer("/event/event").and_then(Value::as_str) == Some("tool_completed")).unwrap().clone();
        let native: AgentEngineEvent = decode(&fixture.facts.event_payloads[completed.event_id.as_ref()]["event"]).unwrap();
        let AgentEngineEvent::ToolCompleted { step, result } = native else { panic!("fixture result"); };
        fixture.facts.events.retain(|event| event.event_id != completed.event_id);
        fixture.runtime(AgentEngineEvent::ToolOutcomeReconciled {
            step, result: AgentToolResult::text(result.call_id, "Historical canonical owner receipt. Inspect current state.", false),
            source: AgentReconciliationSource::OwnerReceipt, evidence_event_id: Some(evidence),
            owner_operation_id: Some(OperationId::from(operation)),
        });
        assert_eq!(fixture.files(root.path()).unwrap().len(), 1);
        let reconciled = fixture.facts.events.iter().find(|event| event.kind.0 == "runtime/progress-recorded"
            && fixture.facts.event_payloads[event.event_id.as_ref()].pointer("/event/event").and_then(Value::as_str) == Some("tool_outcome_reconciled")).unwrap();
        fixture.facts.event_payloads.get_mut(reconciled.event_id.as_ref()).unwrap()["event"]["evidence_event_id"] = json!("different-effect");
        assert!(fixture.files(root.path()).is_err());
    }

    #[test]
    fn cancelled_receipt_does_not_promote_unfinished_assistant_parts_to_output() {
        let mut fixture = Fixture::new();
        fixture.push("message/content-part", "unfinished", None, json!({"turn_id":ROOT,"content":"unfinished answer"}));
        fixture.receipt.status = TurnReceiptStatus::Cancelled;
        fixture.receipt.terminal_event.as_mut().unwrap().kind = SessionEventKind("turn/cancelled".into());
        let delivery = canonical_turn_delivery(&fixture.facts, &fixture.receipt, true).unwrap();
        assert_eq!(delivery.result_ok, Some(false));
        assert_eq!(delivery.result_error_code.as_deref(), Some("cancelled"));
        assert!(delivery.result_text.is_none());
    }

    #[test]
    fn bounded_owner_observation_digest_verifies_large_receipts_without_a_second_ledger() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        let effect = fixture.facts.events.iter().find(|event| event.kind.0 == "effect/succeeded").unwrap().clone();
        let value = fixture.facts.event_payloads.get_mut(effect.event_id.as_ref()).unwrap();
        let result = value.as_object_mut().unwrap().remove("result").unwrap();
        value["observation_truncated"] = json!(true);
        value["observation_digest"] = json!(digest_payload(&json!({"result":result})).unwrap());
        assert_eq!(fixture.files(root.path()).unwrap().len(), 1);
        fixture.facts.event_payloads.get_mut(effect.event_id.as_ref()).unwrap()["observation_digest"] = json!("0".repeat(64));
        assert!(fixture.files(root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn output_symlinks_cannot_redirect_receipt_verification_outside_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::new();
        fixture.write(root.path(), "report.md", b"first");
        std::fs::write(outside.path().join("outside.md"), b"first").unwrap();
        std::fs::remove_file(root.path().join("report.md")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("outside.md"), root.path().join("report.md")).unwrap();
        assert!(fixture.files(root.path()).is_err());
    }

}
