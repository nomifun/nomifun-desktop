//! Read-only Turn file presentation from canonical owner receipts and observations.
//!
//! This query does not inspect current bytes, infer changes from tool arguments,
//! or grant delivery/completion evidence. Deleted and subsequently missing files
//! remain history entries. The file domain owns discovery; Session owns its Turn.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nomifun_agent_contracts::{
    AgentSessionId, EventId, ObservedFileOperation, SessionEventRecord,
    TurnWorkspaceFilesObservedV1, TURN_WORKSPACE_FILES_OBSERVED, MAX_TURN_OBSERVED_FILES,
    digest_bytes, digest_payload,
};
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::{AgentSessionStore, ChatCausalityFacts, SessionStoreError};
use nomifun_api_types::{TurnFileItem, TurnFileSummary, TurnPublishedFile};
use nomifun_common::{AppError, FileChangeOperation};
use nomifun_file::{AgentSessionPatchResult, PublishedWorkspaceArtifact, WorkspacePathObservation};
use serde::Deserialize;
use serde_json::Value;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Conflict(format!("canonical Turn file presentation is invalid: {}", message.into()))
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, AppError> {
    serde_json::from_value(value.clone()).map_err(|error| invalid(error.to_string()))
}
fn payload<'a>(facts: &'a ChatCausalityFacts, event: &SessionEventRecord) -> Result<&'a Value, AppError> {
    if event.agent_session_id != facts.session.agent_session_id || event.kind_version != 1 {
        return Err(invalid("event belongs to another Session or version"));
    }
    facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(|| invalid("event payload is missing"))
}
fn portable_path(path: &str) -> Result<(), AppError> {
    if path.is_empty() || path.len() > 4096 || path.starts_with('/')
        || path.contains('\\') || path.as_bytes().get(1) == Some(&b':') || path.chars().any(char::is_control)
        || path.split('/').any(|part| part.is_empty() || matches!(part, "." | "..")) {
        return Err(invalid("file path is not a normalized relative path"));
    }
    Ok(())
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn mutation_action(action: &str) -> bool {
    matches!(action, "workspace.files/write" | "workspace.files/patch" | "workspace.files/delete" | "workspace.artifacts/publish")
}

fn file_work_action(action: &str) -> bool {
    mutation_action(action) || matches!(action, "workspace.process/exec" | "workspace.process/start"
        | "workspace.process/input" | "workspace.process/close_stdin")
}

/// Absence of a new observation is not a degraded file list for old ordinary
/// chat. Keep partial lists only when the canonical Turn actually admitted
/// file work or has explicit owner/observer facts; read/control tools do not
/// establish filesystem mutation intent.
fn has_file_presentation(facts: &ChatCausalityFacts) -> Result<bool, AppError> {
    let (turn_start, _) = start(facts)?;
    for event in &facts.events {
        if event.kind.0 == TURN_WORKSPACE_FILES_OBSERVED { return Ok(true); }
        if !matches!(event.kind.0.as_str(), "tool/call-started" | "effect/started"
            | "effect/succeeded" | "effect/failed" | "effect/uncertain") { continue; }
        let value = payload(facts, event)?;
        let action = value.get("action_id").and_then(Value::as_str)
            .ok_or_else(|| invalid("admitted file-work source has no canonical action identity"))?;
        if !file_work_action(action) { continue; }
        let belongs = if event.kind.0 == "tool/call-started" {
            event.causation_event_id.as_ref() == Some(&turn_start.event_id)
        } else {
            value.get("turn_id").and_then(Value::as_str) == Some(turn_start.correlation_id.as_ref())
        };
        if belongs { return Ok(true); }
    }
    Ok(false)
}
fn start(facts: &ChatCausalityFacts) -> Result<(&SessionEventRecord, &str), AppError> {
    let start = facts.events.iter().find(|event| event.kind.0 == "turn/started")
        .ok_or_else(|| invalid("Turn start is missing"))?;
    let value = payload(facts, start)?;
    if value.get("operation_id").and_then(Value::as_str) != Some(start.correlation_id.as_ref()) {
        return Err(invalid("Turn start operation differs"));
    }
    let root = value.get("source_message_id").and_then(Value::as_str)
        .ok_or_else(|| invalid("Turn root is missing"))?;
    nomifun_common::validate_uuidv7(root).map_err(|error| invalid(error.to_string()))?;
    Ok((start, root))
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
struct EffectIdentity {
    effect_id: String, turn_id: String, operation_id: String,
    capability_module: String, action_id: String, input_digest: String,
    resource_binding_id: Option<String>, resource_key: Option<String>,
    owner_domain: String, strategy: String,
}
#[derive(Deserialize)]
struct ToolAdmission { operation_id: String, capability_id: String, action_id: String, call_id: String }
#[derive(Deserialize)]
struct WriteReceipt {
    path: String, written: bool, created: bool, workspace_path: Option<WorkspacePathObservation>,
    bytes: u64, sha256: String,
}
#[derive(Deserialize)]
struct DeleteReceipt { path: String, deleted: bool, workspace_path: Option<WorkspacePathObservation> }
#[derive(Clone)]
struct FileFact {
    root: String, path: String, operation: FileChangeOperation,
    bytes: Option<u64>, sha256: Option<String>, published: Option<PublishedWorkspaceArtifact>, seq: u64,
    /// Only the first real Turn baseline can supply this initial byte version.
    initial_sha256: Option<String>,
    /// An owner delete removes the entry and subtree; a snapshot's removed
    /// regular file may instead have become a directory at the same path.
    proves_entry_absent: bool,
}
struct ReceiptFacts { files: Vec<FileFact>, complete: bool }

fn observed_file(observation: WorkspacePathObservation, operation: FileChangeOperation,
    bytes: Option<u64>, sha256: Option<String>, seq: u64) -> Result<FileFact, AppError> {
    portable_path(&observation.path)?;
    if !digest_valid(&observation.root_sha256) || sha256.as_ref().is_some_and(|hash| !digest_valid(hash)) {
        return Err(invalid("file receipt digest is invalid"));
    }
    Ok(FileFact { root: observation.root_sha256, path: observation.path, operation,
        bytes, sha256, published: None, seq, initial_sha256: None, proves_entry_absent: false })
}

/// A succeeded owner settlement proves a physical mutation even when delivery
/// of the Runtime tool result was interrupted. A truncated settlement can only
/// be expanded using its exact admitted call and committed output digest.
fn receipts(facts: &ChatCausalityFacts) -> Result<ReceiptFacts, AppError> {
    let (turn_start, _) = start(facts)?;
    let operation = turn_start.correlation_id.as_ref();
    let by_id: BTreeMap<_, _> = facts.events.iter().map(|event| (event.event_id.as_ref(), event)).collect();
    let mut collected = ReceiptFacts { files: Vec::new(), complete: true };
    for event in &facts.events {
        if event.kind.0 != "effect/succeeded" { continue; }
        let value = payload(facts, event)?;
        let identity: EffectIdentity = decode(value)?;
        if identity.turn_id != operation || !mutation_action(&identity.action_id) { continue; }
        let started = event.causation_event_id.as_ref().and_then(|id| by_id.get(id.as_ref()).copied())
            .ok_or_else(|| invalid("file effect has no start"))?;
        if started.kind.0 != "effect/started" || started.seq >= event.seq
            || event.correlation_id.as_ref() != identity.effect_id
            || decode::<EffectIdentity>(payload(facts, started)?)? != identity {
            return Err(invalid("file effect settlement identity differs from its start"));
        }
        let call = started.causation_event_id.as_ref().and_then(|id| by_id.get(id.as_ref()).copied())
            .ok_or_else(|| invalid("file effect has no tool admission"))?;
        if call.kind.0 != "tool/call-started" || call.causation_event_id.as_ref() != Some(&turn_start.event_id)
            || call.seq <= turn_start.seq || call.seq >= started.seq {
            return Err(invalid("file effect belongs to another Turn"));
        }
        let admission: ToolAdmission = decode(payload(facts, call)?)?;
        if admission.operation_id != identity.operation_id || admission.capability_id != identity.capability_module
            || admission.action_id != identity.action_id {
            return Err(invalid("file effect differs from its admitted action"));
        }
        let expanded;
        let result = if let Some(result) = value.get("result") { result } else {
            if value.get("observation_truncated").and_then(Value::as_bool) != Some(true) {
                return Err(invalid("successful owner receipt has no result"));
            }
            let Some(result) = truncated_result(facts, event.seq, operation, &admission, value)? else {
                collected.complete = false;
                continue;
            };
            expanded = result;
            &expanded
        };
        match identity.action_id.as_str() {
            "workspace.files/write" => {
                let written: WriteReceipt = decode(result)?;
                if !written.written || written.path.is_empty() { return Err(invalid("unsuccessful write receipt")); }
                let Some(observation) = written.workspace_path else { collected.complete = false; continue; };
                collected.files.push(observed_file(observation,
                    if written.created { FileChangeOperation::Create } else { FileChangeOperation::Modify },
                    Some(written.bytes), Some(written.sha256), event.seq)?);
            }
            "workspace.files/patch" => {
                let patched: AgentSessionPatchResult = decode(result)?;
                if patched.file_count != patched.files.len() { return Err(invalid("patch receipt count differs")); }
                for file in patched.files {
                    let Some(observation) = file.workspace_path else { collected.complete = false; continue; };
                    collected.files.push(observed_file(observation,
                        if file.created { FileChangeOperation::Create } else { FileChangeOperation::Modify },
                        Some(file.bytes_after), file.written_sha256, event.seq)?);
                }
            }
            "workspace.files/delete" => {
                let deleted: DeleteReceipt = decode(result)?;
                if !deleted.deleted || deleted.path.is_empty() { return Err(invalid("unsuccessful delete receipt")); }
                let Some(observation) = deleted.workspace_path else { collected.complete = false; continue; };
                let mut file = observed_file(observation, FileChangeOperation::Delete, None, None, event.seq)?;
                file.proves_entry_absent = true;
                collected.files.push(file);
            }
            "workspace.artifacts/publish" => {
                let published: PublishedWorkspaceArtifact = decode(result)?;
                portable_path(&published.source_path)?;
                portable_path(&published.relative_path)?;
                let Some(root) = published.workspace_root_sha256.clone() else { collected.complete = false; continue; };
                if !digest_valid(&root) || !digest_valid(&published.sha256) || published.artifact_id != published.sha256
                    || published.relative_path != format!("{}/{}", nomifun_file::ARTIFACT_RELATIVE_ROOT, published.artifact_id) {
                    return Err(invalid("published artifact identity differs from its receipt"));
                }
                collected.files.push(FileFact { root, path: published.source_path.clone(), operation: FileChangeOperation::Modify,
                    bytes: Some(published.size_bytes), sha256: Some(published.sha256.clone()), published: Some(published), seq: event.seq,
                    initial_sha256: None, proves_entry_absent: false });
            }
            _ => unreachable!(),
        }
    }
    Ok(collected)
}

fn truncated_result(facts: &ChatCausalityFacts, after: u64, operation: &str,
    admission: &ToolAdmission, owner: &Value) -> Result<Option<Value>, AppError> {
    for event in facts.events.iter().filter(|event| event.seq > after
        && event.kind.0 == "runtime/progress-recorded" && event.correlation_id.as_ref() == operation) {
        let native = payload(facts, event)?.get("event").ok_or_else(|| invalid("Runtime event is missing"))?;
        if native.get("event").and_then(Value::as_str) != Some("tool_completed") { continue; }
        if let AgentEngineEvent::ToolCompleted { result, .. } = decode(native)? {
            if result.is_error || result.call_id.as_ref() != admission.call_id { continue; }
            let result: Value = serde_json::from_str(&result.output_text()).map_err(|error| invalid(error.to_string()))?;
            let digest = digest_payload(&serde_json::json!({"result":result})).map_err(|error| invalid(error.to_string()))?;
            if owner.get("observation_digest").and_then(Value::as_str) != Some(digest.as_ref()) {
                return Err(invalid("expanded owner receipt digest differs"));
            }
            return Ok(Some(result));
        }
    }
    Ok(None)
}

/// Known explicit targets let the file owner prove net-zero changes without
/// recording an unbounded directory baseline in Session history.
pub(super) fn mutation_paths(facts: &ChatCausalityFacts, root_sha256: &str) -> Result<Vec<String>, AppError> {
    let receipts = receipts(facts)?;
    if !receipts.complete { return Err(invalid("settled file targets could not be fully resolved")); }
    Ok(receipts.files.into_iter().filter(|file| file.root == root_sha256 && file.published.is_none())
        .map(|file| file.path).collect::<BTreeSet<_>>().into_iter().collect())
}

#[derive(Clone)]
struct PresentedFile { item: TurnFileItem, seq: u64 }
struct Presentation { summary: TurnFileSummary, sources: BTreeMap<String, u64> }

fn merge_receipt(files: &mut BTreeMap<String, FileFact>, file: FileFact) {
    if file.proves_entry_absent {
        let prefix = format!("{}/", file.path);
        let descendants = files.iter().filter(|(_, known)| known.root == file.root && known.path.starts_with(&prefix))
            .map(|(identity, _)| identity.clone()).collect::<Vec<_>>();
        for identity in descendants {
            if files.get(&identity).is_some_and(|known| known.operation == FileChangeOperation::Create && known.published.is_none()) {
                files.remove(&identity);
            } else if let Some(known) = files.get_mut(&identity) {
                known.operation = FileChangeOperation::Delete;
                known.bytes = None;
                known.sha256 = None;
                known.seq = file.seq;
                known.proves_entry_absent = true;
            }
        }
    }
    let key = format!("{}:{}", file.root, file.path);
    if let Some(prior) = files.get_mut(&key) {
        if file.published.is_some() {
            prior.published = file.published;
            return;
        }
        let publication = prior.published.clone();
        let initial_sha256 = prior.initial_sha256.clone();
        let operation = match (prior.operation, file.operation) {
            (FileChangeOperation::Create, FileChangeOperation::Delete) if publication.is_none() => {
                files.remove(&key);
                return;
            }
            (FileChangeOperation::Create, FileChangeOperation::Modify) => FileChangeOperation::Create,
            (FileChangeOperation::Delete, FileChangeOperation::Create) => FileChangeOperation::Modify,
            _ => file.operation,
        };
        *prior = FileFact { operation, published: publication, initial_sha256, ..file };
    } else { files.insert(key, file); }
}

#[cfg(test)]
fn presentation(facts: &ChatCausalityFacts, fallback_workspace: Option<&str>) -> Result<Presentation, AppError> {
    presentation_with_roots(facts, fallback_workspace, BTreeMap::new(), receipts(facts)?)
}

fn presentation_with_roots(facts: &ChatCausalityFacts, fallback_workspace: Option<&str>,
    mut roots: BTreeMap<String, String>, mut receipts: ReceiptFacts) -> Result<Presentation, AppError> {
    let (turn_start, root) = start(facts)?;
    receipts.files.sort_by_key(|file| file.seq);
    let mut files = BTreeMap::new();
    if let Some(fallback) = fallback_workspace.filter(|value| Path::new(value).is_absolute()) {
        roots.entry(digest_bytes(fallback.as_bytes()).as_ref().to_owned()).or_insert(fallback.into());
    }
    let mut observations = facts.events.iter().filter(|event| event.kind.0 == TURN_WORKSPACE_FILES_OBSERVED)
        .collect::<Vec<_>>();
    observations.sort_by_key(|event| event.seq);
    let mut complete = true;
    let mut latest_generation = 0;
    let mut latest_observation_seq = None;
    let mut receipt_cursor = receipts.files.into_iter().peekable();
    let mut segment_base = BTreeMap::<String, FileFact>::new();
    for event in observations {
        let observation: TurnWorkspaceFilesObservedV1 = decode(payload(facts, event)?)?;
        observation.validate().map_err(invalid)?;
        if observation.operation_id.as_ref() != turn_start.correlation_id.as_ref()
            || event.correlation_id != turn_start.correlation_id || event.seq <= turn_start.seq
            || observation.execution_generation == 0
            || observation.execution_generation > facts.execution_generation
            || observation.execution_generation <= latest_generation
            || !Path::new(&observation.workspace_root).is_absolute()
            || digest_bytes(observation.workspace_root.as_bytes()).as_ref() != observation.workspace_root_sha256 {
            return Err(invalid("workspace observation identity differs from its Turn"));
        }
        roots.entry(observation.workspace_root_sha256.clone()).or_insert(observation.workspace_root.clone());
        latest_generation = observation.execution_generation;
        latest_observation_seq = Some(event.seq);
        complete &= observation.complete;
        while receipt_cursor.peek().is_some_and(|file| file.seq < event.seq) {
            merge_receipt(&mut files, receipt_cursor.next().expect("peeked receipt"));
        }
        // A segment's comparison replaces intermediate receipts in that
        // segment, then folds into the previous committed segments. Recovery
        // starts a new real baseline; it cannot erase earlier confirmed files
        // or claim that the unobserved restart interval was fully covered.
        for path in observation.unchanged_paths {
            let key = format!("{}:{}", observation.workspace_root_sha256, path);
            let current = files.remove(&key);
            if let Some(base) = segment_base.get(&key) {
                let mut restored = base.clone();
                if let Some(published) = current.and_then(|file| file.published) { restored.published = Some(published); }
                files.insert(key, restored);
            } else if let Some(current) = current.filter(|file| file.published.is_some()) {
                files.insert(key, current);
            }
        }
        for file in observation.files {
            let key = format!("{}:{}", observation.workspace_root_sha256, file.path);
            let published = files.get(&key).and_then(|file| file.published.clone());
            if let Some(base) = segment_base.get(&key) { files.insert(key.clone(), base.clone()); }
            else { files.remove(&key); }
            let observed = FileFact { root: observation.workspace_root_sha256.clone(), path: file.path,
                operation: match file.operation { ObservedFileOperation::Create => FileChangeOperation::Create,
                    ObservedFileOperation::Modify => FileChangeOperation::Modify, ObservedFileOperation::Delete => FileChangeOperation::Delete },
                bytes: file.size_bytes, sha256: file.sha256, published: None, seq: event.seq,
                initial_sha256: if observation.execution_generation == 1 { file.source_sha256 } else { None },
                proves_entry_absent: false };
            merge_receipt(&mut files, observed.clone());
            if let Some(published) = published {
                let item = files.entry(key).or_insert(observed);
                item.published = Some(published);
            }
        }
        segment_base = files.clone();
    }
    for file in receipt_cursor { merge_receipt(&mut files, file); }
    complete &= receipts.complete && latest_generation != 0 && latest_generation == facts.execution_generation;
    files.retain(|_, file| file.published.is_some() || file.operation == FileChangeOperation::Delete
        || latest_generation != facts.execution_generation || latest_observation_seq != Some(file.seq)
        || file.initial_sha256.as_ref().is_none_or(|initial| file.sha256.as_ref() != Some(initial)));
    let mut presented = files.into_iter().map(|(identity, file)| {
        let workspace = roots.get(&file.root);
        let absolute = workspace.map(|workspace| Path::new(workspace).join(&file.path).to_string_lossy().into_owned());
        let published = file.published.and_then(|published| workspace.map(|workspace| TurnPublishedFile {
            artifact_id: published.artifact_id, path: Path::new(workspace).join(published.relative_path).to_string_lossy().into_owned(),
            mime_type: published.mime_type, size_bytes: published.size_bytes, sha256: published.sha256,
        }));
        PresentedFile { item: TurnFileItem { identity, file_name: file.path.rsplit('/').next().unwrap_or(&file.path).into(),
            relative_path: file.path, path: absolute, operation: file.operation,
            size_bytes: file.bytes, sha256: file.sha256, published }, seq: file.seq }
    }).collect::<Vec<_>>();
    // Keep a deterministic bounded API even for Turns with thousands of explicit
    // file operations. Coverage communicates the limit instead of silently lying.
    if presented.len() > MAX_TURN_OBSERVED_FILES { complete = false; presented.truncate(MAX_TURN_OBSERVED_FILES); }
    let sources = presented.iter().map(|file| (file.item.identity.clone(), file.seq)).collect();
    Ok(Presentation { summary: TurnFileSummary { turn_id: root.into(), complete,
        files: presented.into_iter().map(|file| file.item).collect() }, sources })
}

pub(super) async fn query(store: &AgentSessionStore, session: &AgentSessionId, roots: &[String],
    fallback_workspace: Option<&str>, managed_root: &Path) -> Result<Vec<TurnFileSummary>, AppError> {
    if roots.len() > 100 { return Err(AppError::BadRequest("at most 100 Turn file sources are allowed".into())); }
    let mut presentations = Vec::with_capacity(roots.len());
    for root in roots {
        nomifun_common::validate_uuidv7(root).map_err(|error| AppError::BadRequest(error.to_string()))?;
        let facts = store.turn_file_facts_by_root(session, &EventId::from(root.clone())).await.map_err(store_error)?;
        let Some(facts) = facts else {
            continue;
        };
        if !has_file_presentation(&facts)? { continue; }
        let mut root_sources = BTreeMap::<u64, BTreeSet<String>>::new();
        let receipts = receipts(&facts)?;
        for file in &receipts.files { root_sources.entry(file.seq).or_default().insert(file.root.clone()); }
        for event in facts.events.iter().filter(|event| event.kind.0 == TURN_WORKSPACE_FILES_OBSERVED) {
            let observed: TurnWorkspaceFilesObservedV1 = decode(payload(&facts, event)?)?;
            observed.validate().map_err(invalid)?;
            root_sources.entry(event.seq).or_default().insert(observed.workspace_root_sha256);
        }
        let root_sources = root_sources.into_iter().collect::<Vec<_>>();
        let mut witnessed_roots = BTreeMap::new();
        for chunk in root_sources.chunks(nomifun_agent_session::MAX_EVENT_PAGE_SIZE as usize) {
            let candidates = chunk.iter().cloned().collect();
            for roots in store.history_workspace_roots(session, managed_root, &candidates).await.map_err(store_error)?.into_values() {
                for (hash, root) in roots {
                    if witnessed_roots.get(&hash).is_some_and(|previous| previous != &root) {
                        return Err(invalid("workspace root witnesses disagree inside one Turn"));
                    }
                    witnessed_roots.insert(hash, root);
                }
            }
        }
        let item = presentation_with_roots(&facts, fallback_workspace, witnessed_roots, receipts)?;
        if item.summary.turn_id != *root { return Err(invalid("query root differs from canonical Turn")); }
        presentations.push(item);
    }
    // Existing canonical relocation proofs map only witnessed managed paths.
    // Arbitrary old absolute paths and external projects are never reinterpreted.
    let mut candidates: BTreeMap<u64, BTreeSet<String>> = BTreeMap::new();
    for presentation in &presentations {
        for file in &presentation.summary.files {
            let seq = presentation.sources[&file.identity];
            let paths = candidates.entry(seq).or_default();
            paths.extend(file.path.iter().cloned());
            paths.extend(file.published.iter().map(|published| published.path.clone()));
        }
    }
    let mut relocated = BTreeMap::new();
    let candidates = candidates.into_iter().filter(|(_, paths)| !paths.is_empty()).collect::<Vec<_>>();
    for chunk in candidates.chunks(nomifun_agent_session::MAX_EVENT_PAGE_SIZE as usize) {
        let batch = chunk.iter().cloned().collect();
        relocated.extend(store.managed_history_file_paths(session, managed_root, &batch).await.map_err(store_error)?);
    }
    for presentation in &mut presentations {
        for file in &mut presentation.summary.files {
            let Some(paths) = relocated.get(&presentation.sources[&file.identity]) else { continue; };
            if let Some(current) = file.path.as_ref().and_then(|path| paths.get(path)) { file.path = Some(current.clone()); }
            if let Some(published) = &mut file.published {
                if let Some(current) = paths.get(&published.path) { published.path = current.clone(); }
            }
        }
    }
    Ok(presentations.into_iter().map(|item| item.summary).collect())
}
fn store_error(error: SessionStoreError) -> AppError {
    match error {
        SessionStoreError::NotFound(message) => AppError::NotFound(message),
        other => invalid(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{CorrelationId, EventProducerId, IdempotencyKey,
        SessionEventKind, SessionEventPayloadRef, StrictJsonValue};
    use serde_json::json;

    const SESSION: &str = "0190f5fe-7c00-7a00-8abc-012345678910";
    const ROOT: &str = "0190f5fe-7c00-7a00-8abc-012345678911";
    const TURN: &str = "turn:test";
    fn workspace() -> String { std::env::temp_dir().join("turn-file-presentation").to_string_lossy().into_owned() }
    fn root_hash() -> String { digest_bytes(workspace().as_bytes()).as_ref().into() }

    struct Fixture { facts: ChatCausalityFacts }
    impl Fixture {
        fn new() -> Self {
            let mut fixture = Self { facts: ChatCausalityFacts {
                session: decode(&json!({
                    "agent_session_id":SESSION,"owner_ref":{"principal_kind":"user","principal_id":"owner"},
                    "metadata":{"title":null,"archived":false,"pinned":false},
                    "agent_binding":{"preset_revision_ref":{"preset_id":"preset","revision":1,"revision_digest":"a".repeat(64)},
                        "resolved_snapshot_ref":{"snapshot_id":"snapshot","snapshot_digest":"b".repeat(64)},
                        "typed_resource_bindings":[],"binding_version":1},"next_seq":1001,
                })).unwrap(),
                head: nomifun_agent_session::SessionHeadProjection { session_id: SESSION.into(), status: "completed".into(),
                    active_turn_id: None, active_set_generation: 1, last_seq: 1000, unread_count: 0 },
                events: Vec::new(), event_payloads: BTreeMap::new(), operation_ids: BTreeSet::new(),
                turn_route_identities: BTreeSet::new(), execution_generation: 1, execution_fence: 1, fork_context: None,
            } };
            fixture.push("turn/started", TURN, None, json!({"operation_id":TURN,"source_message_id":ROOT}));
            fixture
        }
        fn push(&mut self, kind: &str, correlation: &str, cause: Option<EventId>, value: Value) -> SessionEventRecord {
            let seq = self.facts.events.len() as u64 + 1;
            let event = SessionEventRecord { agent_session_id: SESSION.into(), seq,
                event_id: EventId::from(format!("event:{seq}")), producer_id: EventProducerId::from("test"),
                idempotency_key: IdempotencyKey::from(format!("event:{seq}")), kind: SessionEventKind(kind.into()), kind_version: 1,
                correlation_id: CorrelationId::from(correlation), causation_event_id: cause,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(value.clone())) };
            self.facts.event_payloads.insert(event.event_id.as_ref().into(), value);
            self.facts.events.push(event.clone());
            event
        }
        fn effect(&mut self, action: &str, result: Value) {
            let operation = format!("effect-operation:{}", self.facts.events.len());
            let effect = format!("effect:{operation}");
            let start = self.facts.events[0].event_id.clone();
            let call = self.push("tool/call-started", &operation, Some(start), json!({
                "operation_id":operation,"call_id":operation,"capability_id":"workspace.files","action_id":action,
            }));
            let mut identity = json!({"effect_id":effect,"turn_id":TURN,"operation_id":operation,
                "capability_module":"workspace.files","action_id":action,"input_digest":"a".repeat(64),
                "resource_binding_id":"workspace","resource_key":"workspace","owner_domain":"file","strategy":"managed_effect"});
            let started = self.push("effect/started", &effect, Some(call.event_id), identity.clone());
            identity["result"] = result;
            self.push("effect/succeeded", &effect, Some(started.event_id), identity);
        }
        fn write(&mut self, path: &str, created: bool) {
            self.effect("workspace.files/write", json!({"path":path,"written":true,"created":created,
                "workspace_path":{"root_sha256":root_hash(),"path":path,"case_resolved":false},"bytes":42,"sha256":"d".repeat(64)}));
        }
        fn delete(&mut self, path: &str) {
            self.effect("workspace.files/delete", json!({"path":path,"deleted":true,
                "workspace_path":{"root_sha256":root_hash(),"path":path,"case_resolved":false}}));
        }
        fn publish(&mut self, path: &str) {
            self.effect("workspace.artifacts/publish", json!({"workspace_root_sha256":root_hash(),
                "artifact_id":"d".repeat(64),"source_path":path,
                "relative_path":format!("{}/{}",nomifun_file::ARTIFACT_RELATIVE_ROOT,"d".repeat(64)),
                "mime_type":"application/octet-stream","size_bytes":42,"sha256":"d".repeat(64)}));
        }
        fn observe(&mut self, files: Value, unchanged: Value, complete: bool) {
            self.push(TURN_WORKSPACE_FILES_OBSERVED, TURN, Some(self.facts.events[0].event_id.clone()), json!({
                "operation_id":TURN,"execution_generation":self.facts.execution_generation,"workspace_root":workspace(),
                "workspace_root_sha256":root_hash(),"complete":complete,"files":files,"unchanged_paths":unchanged,
            }));
        }
        fn show(&self) -> TurnFileSummary { presentation(&self.facts, Some(&workspace())).unwrap().summary }
    }

    #[test]
    fn receipts_and_command_observation_show_script_and_docx_without_reading_disk() {
        let mut f = Fixture::new();
        f.write("build_docx.py", true);
        f.observe(json!([
            {"path":"build_docx.py","operation":"create","size_bytes":42,"sha256":"d".repeat(64)},
            {"path":"骆驼祥子读后感.docx","operation":"create","size_bytes":1234,"sha256":"e".repeat(64)},
        ]), json!([]), true);
        let summary = f.show();
        assert!(summary.complete);
        assert_eq!(summary.files.len(), 2);
        assert_eq!(summary.files[1].file_name, "骆驼祥子读后感.docx");
        assert_eq!(summary.files[1].size_bytes, Some(1234));
        assert_eq!(summary.turn_id, ROOT);
    }

    #[test]
    fn failed_turn_keeps_deleted_files_and_explicit_excluded_receipts() {
        let mut f = Fixture::new();
        f.write("node_modules/generated.txt", true);
        f.delete("old.docx");
        f.observe(json!([{"path":"old.docx","operation":"delete"}]), json!([]), true);
        f.push("turn/failed", TURN, None, json!({"operation_id":TURN}));
        let summary = f.show();
        assert_eq!(summary.files.len(), 2);
        assert_eq!(summary.files[1].operation, FileChangeOperation::Delete);
        assert_eq!(summary.files[1].size_bytes, None);
        assert!(summary.files[1].path.is_some());
    }

    #[test]
    fn proven_net_zero_suppresses_receipts_but_retains_explicit_publication() {
        let mut f = Fixture::new();
        f.write("restored.txt", false);
        f.write("temporary.txt", true);
        f.delete("temporary.txt");
        f.publish("delivered.docx");
        f.observe(json!([]), json!(["restored.txt","delivered.docx"]), true);
        let summary = f.show();
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].file_name, "delivered.docx");
        assert!(summary.files[0].published.is_some());
    }

    #[test]
    fn identical_content_and_basenames_do_not_collapse_distinct_paths() {
        let mut f = Fixture::new();
        f.write("one/report.docx", true);
        f.write("two/report.docx", true);
        f.publish("one/report.docx");
        f.publish("two/report.docx");
        let files = f.show().files;
        assert_eq!(files.len(), 2);
        assert_ne!(files[0].identity, files[1].identity);
        assert_eq!(files[0].published, files[1].published);
    }

    #[test]
    fn observation_overrides_intermediate_write_with_final_command_deletion() {
        let mut f = Fixture::new();
        f.write("report.txt", false);
        f.observe(json!([{"path":"report.txt","operation":"delete"}]), json!([]), true);
        let files = f.show().files;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].operation, FileChangeOperation::Delete);
        assert_eq!(files[0].sha256, None);
    }

    #[test]
    fn fallback_workspace_never_assigns_another_roots_file() {
        let mut f = Fixture::new();
        f.write("report.txt", true);
        let other = std::env::temp_dir().join("other").to_string_lossy().into_owned();
        let summary = presentation(&f.facts, Some(&other)).unwrap().summary;
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].path, None);
        assert!(!summary.complete);
    }

    #[test]
    fn rejects_foreign_observation_generation_or_corrupted_effect_causation() {
        let mut f = Fixture::new();
        f.observe(json!([]), json!([]), true);
        f.facts.execution_generation = 0;
        assert!(presentation(&f.facts, None).is_err());
        let mut f = Fixture::new();
        f.write("report.txt", true);
        f.facts.events.last_mut().unwrap().causation_event_id = None;
        assert!(presentation(&f.facts, None).is_err());
    }

    #[test]
    fn mutation_paths_returns_only_owner_mutation_targets_for_this_root() {
        let mut f = Fixture::new();
        f.write("script.py", true);
        f.write("script.py", false);
        f.publish("deliverable.docx");
        f.delete("old.docx");
        assert_eq!(mutation_paths(&f.facts, &root_hash()).unwrap(), vec!["old.docx", "script.py"]);
        assert!(mutation_paths(&f.facts, &"f".repeat(64)).unwrap().is_empty());
    }

    #[test]
    fn resumed_turn_folds_both_observed_segments_and_original_owner_receipts() {
        let mut f = Fixture::new();
        f.write("before-pause.txt", true);
        f.observe(json!([{"path":"stale-command.txt","operation":"create"}]), json!([]), true);
        f.facts.execution_generation = 2;
        f.observe(json!([{"path":"after-resume.docx","operation":"create"}]), json!([]), false);
        let summary = f.show();
        assert!(!summary.complete);
        assert_eq!(summary.files.iter().map(|file| file.relative_path.as_str()).collect::<Vec<_>>(),
            vec!["after-resume.docx", "before-pause.txt", "stale-command.txt"]);
    }

    #[test]
    fn missing_final_generation_observation_retains_confirmed_files_as_incomplete() {
        let mut f = Fixture::new();
        f.write("owner.txt", true);
        f.observe(json!([{"path":"pause-only.docx","operation":"create"}]), json!([]), true);
        f.facts.execution_generation = 2;
        let summary = f.show();
        assert!(!summary.complete);
        assert_eq!(summary.files.len(), 2);
        assert_eq!(summary.files[0].relative_path, "owner.txt");
        assert_eq!(summary.files[1].relative_path, "pause-only.docx");
    }

    #[test]
    fn resumed_segment_net_deletion_removes_prior_creation_and_restore_keeps_prior_change() {
        let mut f = Fixture::new();
        f.observe(json!([
            {"path":"temporary.docx","operation":"create"},
            {"path":"retained.txt","operation":"create","sha256":"d".repeat(64)},
        ]), json!([]), true);
        f.facts.execution_generation = 2;
        f.write("retained.txt", false);
        f.observe(json!([{"path":"temporary.docx","operation":"delete"}]), json!(["retained.txt"]), false);
        let summary = f.show();
        assert!(!summary.complete);
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].relative_path, "retained.txt");
        assert_eq!(summary.files[0].operation, FileChangeOperation::Create);
        assert_eq!(summary.files[0].sha256, Some("d".repeat(64)));
    }

    #[test]
    fn later_owner_delete_is_not_overwritten_by_an_earlier_observation() {
        let mut f = Fixture::new();
        f.observe(json!([{"path":"report.docx","operation":"modify"}]), json!([]), true);
        f.facts.execution_generation = 2;
        f.delete("report.docx");
        f.observe(json!([]), json!([]), false);
        let summary = f.show();
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].operation, FileChangeOperation::Delete);
    }

    #[test]
    fn cross_resume_restoration_uses_only_proven_first_generation_source_bytes() {
        let mut f = Fixture::new();
        f.observe(json!([{"path":"restored.txt","operation":"modify","source_sha256":"a".repeat(64),
            "sha256":"b".repeat(64)}]), json!([]), true);
        f.facts.execution_generation = 2;
        f.observe(json!([{"path":"restored.txt","operation":"modify","source_sha256":"b".repeat(64),
            "sha256":"a".repeat(64)}]), json!([]), false);
        let summary = f.show();
        assert!(!summary.complete);
        assert!(summary.files.is_empty());
        let mut unknown = Fixture::new();
        unknown.observe(json!([{"path":"unknown.txt","operation":"modify","sha256":"b".repeat(64)}]), json!([]), true);
        unknown.facts.execution_generation = 2;
        unknown.observe(json!([{"path":"unknown.txt","operation":"modify","source_sha256":"a".repeat(64),
            "sha256":"a".repeat(64)}]), json!([]), false);
        assert_eq!(unknown.show().files.len(), 1, "resume baseline cannot become the unknown initial Turn version");
    }

    #[test]
    fn restoration_is_not_inferred_from_an_intermediate_receipt_without_final_file_observation() {
        let mut f = Fixture::new();
        f.observe(json!([{"path":"uncertain.txt","operation":"modify","source_sha256":"a".repeat(64),
            "sha256":"b".repeat(64)}]), json!([]), true);
        f.facts.execution_generation = 2;
        f.effect("workspace.files/write", json!({"path":"uncertain.txt","written":true,"created":false,
            "workspace_path":{"root_sha256":root_hash(),"path":"uncertain.txt","case_resolved":false},
            "bytes":42,"sha256":"a".repeat(64)}));
        // Final capture could not observe this path after later opaque work.
        f.observe(json!([]), json!([]), false);
        assert_eq!(f.show().files.len(), 1);
    }

    #[test]
    fn recursive_native_delete_removes_created_descendants_and_keeps_only_its_known_target() {
        let mut f = Fixture::new();
        f.write("node_modules/cache/a.txt", true);
        f.write("node_modules/cache-adjacent/keep.txt", true);
        f.delete("node_modules/cache");
        let files = f.show().files;
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|file| file.relative_path == "node_modules/cache" && file.operation == FileChangeOperation::Delete));
        assert!(files.iter().any(|file| file.relative_path == "node_modules/cache-adjacent/keep.txt" && file.operation == FileChangeOperation::Create));
        assert!(!files.iter().any(|file| file.relative_path == "node_modules/cache/a.txt"));
    }

    #[test]
    fn recursive_delete_marks_modified_and_published_children_as_deleted_without_crossing_roots() {
        let mut f = Fixture::new();
        f.write("node_modules/cache/old.txt", false);
        f.write("node_modules/cache/delivered.txt", true);
        f.publish("node_modules/cache/delivered.txt");
        f.delete("node_modules/cache");
        let files = f.show().files;
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|file| file.operation == FileChangeOperation::Delete && file.size_bytes.is_none() && file.sha256.is_none()));
        assert!(files.iter().find(|file| file.relative_path.ends_with("delivered.txt")).unwrap().published.is_some());
        let mut facts = BTreeMap::new();
        let own = observed_file(WorkspacePathObservation { root_sha256:root_hash(), path:"cache/own.txt".into(), case_resolved:false },
            FileChangeOperation::Modify, Some(1), None, 1).unwrap();
        let foreign = observed_file(WorkspacePathObservation { root_sha256:"f".repeat(64), path:"cache/foreign.txt".into(), case_resolved:false },
            FileChangeOperation::Create, Some(1), None, 2).unwrap();
        merge_receipt(&mut facts, own);
        merge_receipt(&mut facts, foreign);
        let mut removed = observed_file(WorkspacePathObservation { root_sha256:root_hash(), path:"cache".into(), case_resolved:false },
            FileChangeOperation::Delete, None, None, 3).unwrap();
        removed.proves_entry_absent = true;
        merge_receipt(&mut facts, removed);
        assert_eq!(facts[&format!("{}:cache/foreign.txt", "f".repeat(64))].operation, FileChangeOperation::Create);
    }

    #[test]
    fn snapshot_removed_regular_file_does_not_delete_children_of_a_replacement_directory() {
        let mut f = Fixture::new();
        f.write("old.txt/kept.txt", true);
        f.observe(json!([{"path":"old.txt","operation":"delete","source_sha256":"a".repeat(64)}]), json!([]), true);
        let files = f.show().files;
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|file| file.relative_path == "old.txt/kept.txt" && file.operation == FileChangeOperation::Create));
    }

    #[test]
    fn old_pure_chat_and_read_only_tools_have_no_file_presentation() {
        let mut f = Fixture::new();
        f.push("runtime/progress-recorded", TURN, None, json!({"event":{"event":"model_step_started","step":1,"operation_id":"model"}}));
        let root = f.facts.events[0].event_id.clone();
        for action in ["workspace.files/read", "workspace.process/poll", "workspace.process/resize", "tools/search"] {
            f.push("tool/call-started", "read-control", Some(root.clone()), json!({"operation_id":"read-control",
                "call_id":"read-control","capability_id":"workspace.files","action_id":action}));
        }
        assert!(!has_file_presentation(&f.facts).unwrap());
    }

    #[test]
    fn old_admitted_command_only_turn_keeps_partial_file_summary_without_guessing_paths() {
        let mut f = Fixture::new();
        let root = f.facts.events[0].event_id.clone();
        f.push("tool/call-started", "command", Some(root), json!({"operation_id":"command","call_id":"command",
            "capability_id":"workspace.process","action_id":"workspace.process/exec"}));
        assert!(has_file_presentation(&f.facts).unwrap());
        assert!(!f.show().complete);
        assert!(f.show().files.is_empty());
    }

    #[test]
    fn new_degraded_observation_keeps_empty_partial_file_presentation() {
        let mut f = Fixture::new();
        f.observe(json!([]), json!([]), false);
        assert!(has_file_presentation(&f.facts).unwrap());
        assert!(!f.show().complete);
        assert!(f.show().files.is_empty());
    }

    #[test]
    fn truncated_owner_receipt_requires_the_exact_persisted_output_digest() {
        let mut f = Fixture::new();
        f.write("report.docx", true);
        let terminal_id = f.facts.events.last().unwrap().event_id.as_ref().to_owned();
        let owner = f.facts.event_payloads.get_mut(&terminal_id).unwrap();
        let result = owner.as_object_mut().unwrap().remove("result").unwrap();
        let digest = digest_payload(&json!({"result":result})).unwrap();
        owner["observation_truncated"] = json!(true);
        owner["observation_digest"] = json!(digest);
        // Missing Runtime delivery does not fabricate a result from its preview.
        assert!(f.show().files.is_empty());
        assert!(!f.show().complete);
        let operation = f.facts.event_payloads.values().find_map(|value|
            value.get("call_id").and_then(Value::as_str)).unwrap().to_owned();
        f.push("runtime/progress-recorded", TURN, None, json!({"event":AgentEngineEvent::ToolCompleted {
            step:1, result:nomifun_agent_runtime::AgentToolResult::text(operation.into(), result.to_string(), false),
        }}));
        assert_eq!(f.show().files.len(), 1);
        f.facts.event_payloads.get_mut(&terminal_id).unwrap()["observation_digest"] = json!("f".repeat(64));
        assert!(presentation(&f.facts, Some(&workspace())).is_err());
    }

    #[test]
    fn large_explicit_change_lists_are_bounded_and_marked_incomplete() {
        let mut f = Fixture::new();
        for n in 0..=MAX_TURN_OBSERVED_FILES { f.write(&format!("file-{n}.txt"), true); }
        f.observe(json!([]), json!([]), true);
        let summary = f.show();
        assert_eq!(summary.files.len(), MAX_TURN_OBSERVED_FILES);
        assert!(!summary.complete);
    }
}
