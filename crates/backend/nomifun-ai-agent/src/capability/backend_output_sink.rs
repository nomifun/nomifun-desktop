use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nomi_agent::output::{
    ArtifactContract, ArtifactExpectation, ArtifactRequirement, OutputSink, ToolMediaDelivery,
    ToolCallExecutionContext, ToolCallRetryContext, artifact_contract,
    artifact_contract_with_input, is_context_only_image_tool,
};
use nomi_types::tool::ToolImage;
use same_file::Handle as SameFileHandle;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::artifact_store::{
    ArtifactKind, ArtifactRecoveryEnvelope, ArtifactRecoverySource, ArtifactStore,
    PersistedArtifact, VerifiedExistingArtifactSource,
};
use crate::protocol::events::{
    AgentStatusEventData, AgentStreamEvent, ErrorEventData, FinishEventData, PlanEventData,
    OutputDiscardedEventData, StartEventData, TextEventData, ThinkingEventData, TipType,
    TipsEventData, ToolCallEventData, ToolCallRetryData, ToolCallStatus,
};

pub struct BackendOutputSink {
    event_tx: broadcast::Sender<AgentStreamEvent>,
    /// File-based memory directory for citation reflow. `None` = this session
    /// does not participate (companion sessions, or no base dir).
    distill_dir: Option<PathBuf>,
    /// Workspace-scoped verified store for binary tool outputs. A desktop
    /// session wires this unconditionally; `None` is retained for lightweight
    /// unit/companion sinks and causes media delivery to fail closed.
    artifact_store: Option<ArtifactStore>,
    artifact_workspace: Option<PathBuf>,
    /// Accumulates this turn's assistant text so the `<nomi-mem-citation>`
    /// block can be parsed at stream end. Reset only at the accepted-turn
    /// boundary; race-tail provider Starts remain part of the same buffer.
    turn_text: Mutex<String>,
    /// Immutable boundary captured by the first Start of this accepted turn.
    /// Host commit rollback uses it to retract every provider pass.
    accepted_turn_output_checkpoint: Mutex<Option<SinkOutputCheckpoint>>,
    /// Rolling boundary for B1/internal provider-attempt rollback.
    attempt_output_checkpoint: Mutex<Option<SinkOutputCheckpoint>>,
    /// Schema-valid, committed tool calls announced to the frontend that have
    /// not yet produced a result. Unexpected termination and cancellation drain
    /// this map so no Running lifecycle can leak into a later turn.
    active_tool_calls: Mutex<HashMap<String, ActiveToolCall>>,
    /// Per-result context supplied by the engine. Pre-dispatch validation
    /// failures never enter `active_tool_calls`, so this short-lived map keeps
    /// their original args and retry identity through the legacy artifact
    /// delivery implementation.
    tool_result_contexts: Mutex<HashMap<String, ToolTerminalContext>>,
    /// Accepted-user-turn artifact obligations. Provider sub-streams and
    /// automatic continuations share this state; only the manager's accepted
    /// turn boundary begins/seals it.
    artifact_delivery_turn: Mutex<ArtifactDeliveryTurn>,
}

#[derive(Debug, Clone, Copy)]
struct SinkOutputCheckpoint {
    turn_text_len: usize,
    held_text_len: usize,
}

#[derive(Debug, Clone)]
struct ActiveToolCall {
    call_id: String,
    name: String,
    artifact_identity: String,
    args: serde_json::Value,
    input: Option<serde_json::Value>,
    contract: Option<ArtifactContract>,
    contract_error: Option<String>,
    artifact_path_baselines: ArtifactPathBaselines,
    retry: Option<ToolCallRetryData>,
}

#[derive(Debug, Clone)]
struct ToolTerminalContext {
    args: serde_json::Value,
    input: Option<serde_json::Value>,
    retry: Option<ToolCallRetryData>,
}

const MAX_DECLARED_ARTIFACT_PATHS: usize = 32;
const MAX_DECLARED_PATH_LENGTH: usize = 4096;
const MAX_ARTIFACT_OUTPUT_JSON_NODES: usize = 512;
/// Synchronous pre-call hashing runs on the model stream callback. Keep it
/// tightly bounded: newly-created outputs use an `Absent` baseline and do not
/// consume this budget, while overwriting a larger existing artifact must use
/// a fresh output path instead of stalling the runtime before tool dispatch.
const MAX_BASELINE_ARTIFACT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BASELINE_ARTIFACT_BATCH_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
struct ArtifactPathBaselines {
    entries: Vec<ArtifactPathBaseline>,
    errors: Vec<String>,
}

impl ArtifactPathBaselines {
    fn declares_artifact(&self) -> bool {
        !self.entries.is_empty() || !self.errors.is_empty()
    }
}

#[derive(Debug, Clone)]
struct ArtifactPathBaseline {
    path: PathBuf,
    fingerprint: ArtifactPathFingerprint,
}

#[derive(Debug, Clone)]
enum ArtifactPathFingerprint {
    Absent,
    Present { size_bytes: u64, sha256: String },
}

#[derive(Debug, Clone, Default)]
struct DeclaredArtifactPaths {
    paths: Vec<String>,
    saw_explicit_key: bool,
    errors: Vec<String>,
    resource_limit_errors: Vec<String>,
}

impl DeclaredArtifactPaths {
    fn push_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        if !self.errors.iter().any(|known| known == &error) {
            self.errors.push(error);
        }
    }

    fn push_resource_limit_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        if !self
            .resource_limit_errors
            .iter()
            .any(|known| known == &error)
        {
            self.resource_limit_errors.push(error);
        }
    }

    fn has_artifact_signal(&self) -> bool {
        self.saw_explicit_key || !self.paths.is_empty() || !self.errors.is_empty()
    }

    /// Resource limits protect artifact-contract parsing; they are not, by
    /// themselves, evidence that an ordinary JSON tool result is an artifact.
    /// Promote them to delivery errors only after an artifact signal or an
    /// existing contract makes this scan security-sensitive.
    fn enforce_resource_limits_if_artifact_expected(&mut self, artifact_expected: bool) {
        if !artifact_expected && !self.has_artifact_signal() {
            return;
        }
        for error in std::mem::take(&mut self.resource_limit_errors) {
            self.push_error(error);
        }
    }
}

#[derive(Debug, Default)]
struct ArtifactDeliveryTurn {
    active: bool,
    /// Monotonic ledger revision. Blocking persistence/re-verification works
    /// from a snapshot and may publish its result only when this value still
    /// matches, preventing a cancelled or successor turn from inheriting it.
    generation: u64,
    /// Production Nomi turns defer every artifact persistence and successful
    /// terminal card to the manager's cancellable commit phase. The synchronous
    /// mode remains for lightweight sink tests and non-manager callers.
    defer_artifact_terminals: bool,
    calls: HashMap<String, ArtifactCallObligation>,
    /// Host-routed output requirement for the accepted user turn. Unlike a
    /// call obligation this exists before the model chooses a tool, so a model
    /// cannot satisfy an image-generation request with text alone (or with an
    /// observational Browser screenshot that deliberately has no receipt).
    required_contract: Option<ArtifactContract>,
    /// Assistant prose for a host-routed artifact turn stays provisional until
    /// the required receipt survives final re-verification. This closes the
    /// last false-success window: a provider cannot stream "generated" to the
    /// UI and only afterwards fail the durable-artifact gate.
    hold_text_until_verified: bool,
    held_text: String,
    /// Exact lossy broadcast wire that owns a recoverable receipt envelope.
    /// The relay later binds this to its stable root turn and durable row ID.
    recovery_source: Option<ArtifactRecoverySource>,
}

#[derive(Debug)]
struct ArtifactCallObligation {
    tool_name: String,
    contract: ArtifactContract,
    status: ArtifactCallDeliveryStatus,
}

#[derive(Debug)]
enum ArtifactCallDeliveryStatus {
    Running,
    Pending(PendingArtifactDelivery),
    Persisting(DeferredToolResult),
    CompletedVerified {
        artifacts: Vec<PersistedArtifact>,
        /// Deferred artifact cards are not published until final generation-CAS
        /// commit. `None` means the synchronous path already emitted its card.
        deferred_terminal: Option<DeferredToolResult>,
    },
    Failed(String),
}

#[derive(Debug)]
struct PendingArtifactDelivery {
    inline: Vec<OwnedInlineArtifact>,
    existing_sources: Vec<VerifiedExistingArtifactSource>,
    terminal: DeferredToolResult,
}

#[derive(Debug)]
struct OwnedInlineArtifact {
    kind: ArtifactKind,
    mime_type: String,
    data: String,
}

#[derive(Debug, Clone)]
struct DeferredToolResult {
    call_id: String,
    name: String,
    content: String,
    context: Option<ToolTerminalContext>,
}

#[derive(Debug)]
struct PendingArtifactWork {
    call_id: String,
    contract: ArtifactContract,
    pending: PendingArtifactDelivery,
}

#[derive(Debug)]
struct PersistedPendingArtifact {
    call_id: String,
    artifacts: Vec<PersistedArtifact>,
}

#[derive(Debug, Clone)]
struct ArtifactVerificationSnapshot {
    generation: u64,
    receipts: Vec<ArtifactReceiptVerification>,
}

#[derive(Debug, Clone)]
struct ArtifactReceiptVerification {
    tool_name: String,
    contract_label: &'static str,
    artifact: PersistedArtifact,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VerifiedArtifactDeliveryTurn {
    generation: u64,
}

#[derive(Debug)]
pub(crate) enum AsyncArtifactDeliveryOutcome {
    Verified(VerifiedArtifactDeliveryTurn),
    Cancelled,
    Failed(String),
}

impl ArtifactDeliveryTurn {
    fn advance_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }
}

fn any_artifact_contract() -> ArtifactContract {
    ArtifactContract {
        expectation: ArtifactExpectation::Any,
        requirement: ArtifactRequirement::Any,
        requested_count: None,
    }
}

fn normalized_path_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_explicit_artifact_path_key(key: &str) -> bool {
    let normalized = normalized_path_key(key);
    const PREFIXES: &[&str] = &[
        "output",
        "outputs",
        "artifact",
        "artifacts",
        "result",
        "results",
        "save",
        "saves",
        "destination",
        "destinations",
    ];
    const SUFFIXES: &[&str] = &[
        "path",
        "paths",
        "file",
        "files",
        "filepath",
        "filepaths",
    ];
    PREFIXES.iter().any(|prefix| {
        normalized
            .strip_prefix(prefix)
            .is_some_and(|suffix| SUFFIXES.contains(&suffix))
    })
}

fn is_unambiguous_artifact_path_key(key: &str) -> bool {
    let normalized = normalized_path_key(key);
    const PREFIXES: &[&str] = &[
        "output",
        "outputs",
        "artifact",
        "artifacts",
        "result",
        "results",
        "save",
        "saves",
        "destination",
        "destinations",
    ];
    // Plural `*files` fields are frequently read-model history (for example
    // execution attempt `output_files`). A singular file or an explicit path
    // suffix is strong enough to recognize inside a root array item.
    const SUFFIXES: &[&str] = &["path", "paths", "file", "filepath", "filepaths"];
    PREFIXES.iter().any(|prefix| {
        normalized
            .strip_prefix(prefix)
            .is_some_and(|suffix| SUFFIXES.contains(&suffix))
    })
}

fn is_plain_path_key(key: &str) -> bool {
    matches!(normalized_path_key(key).as_str(), "path" | "paths")
}

fn is_result_scope(key: &str) -> bool {
    matches!(
        normalized_path_key(key).as_str(),
        "result" | "results" | "output" | "outputs" | "artifact" | "artifacts"
    )
}

fn is_blocked_path_scope(key: &str) -> bool {
    matches!(
        normalized_path_key(key).as_str(),
        "input"
            | "inputs"
            | "source"
            | "sources"
            | "request"
            | "requests"
            | "argument"
            | "arguments"
            | "arg"
            | "args"
            | "parameter"
            | "parameters"
    )
}

fn is_blocked_input_read_model_scope(key: &str) -> bool {
    matches!(
        normalized_path_key(key).as_str(),
        "filter"
            | "filters"
            | "query"
            | "queries"
            | "where"
            | "history"
            | "histories"
            | "attempt"
            | "attempts"
    )
}

fn push_declared_path(declared: &mut DeclaredArtifactPaths, value: &str) {
    let value = value
        .trim()
        .trim_matches(|character| matches!(character, '`' | '"' | '\''));
    if value.is_empty() {
        declared.push_error("declared artifact path is empty");
        return;
    }
    if value.len() > MAX_DECLARED_PATH_LENGTH {
        declared.push_error(format!(
            "declared artifact path exceeds {MAX_DECLARED_PATH_LENGTH} bytes"
        ));
        return;
    }
    if value.chars().any(char::is_control) {
        declared.push_error("declared artifact path contains a control character");
        return;
    }
    if declared.paths.iter().any(|known| known == value) {
        return;
    }
    if declared.paths.len() >= MAX_DECLARED_ARTIFACT_PATHS {
        declared.push_error(format!(
            "artifact contract declares more than {MAX_DECLARED_ARTIFACT_PATHS} distinct paths"
        ));
        return;
    }
    declared.paths.push(value.to_owned());
}

fn collect_path_value(value: &serde_json::Value, declared: &mut DeclaredArtifactPaths) {
    match value {
        serde_json::Value::String(value) => push_declared_path(declared, value),
        serde_json::Value::Array(values) => {
            for value in values {
                if let Some(value) = value.as_str() {
                    push_declared_path(declared, value);
                } else {
                    declared.push_error(
                        "declared artifact path list contains a non-string value",
                    );
                }
            }
        }
        _ => declared.push_error("declared artifact path value is not a string or string list"),
    }
}

fn collect_json_artifact_paths(
    value: &serde_json::Value,
    declared: &mut DeclaredArtifactPaths,
    nodes: &mut usize,
    depth: usize,
    allow_plain_path: bool,
    explicit_paths_require_result_scope: bool,
    at_result_envelope: bool,
    at_root_array_item: bool,
) {
    if depth > 12 {
        declared.push_resource_limit_error("artifact contract JSON nesting exceeds 12 levels");
    }
    if *nodes >= MAX_ARTIFACT_OUTPUT_JSON_NODES {
        declared.push_resource_limit_error(format!(
            "artifact contract JSON exceeds {MAX_ARTIFACT_OUTPUT_JSON_NODES} nodes"
        ));
    } else {
        *nodes += 1;
    }

    // Continue walking object/array structure after the artifact parsing
    // budget is exhausted. This is necessary to distinguish a large ordinary
    // JSON result from a large result that contains a real explicit artifact
    // declaration after the limit. Path collection remains bounded separately.
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if is_blocked_path_scope(key) {
                    continue;
                }
                // Output-shaped field names are common inside ordinary nested
                // domain data (for example execution attempts expose an
                // `output_files` history field). Treat them as an artifact
                // declaration only at the response root or beneath an
                // explicit result/output/artifact envelope. This preserves
                // output-only contracts without reclassifying read-model
                // fields as files produced by the query itself.
                if is_explicit_artifact_path_key(key)
                    && (!explicit_paths_require_result_scope
                        || depth == 0
                        || at_result_envelope
                        || (at_root_array_item
                            && is_unambiguous_artifact_path_key(key)))
                {
                    declared.saw_explicit_key = true;
                    collect_path_value(child, declared);
                } else if allow_plain_path
                    && is_plain_path_key(key)
                    && (depth == 0 || at_result_envelope)
                {
                    collect_path_value(child, declared);
                }
                collect_json_artifact_paths(
                    child,
                    declared,
                    nodes,
                    depth + 1,
                    allow_plain_path,
                    explicit_paths_require_result_scope,
                    is_result_scope(key),
                    false,
                );
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_json_artifact_paths(
                    child,
                    declared,
                    nodes,
                    depth + 1,
                    allow_plain_path,
                    explicit_paths_require_result_scope,
                    at_result_envelope,
                    at_root_array_item
                        || (explicit_paths_require_result_scope && depth == 0),
                );
            }
        }
        _ => {}
    }
}

fn collect_input_artifact_paths(
    value: &serde_json::Value,
    declared: &mut DeclaredArtifactPaths,
    nodes: &mut usize,
    depth: usize,
    allow_nested: bool,
    at_direct_options: bool,
) {
    if depth > 12 {
        declared.push_resource_limit_error("artifact contract JSON nesting exceeds 12 levels");
    }
    if *nodes >= MAX_ARTIFACT_OUTPUT_JSON_NODES {
        declared.push_resource_limit_error(format!(
            "artifact contract JSON exceeds {MAX_ARTIFACT_OUTPUT_JSON_NODES} nodes"
        ));
    } else {
        *nodes += 1;
    }

    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if is_blocked_path_scope(key) || is_blocked_input_read_model_scope(key) {
                    continue;
                }
                let direct_options = depth == 0 && normalized_path_key(key) == "options";
                if is_explicit_artifact_path_key(key)
                    && (allow_nested || depth == 0 || at_direct_options)
                {
                    declared.saw_explicit_key = true;
                    collect_path_value(child, declared);
                }
                // Ordinary tools may declare a destination only at the input
                // root or immediately under the conventional `options`
                // object. Deep output-shaped fields are commonly read-model
                // filters/history, not files this invocation promises to make.
                if allow_nested || direct_options {
                    collect_input_artifact_paths(
                        child,
                        declared,
                        nodes,
                        depth + 1,
                        allow_nested,
                        direct_options,
                    );
                }
            }
        }
        serde_json::Value::Array(values) if allow_nested => {
            for child in values {
                collect_input_artifact_paths(
                    child,
                    declared,
                    nodes,
                    depth + 1,
                    allow_nested,
                    false,
                );
            }
        }
        _ => {}
    }
}

fn input_artifact_paths(
    value: &serde_json::Value,
    allow_nested: bool,
) -> DeclaredArtifactPaths {
    let mut declared = DeclaredArtifactPaths::default();
    let mut nodes = 0;
    collect_input_artifact_paths(
        value,
        &mut declared,
        &mut nodes,
        0,
        allow_nested,
        false,
    );
    declared.enforce_resource_limits_if_artifact_expected(false);
    declared
}

fn output_artifact_paths(content: &str, allow_plain_path: bool) -> DeclaredArtifactPaths {
    let mut declared = DeclaredArtifactPaths::default();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) {
        let mut nodes = 0;
        collect_json_artifact_paths(
            &value,
            &mut declared,
            &mut nodes,
            0,
            allow_plain_path,
            true,
            false,
            false,
        );
    }

    // A number of native/export tools return a human-readable locator rather
    // than JSON. Only accept explicit output labels; arbitrary prose and input
    // paths are deliberately ignored.
    const LABELS: &[&str] = &[
        "saved to:",
        "output path:",
        "output file:",
        "artifact path:",
        "artifact file:",
        "result path:",
        "result file:",
        "destination path:",
        "destination file:",
    ];
    for line in content.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        for label in LABELS {
            if lower.starts_with(label) {
                declared.saw_explicit_key = true;
                push_declared_path(&mut declared, &trimmed[label.len()..]);
                break;
            }
        }
    }
    declared
}

fn artifact_candidate_path(value: &str) -> Result<PathBuf, String> {
    if value.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:")) {
        let url = url::Url::parse(value).map_err(|error| format!("invalid artifact file URI: {error}"))?;
        return url
            .to_file_path()
            .map_err(|_| "artifact file URI is not a local filesystem path".to_owned());
    }
    Ok(PathBuf::from(value))
}

fn intended_artifact_path(workspace: &Path, value: &str) -> Result<PathBuf, String> {
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|error| format!("cannot canonicalize artifact workspace: {error}"))?;
    let requested = artifact_candidate_path(value)?;
    if requested
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("artifact path contains a parent-directory traversal".to_owned());
    }
    let candidate = if requested.is_absolute() {
        requested
    } else {
        workspace.join(requested)
    };

    // Canonicalize the nearest existing ancestor, then re-attach the missing
    // suffix. This validates both paths that already exist and paths the tool
    // promises to create, including symlinked parents, without a time-of-check
    // assumption about filesystem timestamp precision.
    let mut ancestor = candidate.as_path();
    let mut missing_suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let file_name = ancestor
                    .file_name()
                    .ok_or_else(|| "artifact path has no existing ancestor".to_owned())?;
                missing_suffix.push(file_name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| "artifact path has no existing ancestor".to_owned())?;
            }
            Err(error) => return Err(format!("cannot inspect artifact path: {error}")),
        }
    }
    let mut resolved = std::fs::canonicalize(ancestor)
        .map_err(|error| format!("cannot canonicalize artifact path: {error}"))?;
    if !resolved.starts_with(&workspace) {
        return Err("artifact path escapes the workspace boundary".to_owned());
    }
    for component in missing_suffix.into_iter().rev() {
        resolved.push(component);
    }
    if !resolved.starts_with(&workspace) {
        return Err("artifact path escapes the workspace boundary".to_owned());
    }
    Ok(resolved)
}

fn reserve_artifact_baseline_hash_bytes(
    already_reserved: u64,
    file_size: u64,
) -> Result<u64, String> {
    if file_size > MAX_BASELINE_ARTIFACT_FILE_BYTES {
        return Err(format!(
            "artifact baseline is {file_size} bytes, over the {} byte per-file hash limit",
            MAX_BASELINE_ARTIFACT_FILE_BYTES
        ));
    }
    let reserved = already_reserved
        .checked_add(file_size)
        .ok_or_else(|| "artifact baseline hash budget overflowed".to_owned())?;
    if reserved > MAX_BASELINE_ARTIFACT_BATCH_BYTES {
        return Err(format!(
            "artifact baseline batch would exceed the {} byte aggregate hash limit",
            MAX_BASELINE_ARTIFACT_BATCH_BYTES
        ));
    }
    Ok(reserved)
}

fn ensure_artifact_baseline_path_identity(path: &Path, file: &File) -> Result<(), String> {
    let open_identity = SameFileHandle::from_file(
        file.try_clone()
            .map_err(|error| format!("cannot clone artifact baseline handle: {error}"))?,
    )
    .map_err(|error| format!("cannot identify artifact baseline handle: {error}"))?;
    let path_identity = SameFileHandle::from_path(path)
        .map_err(|error| format!("cannot identify artifact baseline path: {error}"))?;
    if open_identity != path_identity {
        return Err("artifact baseline path was replaced while it was being fingerprinted".to_owned());
    }
    Ok(())
}

fn hash_file_for_baseline(path: &Path, expected_size: u64) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot open artifact baseline: {error}"))?;
    let handle_before = file
        .metadata()
        .map_err(|error| format!("cannot inspect artifact baseline handle: {error}"))?;
    let path_before = std::fs::metadata(path)
        .map_err(|error| format!("cannot inspect artifact baseline path: {error}"))?;
    let modified_before = handle_before
        .modified()
        .map_err(|error| format!("cannot timestamp artifact baseline handle: {error}"))?;
    if !handle_before.is_file()
        || !path_before.is_file()
        || handle_before.len() != expected_size
        || path_before.len() != expected_size
        || path_before
            .modified()
            .map_err(|error| format!("cannot timestamp artifact baseline path: {error}"))?
            != modified_before
    {
        return Err("artifact baseline changed before it could be fingerprinted".to_owned());
    }
    // Bind the digest to the file object currently named by `path`, both before
    // and after the read. Size checks alone let a same-size rename replacement
    // pair an old open handle's digest with a different current path.
    ensure_artifact_baseline_path_identity(path, &file)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes_read = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read artifact baseline: {error}"))?;
        if read == 0 {
            break;
        }
        bytes_read = bytes_read
            .checked_add(read as u64)
            .ok_or_else(|| "artifact baseline byte count overflowed".to_owned())?;
        if bytes_read > expected_size {
            return Err("artifact baseline changed while it was being fingerprinted".to_owned());
        }
        digest.update(&buffer[..read]);
    }
    let handle_metadata = file
        .metadata()
        .map_err(|error| format!("cannot re-check artifact baseline handle: {error}"))?;
    let path_metadata = std::fs::metadata(path)
        .map_err(|error| format!("cannot re-check artifact baseline: {error}"))?;
    if !handle_metadata.is_file()
        || handle_metadata.len() != expected_size
        || !path_metadata.is_file()
        || path_metadata.len() != expected_size
        || bytes_read != expected_size
        || handle_metadata
            .modified()
            .map_err(|error| format!("cannot re-check artifact baseline timestamp: {error}"))?
            != modified_before
        || path_metadata
            .modified()
            .map_err(|error| format!("cannot re-check artifact baseline path timestamp: {error}"))?
            != modified_before
    {
        return Err("artifact baseline changed while it was being fingerprinted".to_owned());
    }
    ensure_artifact_baseline_path_identity(path, &file)?;
    Ok(hex::encode(digest.finalize()))
}

fn capture_path_fingerprint(
    path: &Path,
    batch_hash_bytes: &mut u64,
) -> Result<ArtifactPathFingerprint, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ArtifactPathFingerprint::Absent);
        }
        Err(error) => return Err(format!("cannot inspect artifact baseline: {error}")),
    };
    if !metadata.is_file() {
        return Err("declared artifact baseline is not a regular file".to_owned());
    }
    let size_bytes = metadata.len();
    // Reserve before reading so repeated files that race or fail their final
    // stability check cannot each consume the full synchronous budget.
    *batch_hash_bytes = reserve_artifact_baseline_hash_bytes(*batch_hash_bytes, size_bytes)?;
    let sha256 = hash_file_for_baseline(path, size_bytes)?;
    Ok(ArtifactPathFingerprint::Present { size_bytes, sha256 })
}

/// Parse the `update_plan` tool result content into frontend plan entries.
/// The content may carry a soft-warning prefix, so we start from the first '{'.
fn parse_plan_entries(content: &str) -> Option<Vec<serde_json::Value>> {
    let start = content.find('{')?;
    let mut v: serde_json::Value = serde_json::from_str(&content[start..]).ok()?;
    if v.get("kind").and_then(|k| k.as_str()) != Some("plan_update") {
        return None;
    }
    match v.get_mut("entries").map(serde_json::Value::take) {
        Some(serde_json::Value::Array(entries)) => Some(entries),
        _ => None,
    }
}

impl BackendOutputSink {
    pub fn new(event_tx: broadcast::Sender<AgentStreamEvent>) -> Self {
        Self {
            event_tx,
            distill_dir: None,
            artifact_store: None,
            artifact_workspace: None,
            turn_text: Mutex::new(String::new()),
            accepted_turn_output_checkpoint: Mutex::new(None),
            attempt_output_checkpoint: Mutex::new(None),
            active_tool_calls: Mutex::new(HashMap::new()),
            tool_result_contexts: Mutex::new(HashMap::new()),
            artifact_delivery_turn: Mutex::new(ArtifactDeliveryTurn::default()),
        }
    }

    /// Set the file-based memory directory used for citation reflow. `None`
    /// (the default) disables reflow for this session.
    pub fn with_distill_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.distill_dir = dir;
        self
    }

    /// Enable durable, verified binary output delivery under the session's
    /// trusted workspace.
    pub fn with_artifact_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        self.artifact_store = Some(ArtifactStore::new(workspace.clone()));
        self.artifact_workspace = Some(workspace);
        self
    }

    /// Begin one accepted user turn's artifact-delivery ledger. Engine
    /// sub-streams do not call this: steering and truncation continuations are
    /// part of the same accepted turn and must retain earlier failures.
    #[cfg(test)]
    pub fn begin_artifact_delivery_turn(&self) {
        self.begin_artifact_delivery_turn_with_mode(false, None);
    }

    /// Production Nomi entry point. Artifact payloads and successful terminal
    /// cards remain provisional until the manager can run durable I/O on the
    /// blocking pool and atomically commit the accepted turn.
    #[cfg(test)]
    pub(crate) fn begin_deferred_artifact_delivery_turn(&self) {
        self.begin_artifact_delivery_turn_with_mode(true, None);
    }

    /// Production owner-aware entry point. The complete terminal tool event is
    /// journaled against this exact relay wire before broadcast publication.
    pub(crate) fn begin_deferred_artifact_delivery_turn_for(
        &self,
        conversation_id: &str,
        wire_msg_id: &str,
    ) -> Result<(), String> {
        if conversation_id.trim().is_empty() || wire_msg_id.trim().is_empty() {
            return Err("artifact recovery owner is incomplete".to_owned());
        }
        self.begin_artifact_delivery_turn_with_mode(
            true,
            Some(ArtifactRecoverySource {
                conversation_id: conversation_id.to_owned(),
                wire_msg_id: wire_msg_id.to_owned(),
            }),
        );
        Ok(())
    }

    fn begin_artifact_delivery_turn_with_mode(
        &self,
        defer_artifact_terminals: bool,
        recovery_source: Option<ArtifactRecoverySource>,
    ) {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stale_calls = std::mem::take(&mut turn.calls);
        turn.advance_generation();
        turn.defer_artifact_terminals = defer_artifact_terminals;
        turn.required_contract = None;
        turn.hold_text_until_verified = false;
        turn.held_text.clear();
        turn.recovery_source = recovery_source;
        turn.active = true;
        drop(turn);
        self.turn_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        *self
            .accepted_turn_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        *self
            .attempt_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.retire_artifact_calls(
            stale_calls,
            "artifact delivery was superseded by a new accepted turn",
        );
    }

    /// Require this accepted turn to end with at least one durable, verified
    /// image receipt. The requirement is intentionally turn-scoped rather than
    /// tied to a guessed call id: the provider has not emitted the native
    /// `image_gen` call yet. `finish_artifact_delivery_turn` re-verifies the
    /// matching receipt immediately before the manager may publish Finish.
    pub fn require_image_artifact_for_turn(&self) -> Result<(), String> {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.active {
            return Err("artifact-delivery turn was not active".to_owned());
        }
        turn.required_contract = Some(ArtifactContract {
            expectation: ArtifactExpectation::Image,
            requirement: ArtifactRequirement::Image,
            requested_count: None,
        });
        turn.hold_text_until_verified = true;
        turn.held_text.clear();
        turn.advance_generation();
        Ok(())
    }

    /// Abandon the provisional ledger and any assistant prose held behind its
    /// receipt gate. Used for cancellation/provider failure paths that cannot
    /// reach the normal sealing step.
    pub(crate) fn abort_artifact_delivery_turn(&self) {
        self.abort_artifact_delivery_turn_with_reason(
            "artifact delivery ended before durable turn commit",
        );
    }

    fn abort_artifact_delivery_turn_with_reason(&self, reason: &str) {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let calls = std::mem::take(&mut turn.calls);
        turn.active = false;
        turn.advance_generation();
        turn.defer_artifact_terminals = false;
        turn.required_contract = None;
        turn.hold_text_until_verified = false;
        turn.held_text.clear();
        turn.recovery_source = None;
        drop(turn);
        self.retire_artifact_calls(calls, reason);
    }

    fn abort_artifact_delivery_turn_if_generation(&self, generation: u64, reason: &str) {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.active || turn.generation != generation {
            return;
        }
        let calls = std::mem::take(&mut turn.calls);
        turn.active = false;
        turn.advance_generation();
        turn.defer_artifact_terminals = false;
        turn.required_contract = None;
        turn.hold_text_until_verified = false;
        turn.held_text.clear();
        turn.recovery_source = None;
        drop(turn);
        self.retire_artifact_calls(calls, reason);
    }

    fn retire_artifact_calls(
        &self,
        calls: HashMap<String, ArtifactCallObligation>,
        reason: &str,
    ) {
        if calls.is_empty() {
            return;
        }
        let mut receipts = Vec::new();
        let mut pending_payloads = Vec::new();
        let mut deferred_terminals = Vec::new();
        for obligation in calls.into_values() {
            match obligation.status {
                ArtifactCallDeliveryStatus::Pending(pending) => {
                    deferred_terminals.push(pending.terminal.clone());
                    pending_payloads.push(pending);
                }
                ArtifactCallDeliveryStatus::Persisting(terminal) => {
                    deferred_terminals.push(terminal);
                }
                ArtifactCallDeliveryStatus::CompletedVerified {
                    artifacts,
                    deferred_terminal,
                } => {
                    receipts.extend(artifacts);
                    if let Some(terminal) = deferred_terminal {
                        deferred_terminals.push(terminal);
                    }
                }
                ArtifactCallDeliveryStatus::Running | ArtifactCallDeliveryStatus::Failed(_) => {}
            }
        }
        for terminal in deferred_terminals {
            let _ = self.emit_deferred_tool_result_event(
                terminal,
                true,
                reason.to_owned(),
                Vec::new(),
                None,
            );
        }
        self.schedule_artifact_cleanup(receipts, pending_payloads);
    }

    fn schedule_artifact_cleanup(
        &self,
        receipts: Vec<PersistedArtifact>,
        pending_payloads: Vec<PendingArtifactDelivery>,
    ) {
        if receipts.is_empty() && pending_payloads.is_empty() {
            return;
        }
        let store = self.artifact_store.clone();
        let cleanup = move || {
            if !receipts.is_empty() {
                match store.as_ref() {
                    Some(store) => {
                        if let Err(error) = store.rollback_owned_receipts(&receipts) {
                            tracing::error!(
                                artifact_count = receipts.len(),
                                error = %error,
                                "failed to roll back provisional turn artifacts"
                            );
                        }
                    }
                    None => tracing::error!(
                        artifact_count = receipts.len(),
                        "cannot roll back provisional artifacts because the store is unavailable"
                    ),
                }
            }
            drop(pending_payloads);
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let _cleanup = handle.spawn_blocking(cleanup);
            }
            Err(_) => cleanup(),
        }
    }

    /// Clone only receipt metadata and contracts under the ledger mutex. Every
    /// filesystem read and media decode happens after this snapshot is released.
    fn artifact_verification_snapshot(&self) -> Result<ArtifactVerificationSnapshot, String> {
        let turn = self
            .artifact_delivery_turn
            .lock()
            .map_err(|_| "artifact-delivery ledger lock was poisoned".to_owned())?;
        if !turn.active {
            return Err("artifact-delivery turn was not active".to_owned());
        }

        let generation = turn.generation;
        let required_contract = turn.required_contract;
        let mut required_contract_satisfied = false;
        let mut failures = Vec::new();
        let mut receipts = Vec::new();
        for obligation in turn.calls.values() {
            match &obligation.status {
                ArtifactCallDeliveryStatus::CompletedVerified { artifacts, .. } => {
                    let mime_types = artifacts
                        .iter()
                        .map(|artifact| artifact.mime_type.as_str())
                        .collect::<Vec<_>>();
                    if let Err(error) = obligation.contract.validate_mimes(&mime_types) {
                        failures.push(format!(
                            "{} ({}) failed final contract verification: {error}",
                            obligation.tool_name,
                            obligation.contract.label()
                        ));
                    }
                    if required_contract
                        .is_some_and(|required| required.validate_mimes(&mime_types).is_ok())
                    {
                        required_contract_satisfied = true;
                    }
                    receipts.extend(artifacts.iter().cloned().map(|artifact| {
                        ArtifactReceiptVerification {
                            tool_name: obligation.tool_name.clone(),
                            contract_label: obligation.contract.label(),
                            artifact,
                        }
                    }));
                }
                ArtifactCallDeliveryStatus::Running => failures.push(format!(
                    "{} ({}) ended without a verified artifact receipt",
                    obligation.tool_name,
                    obligation.contract.label()
                )),
                ArtifactCallDeliveryStatus::Pending(_)
                | ArtifactCallDeliveryStatus::Persisting(_) => failures.push(format!(
                    "{} ({}) still has pending durable artifact work",
                    obligation.tool_name,
                    obligation.contract.label()
                )),
                ArtifactCallDeliveryStatus::Failed(reason) => failures.push(format!(
                    "{} ({}) failed artifact delivery: {reason}",
                    obligation.tool_name,
                    obligation.contract.label()
                )),
            }
        }
        if let Some(required) = required_contract
            && !required_contract_satisfied
        {
            failures.push(format!(
                "accepted turn required a verified {}, but no matching receipt was committed",
                required.label()
            ));
        }
        drop(turn);

        if failures.is_empty() {
            Ok(ArtifactVerificationSnapshot {
                generation,
                receipts,
            })
        } else {
            let error = failures.join("; ");
            self.abort_artifact_delivery_turn_if_generation(generation, &error);
            Err(error)
        }
    }

    fn reverify_snapshot_blocking(
        store: Option<ArtifactStore>,
        snapshot: &ArtifactVerificationSnapshot,
    ) -> Result<(), String> {
        if snapshot.receipts.is_empty() {
            return Ok(());
        }
        let store = store.ok_or_else(|| {
            "artifact delivery lost its workspace store before final verification".to_owned()
        })?;
        let mut failures = Vec::new();
        for receipt in &snapshot.receipts {
            if let Err(error) = store.reverify_receipt(&receipt.artifact) {
                failures.push(format!(
                    "{} ({}) artifact {} failed final verification: {error}",
                    receipt.tool_name, receipt.contract_label, receipt.artifact.path
                ));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    #[cfg(test)]
    fn verify_artifact_delivery_turn_sync_snapshot(
        &self,
    ) -> Result<VerifiedArtifactDeliveryTurn, String> {
        let snapshot = self.artifact_verification_snapshot()?;
        if let Err(error) =
            Self::reverify_snapshot_blocking(self.artifact_store.clone(), &snapshot)
        {
            self.abort_artifact_delivery_turn_if_generation(snapshot.generation, &error);
            return Err(error);
        }
        Ok(VerifiedArtifactDeliveryTurn {
            generation: snapshot.generation,
        })
    }

    fn take_pending_artifact_work(
        &self,
    ) -> Result<
        Option<(
            u64,
            ArtifactStore,
            Vec<PendingArtifactWork>,
            Option<ArtifactRecoverySource>,
        )>,
        String,
    > {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .map_err(|_| "artifact-delivery ledger lock was poisoned".to_owned())?;
        if !turn.active {
            return Err("artifact-delivery turn was not active".to_owned());
        }
        if turn.calls.values().any(|obligation| {
            matches!(obligation.status, ArtifactCallDeliveryStatus::Persisting(_))
        }) {
            return Err("artifact delivery already has blocking work in progress".to_owned());
        }

        let pending_ids = turn
            .calls
            .iter()
            .filter_map(|(call_id, obligation)| {
                matches!(obligation.status, ArtifactCallDeliveryStatus::Pending(_))
                    .then_some(call_id.clone())
            })
            .collect::<Vec<_>>();
        if pending_ids.is_empty() {
            return Ok(None);
        }
        let store = self
            .artifact_store
            .clone()
            .ok_or_else(|| "session has no workspace artifact store".to_owned())?;

        let mut work = Vec::with_capacity(pending_ids.len());
        for call_id in pending_ids {
            let obligation = turn.calls.get_mut(&call_id).ok_or_else(|| {
                "pending artifact obligation disappeared before persistence".to_owned()
            })?;
            let status = std::mem::replace(
                &mut obligation.status,
                ArtifactCallDeliveryStatus::Failed(
                    "pending artifact state could not be transferred".to_owned(),
                ),
            );
            let ArtifactCallDeliveryStatus::Pending(pending) = status else {
                return Err("pending artifact obligation changed before persistence".to_owned());
            };
            obligation.status =
                ArtifactCallDeliveryStatus::Persisting(pending.terminal.clone());
            work.push(PendingArtifactWork {
                call_id,
                contract: obligation.contract,
                pending,
            });
        }
        turn.advance_generation();
        Ok(Some((
            turn.generation,
            store,
            work,
            turn.recovery_source.clone(),
        )))
    }

    fn persist_pending_work_blocking(
        store: &ArtifactStore,
        work: Vec<PendingArtifactWork>,
        recovery_source: Option<ArtifactRecoverySource>,
    ) -> Result<Vec<PersistedPendingArtifact>, String> {
        let mut persisted = Vec::with_capacity(work.len());
        for work_item in work {
            let inline = work_item.pending.inline.iter().map(|artifact| {
                (
                    artifact.kind,
                    artifact.mime_type.as_str(),
                    artifact.data.as_str(),
                )
            });
            let artifacts = match if let Some(source) = recovery_source.as_ref() {
                store.persist_inline_and_verified_existing_batch_recoverable(
                    inline,
                    &work_item.pending.existing_sources,
                    source,
                )
            } else {
                store.persist_inline_and_verified_existing_batch(
                    inline,
                    &work_item.pending.existing_sources,
                )
            } {
                Ok(artifacts) => artifacts,
                Err(error) => {
                    let receipts = persisted
                        .iter()
                        .flat_map(|item: &PersistedPendingArtifact| item.artifacts.iter().cloned())
                        .collect::<Vec<_>>();
                    let _ = store.rollback_owned_receipts(&receipts);
                    return Err(error.to_string());
                }
            };
            let mime_types = artifacts
                .iter()
                .map(|artifact| artifact.mime_type.as_str())
                .collect::<Vec<_>>();
            if let Err(error) = work_item.contract.validate_mimes(&mime_types) {
                let mut receipts = persisted
                    .iter()
                    .flat_map(|item: &PersistedPendingArtifact| item.artifacts.iter().cloned())
                    .collect::<Vec<_>>();
                receipts.extend(artifacts);
                let _ = store.rollback_owned_receipts(&receipts);
                return Err(format!(
                    "verified receipts do not satisfy the artifact contract: {error}"
                ));
            }
            persisted.push(PersistedPendingArtifact {
                call_id: work_item.call_id,
                artifacts,
            });
        }
        Ok(persisted)
    }

    fn install_persisted_pending(
        &self,
        generation: u64,
        persisted: Vec<PersistedPendingArtifact>,
    ) -> Result<(), (String, Vec<PersistedArtifact>)> {
        let cleanup = persisted
            .iter()
            .flat_map(|item| item.artifacts.iter().cloned())
            .collect::<Vec<_>>();
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .map_err(|_| {
                (
                    "artifact-delivery ledger lock was poisoned".to_owned(),
                    cleanup.clone(),
                )
            })?;
        if !turn.active || turn.generation != generation {
            return Err((
                "artifact-delivery generation changed before persistence completed".to_owned(),
                cleanup,
            ));
        }
        for item in &persisted {
            if !turn.calls.get(&item.call_id).is_some_and(|obligation| {
                matches!(obligation.status, ArtifactCallDeliveryStatus::Persisting(_))
            }) {
                return Err((
                    "artifact obligation changed before persistence completed".to_owned(),
                    cleanup,
                ));
            }
        }
        for item in persisted {
            let obligation = turn
                .calls
                .get_mut(&item.call_id)
                .expect("validated pending artifact obligation must still exist");
            let terminal = match std::mem::replace(
                &mut obligation.status,
                ArtifactCallDeliveryStatus::Failed(
                    "persisted artifact could not be installed".to_owned(),
                ),
            ) {
                ArtifactCallDeliveryStatus::Persisting(terminal) => terminal,
                _ => unreachable!("pending artifact status was validated above"),
            };
            obligation.status = ArtifactCallDeliveryStatus::CompletedVerified {
                artifacts: item.artifacts,
                deferred_terminal: Some(terminal),
            };
        }
        turn.advance_generation();
        Ok(())
    }

    fn detach_persistence_cleanup(
        task: tokio::task::JoinHandle<Result<Vec<PersistedPendingArtifact>, String>>,
        store: ArtifactStore,
    ) {
        tokio::spawn(async move {
            if let Ok(Ok(persisted)) = task.await {
                let receipts = persisted
                    .into_iter()
                    .flat_map(|item| item.artifacts)
                    .collect::<Vec<_>>();
                let _cleanup = tokio::task::spawn_blocking(move || {
                    let _ = store.rollback_owned_receipts(&receipts);
                })
                .await;
            }
        });
    }

    async fn persist_pending_artifacts_async(
        &self,
        cancellation: &CancellationToken,
    ) -> AsyncArtifactDeliveryOutcome {
        if cancellation.is_cancelled() {
            return AsyncArtifactDeliveryOutcome::Cancelled;
        }
        let Some((generation, store, work, recovery_source)) = (match self.take_pending_artifact_work() {
            Ok(work) => work,
            Err(error) => return AsyncArtifactDeliveryOutcome::Failed(error),
        }) else {
            return AsyncArtifactDeliveryOutcome::Verified(VerifiedArtifactDeliveryTurn {
                generation: self
                    .artifact_delivery_turn
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .generation,
            });
        };

        let job_store = store.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            Self::persist_pending_work_blocking(&job_store, work, recovery_source)
        });
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                Self::detach_persistence_cleanup(task, store);
                return AsyncArtifactDeliveryOutcome::Cancelled;
            }
            result = &mut task => result,
        };

        let persisted = match result {
            Ok(Ok(persisted)) => persisted,
            Ok(Err(error)) => return AsyncArtifactDeliveryOutcome::Failed(error),
            Err(error) => {
                let error = format!("artifact persistence worker failed: {error}");
                return AsyncArtifactDeliveryOutcome::Failed(error);
            }
        };
        if cancellation.is_cancelled() {
            let receipts = persisted
                .into_iter()
                .flat_map(|item| item.artifacts)
                .collect::<Vec<_>>();
            self.schedule_artifact_cleanup(receipts, Vec::new());
            return AsyncArtifactDeliveryOutcome::Cancelled;
        }
        if let Err((error, receipts)) = self.install_persisted_pending(generation, persisted) {
            self.schedule_artifact_cleanup(receipts, Vec::new());
            return if cancellation.is_cancelled() {
                AsyncArtifactDeliveryOutcome::Cancelled
            } else {
                AsyncArtifactDeliveryOutcome::Failed(error)
            };
        }
        AsyncArtifactDeliveryOutcome::Verified(VerifiedArtifactDeliveryTurn { generation })
    }

    /// Flush deferred artifact payloads and re-verify every receipt on the blocking
    /// pool. Cancellation returns immediately; any detached persistence job is
    /// responsible for deleting receipts it may finish creating afterwards.
    ///
    /// A non-verified outcome intentionally leaves the output checkpoint and
    /// artifact terminal ledger intact. The manager must first restore the
    /// accepted-turn root (which emits `OutputDiscarded`) and only then abort
    /// artifact delivery. Clearing held prose here would make a non-zero output
    /// checkpoint invalid before the manager has a chance to retract it.
    pub(crate) async fn verify_artifact_delivery_turn_async(
        &self,
        cancellation: &CancellationToken,
    ) -> AsyncArtifactDeliveryOutcome {
        match self.persist_pending_artifacts_async(cancellation).await {
            AsyncArtifactDeliveryOutcome::Verified(_) => {}
            outcome => return outcome,
        }
        if cancellation.is_cancelled() {
            return AsyncArtifactDeliveryOutcome::Cancelled;
        }
        let snapshot = match self.artifact_verification_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => return AsyncArtifactDeliveryOutcome::Failed(error),
        };
        if snapshot.receipts.is_empty() {
            return AsyncArtifactDeliveryOutcome::Verified(VerifiedArtifactDeliveryTurn {
                generation: snapshot.generation,
            });
        }

        let store = self.artifact_store.clone();
        let job_snapshot = snapshot.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            Self::reverify_snapshot_blocking(store, &job_snapshot)
        });
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                let retry_store = self.artifact_store.clone();
                let retry_receipts = snapshot
                    .receipts
                    .iter()
                    .map(|receipt| receipt.artifact.clone())
                    .collect::<Vec<_>>();
                tokio::spawn(async move {
                    let _ = task.await;
                    if let Some(store) = retry_store {
                        let _cleanup = tokio::task::spawn_blocking(move || {
                            let _ = store.rollback_owned_receipts(&retry_receipts);
                        }).await;
                    }
                });
                return AsyncArtifactDeliveryOutcome::Cancelled;
            }
            result = &mut task => result,
        };
        match result {
            Ok(Ok(())) if !cancellation.is_cancelled() => {
                AsyncArtifactDeliveryOutcome::Verified(VerifiedArtifactDeliveryTurn {
                    generation: snapshot.generation,
                })
            }
            Ok(Ok(())) => AsyncArtifactDeliveryOutcome::Cancelled,
            Ok(Err(error)) => AsyncArtifactDeliveryOutcome::Failed(error),
            Err(error) => {
                let error = format!("artifact verification worker failed: {error}");
                AsyncArtifactDeliveryOutcome::Failed(error)
            }
        }
    }

    /// Generation-CAS commit only: no filesystem access or media decode occurs
    /// here, so it is safe to call while the manager holds its lifecycle gate.
    pub(crate) fn finish_verified_artifact_delivery_turn(
        &self,
        verified: VerifiedArtifactDeliveryTurn,
    ) -> Result<(), String> {
        self.finish_verified_artifact_delivery_turn_with(verified, |_| {})
    }

    fn finish_verified_artifact_delivery_turn_with<F>(
        &self,
        verified: VerifiedArtifactDeliveryTurn,
        mut after_successful_send: F,
    ) -> Result<(), String>
    where
        F: FnMut(usize),
    {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .map_err(|_| "artifact-delivery ledger lock was poisoned".to_owned())?;
        if !turn.active || turn.generation != verified.generation {
            return Err("artifact-delivery generation changed after final verification".to_owned());
        }
        let image_delivery = turn
            .required_contract
            .is_some_and(|contract| contract.expectation == ArtifactExpectation::Image)
            || turn.calls.values().any(|obligation| {
                obligation.contract.expectation == ArtifactExpectation::Image
            });
        let calls = std::mem::take(&mut turn.calls);
        turn.active = false;
        turn.advance_generation();
        let sealed_generation = turn.generation;
        turn.defer_artifact_terminals = false;
        turn.required_contract = None;
        // Keep the sealed prose in the ledger until every deferred artifact
        // terminal is published. A failed prepare/send returns to the manager's
        // accepted-root restore, whose OutputDiscarded must still be able to
        // truncate this exact generation at its non-zero checkpoint.
        let held_text = turn.held_text.clone();
        turn.hold_text_until_verified = false;
        let recovery_source = turn.recovery_source.take();
        drop(turn);

        let mut deferred_events = Vec::new();
        for obligation in calls.into_values() {
            if let ArtifactCallDeliveryStatus::CompletedVerified {
                artifacts,
                deferred_terminal: Some(terminal),
            } = obligation.status
            {
                let context = Self::delivery_context(&artifacts);
                let output = Self::append_delivery_context(&terminal.content, &context);
                deferred_events.push(Self::deferred_tool_result_event_data(
                    terminal,
                    false,
                    output,
                    artifacts,
                ));
            }
        }
        // Prepare the whole drained ledger before publishing its first event.
        // If any journal write fails, every still-unpublished snapshot remains
        // known-not-committed and can be rolled back as one batch.
        if let Some(source) = recovery_source.as_ref() {
            for data in &deferred_events {
                if let Err(error) = self.prepare_deferred_tool_result_recovery(data, source) {
                    self.rollback_deferred_event_artifacts(&deferred_events);
                    return Err(error);
                }
            }
        }
        for (index, data) in deferred_events.iter().cloned().enumerate() {
            if self.event_tx.send(AgentStreamEvent::ToolCall(data)).is_err() {
                // Earlier successful sends keep their journals for the relay
                // (or recovery scanner). This event and every later one were
                // never exposed and are therefore safe to delete immediately.
                // The manager truthfully fails the overall turn; an earlier
                // journal-owned card is a durable partial delivery, not a
                // success claim for the uncommitted remainder.
                self.rollback_deferred_event_artifacts(&deferred_events[index..]);
                return Err("artifact terminal event had no live relay receiver".to_owned());
            }
            after_successful_send(index);
        }
        {
            let mut turn = self
                .artifact_delivery_turn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !turn.active && turn.generation == sealed_generation {
                turn.held_text.clear();
            }
        }
        // Conversation projection is the final ownership transfer for native
        // images. The verified result card above is the authoritative success
        // surface, so model prose and Thinking remain suppressed for image turns.
        if !image_delivery && !held_text.is_empty() {
            if self.distill_dir.is_some()
                && let Ok(mut buf) = self.turn_text.lock()
            {
                buf.push_str(&held_text);
            }
            let _ = self.event_tx.send(AgentStreamEvent::Text(TextEventData {
                content: held_text,
            }));
        }
        Ok(())
    }

    /// Synchronous compatibility path used outside the production Nomi turn.
    #[cfg(test)]
    pub fn finish_artifact_delivery_turn(&self) -> Result<(), String> {
        let verified = self.verify_artifact_delivery_turn_sync_snapshot()?;
        self.finish_verified_artifact_delivery_turn(verified)
    }

    fn record_artifact_obligation(
        &self,
        call_id: &str,
        tool_name: &str,
        contract: Option<ArtifactContract>,
    ) {
        let Some(contract) = contract else {
            return;
        };
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.active {
            return;
        }
        if contract.expectation == ArtifactExpectation::Image {
            turn.hold_text_until_verified = true;
        }
        turn.calls
            .entry(call_id.to_owned())
            .and_modify(|obligation| {
                if matches!(obligation.status, ArtifactCallDeliveryStatus::Running) {
                    match obligation.contract.merge(contract) {
                        Ok(merged) => obligation.contract = merged,
                        Err(error) => {
                            obligation.status = ArtifactCallDeliveryStatus::Failed(format!(
                                "conflicting artifact contract metadata: {error}"
                            ));
                        }
                    }
                }
            })
            .or_insert_with(|| ArtifactCallObligation {
                tool_name: tool_name.to_owned(),
                contract,
                status: ArtifactCallDeliveryStatus::Running,
            });
        turn.advance_generation();
    }

    fn register_artifact_obligation(
        &self,
        call_id: &str,
        tool_name: &str,
        contract: Option<ArtifactContract>,
    ) {
        let Some(contract) = contract else {
            return;
        };
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.active {
            return;
        }
        if contract.expectation == ArtifactExpectation::Image {
            turn.hold_text_until_verified = true;
        }
        match turn.calls.entry(call_id.to_owned()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(ArtifactCallObligation {
                    tool_name: tool_name.to_owned(),
                    contract,
                    status: ArtifactCallDeliveryStatus::Running,
                });
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().status = ArtifactCallDeliveryStatus::Failed(
                    "artifact-producing tool call reused a prior call id".to_owned(),
                );
            }
        }
        turn.advance_generation();
    }

    fn settle_artifact_obligation(
        &self,
        call_id: &str,
        tool_name: &str,
        is_error: bool,
        artifacts: &[PersistedArtifact],
    ) {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.calls.contains_key(call_id) && !is_error && !artifacts.is_empty() && turn.active {
            // An unknown/brand-named tool may still return a real persisted
            // artifact even when its name carried no pre-call requirement.
            // Every receipt that we publish must enter the turn ledger so a
            // later tool cannot delete it before Finish and escape final
            // re-verification.
            turn.calls.insert(
                call_id.to_owned(),
                ArtifactCallObligation {
                    tool_name: tool_name.to_owned(),
                    contract: any_artifact_contract(),
                    status: ArtifactCallDeliveryStatus::Running,
                },
            );
        }
        let Some(obligation) = turn.calls.get_mut(call_id) else {
            return;
        };
        obligation.status = match &obligation.status {
            ArtifactCallDeliveryStatus::Running => {
                if is_error {
                    ArtifactCallDeliveryStatus::Failed("tool returned an error".to_owned())
                } else if artifacts.is_empty() {
                    ArtifactCallDeliveryStatus::Failed(
                        "tool completed without a verified artifact receipt".to_owned(),
                    )
                } else {
                    let mime_types = artifacts
                        .iter()
                        .map(|artifact| artifact.mime_type.as_str())
                        .collect::<Vec<_>>();
                    match obligation.contract.validate_mimes(&mime_types) {
                        Ok(()) => ArtifactCallDeliveryStatus::CompletedVerified {
                            artifacts: artifacts.to_vec(),
                            deferred_terminal: None,
                        },
                        Err(error) => ArtifactCallDeliveryStatus::Failed(format!(
                            "verified receipts do not satisfy the artifact contract: {error}"
                        )),
                    }
                }
            }
            ArtifactCallDeliveryStatus::Pending(_)
            | ArtifactCallDeliveryStatus::Persisting(_)
            | ArtifactCallDeliveryStatus::CompletedVerified { .. } => {
                ArtifactCallDeliveryStatus::Failed(
                    "artifact-producing tool call emitted more than one terminal result".to_owned(),
                )
            }
            ArtifactCallDeliveryStatus::Failed(reason) => {
                ArtifactCallDeliveryStatus::Failed(reason.clone())
            }
        };
        turn.advance_generation();
    }

    fn fail_artifact_obligation(&self, call_id: &str, reason: &str) {
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(obligation) = turn.calls.get_mut(call_id) {
            obligation.status = ArtifactCallDeliveryStatus::Failed(reason.to_owned());
            turn.advance_generation();
        }
    }

    fn record_unidentified_artifact_failure(
        &self,
        tool_name: &str,
        contract: Option<ArtifactContract>,
        reason: &str,
    ) {
        let Some(contract) = contract else {
            return;
        };
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !turn.active {
            return;
        }
        let mut sequence = turn.calls.len();
        let call_id = loop {
            let candidate = format!("invalid-artifact-call-{sequence}");
            if !turn.calls.contains_key(&candidate) {
                break candidate;
            }
            sequence += 1;
        };
        turn.calls.insert(
            call_id,
            ArtifactCallObligation {
                tool_name: tool_name.to_owned(),
                contract,
                status: ArtifactCallDeliveryStatus::Failed(reason.to_owned()),
            },
        );
        turn.advance_generation();
    }

    fn capture_artifact_path_baselines(
        &self,
        artifact_identity: &str,
        input: &serde_json::Value,
    ) -> ArtifactPathBaselines {
        let declared = input_artifact_paths(input, artifact_contract(artifact_identity).is_some());
        let mut baselines = ArtifactPathBaselines::default();
        if !declared.saw_explicit_key && declared.errors.is_empty() {
            return baselines;
        }
        baselines.errors.extend(declared.errors);
        if declared.paths.is_empty() {
            if baselines.errors.is_empty() {
                baselines
                    .errors
                    .push("declared artifact output key contains no usable path".to_owned());
            }
            return baselines;
        }
        let Some(workspace) = self.artifact_workspace.as_deref() else {
            baselines
                .errors
                .push("session has no workspace for artifact baselines".to_owned());
            return baselines;
        };
        let mut batch_hash_bytes = 0_u64;
        for raw_path in declared.paths {
            let path = match intended_artifact_path(workspace, &raw_path) {
                Ok(path) => path,
                Err(error) => {
                    baselines
                        .errors
                        .push(format!("invalid declared artifact path {raw_path:?}: {error}"));
                    continue;
                }
            };
            if baselines.entries.iter().any(|known| known.path == path) {
                continue;
            }
            match capture_path_fingerprint(&path, &mut batch_hash_bytes) {
                Ok(fingerprint) => baselines
                    .entries
                    .push(ArtifactPathBaseline { path, fingerprint }),
                Err(error) => baselines.errors.push(format!(
                    "cannot fingerprint declared artifact path {raw_path:?}: {error}"
                )),
            }
        }
        baselines
    }

    fn inline_artifact_kind(mime_type: &str) -> ArtifactKind {
        let mime = mime_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if mime.starts_with("image/") {
            ArtifactKind::Image
        } else if mime.starts_with("audio/") {
            ArtifactKind::Audio
        } else if mime.starts_with("video/") {
            ArtifactKind::Video
        } else if mime.starts_with("text/")
            || matches!(mime.as_str(), "application/json" | "application/xml")
        {
            ArtifactKind::Text
        } else {
            ArtifactKind::File
        }
    }

    fn append_delivery_context(content: &str, context: &str) -> String {
        match (content.trim().is_empty(), context.trim().is_empty()) {
            (true, _) => context.to_owned(),
            (_, true) => content.to_owned(),
            (false, false) => format!("{content}\n{context}"),
        }
    }

    fn delivery_context(artifacts: &[PersistedArtifact]) -> String {
        let mut context = String::from("Verified artifacts saved to:");
        for artifact in artifacts {
            context.push_str("\n- ");
            context.push_str(&artifact.path);
        }
        context
    }

    fn should_defer_artifact_terminal(&self) -> bool {
        let turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        turn.active && turn.defer_artifact_terminals
    }

    fn queue_deferred_artifact_delivery(
        &self,
        call_id: String,
        name: &str,
        contract: ArtifactContract,
        content: &str,
        images: &[ToolImage],
        existing_sources: Vec<VerifiedExistingArtifactSource>,
    ) -> Result<String, String> {
        if !images.is_empty() {
            ArtifactStore::preflight_inline_image_batch(images.iter().map(|image| &image.data))
                .map_err(|error| error.to_string())?;
        }
        let inline = images
            .iter()
            .map(|artifact| OwnedInlineArtifact {
                kind: Self::inline_artifact_kind(&artifact.media_type),
                mime_type: artifact.media_type.clone(),
                data: artifact.data.clone(),
            })
            .collect::<Vec<_>>();
        let terminal = self.take_deferred_tool_result(call_id.clone(), name, content);
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .map_err(|_| "artifact-delivery ledger lock was poisoned".to_owned())?;
        if !turn.active || !turn.defer_artifact_terminals {
            return Err("artifact-delivery turn changed before deferred persistence".to_owned());
        }
        let obligation = turn
            .calls
            .get_mut(&call_id)
            .ok_or_else(|| "artifact obligation disappeared before deferred persistence".to_owned())?;
        if !matches!(obligation.status, ArtifactCallDeliveryStatus::Running) {
            return Err("artifact obligation was not running at deferred persistence".to_owned());
        }
        obligation.contract = contract;
        obligation.status = ArtifactCallDeliveryStatus::Pending(PendingArtifactDelivery {
            inline,
            existing_sources,
            terminal,
        });
        turn.advance_generation();
        drop(turn);
        self.forget_active_tool_call(&call_id);
        Ok("Artifact payload received; durable host verification is pending.".to_owned())
    }

    fn preflight_declared_path_artifacts(
        &self,
        active: Option<&ActiveToolCall>,
        contract: ArtifactContract,
        declared_output: &DeclaredArtifactPaths,
    ) -> Result<Vec<VerifiedExistingArtifactSource>, String> {
        if !declared_output.errors.is_empty() {
            return Err(declared_output.errors.join("; "));
        }
        let has_output_declaration =
            declared_output.saw_explicit_key || !declared_output.paths.is_empty();
        let has_input_declaration = active
            .is_some_and(|call| call.artifact_path_baselines.declares_artifact());
        if !has_output_declaration && !has_input_declaration {
            return Ok(Vec::new());
        }
        let active = active.ok_or_else(|| {
            "result-only artifact path has no pre-call baseline; refusing an unproven file"
                .to_owned()
        })?;
        if !active.artifact_path_baselines.errors.is_empty() {
            return Err(active.artifact_path_baselines.errors.join("; "));
        }
        if declared_output.saw_explicit_key && declared_output.paths.is_empty() {
            return Err("artifact result contains an explicit output key but no usable path".to_owned());
        }
        let store = self
            .artifact_store
            .as_ref()
            .ok_or_else(|| "session has no workspace artifact store".to_owned())?;

        let Some(workspace) = self.artifact_workspace.as_deref() else {
            return Err("session has no workspace for artifact verification".to_owned());
        };
        for raw_path in &declared_output.paths {
            let output_path = intended_artifact_path(workspace, raw_path)?;
            if !active
                .artifact_path_baselines
                .entries
                .iter()
                .any(|baseline| baseline.path == output_path)
            {
                return Err(format!(
                    "result-only artifact path {raw_path:?} has no matching pre-call baseline"
                ));
            }
        }

        let mut verified = Vec::with_capacity(active.artifact_path_baselines.entries.len());
        for baseline in &active.artifact_path_baselines.entries {
            let file_type = match std::fs::symlink_metadata(&baseline.path) {
                Ok(metadata) => metadata.file_type(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(
                        "declared artifact path is still missing after the tool completed"
                            .to_owned(),
                    );
                }
                Err(error) => {
                    return Err(format!("cannot inspect declared artifact path: {error}"));
                }
            };
            if file_type.is_symlink() {
                return Err(
                    "declared artifact path became a symbolic link after the call began".to_owned(),
                );
            }
            let artifact = store
                .verify_existing_path(&baseline.path)
                .map_err(|error| format!("declared artifact path failed verification: {error}"))?;
            if !contract.accepts_mime(&artifact.mime_type) {
                return Err(format!(
                    "declared artifact path has MIME {}, expected {}",
                    artifact.mime_type,
                    contract.label()
                ));
            }
            if let ArtifactPathFingerprint::Present { size_bytes, sha256 } = &baseline.fingerprint
                && artifact.sha256 == *sha256
            {
                return Err(format!(
                    "declared artifact path is unchanged from its pre-call fingerprint ({} bytes)",
                    size_bytes
                ));
            }
            if !verified
                .iter()
                .any(|known: &VerifiedExistingArtifactSource| known.path == baseline.path)
            {
                verified.push(VerifiedExistingArtifactSource {
                    path: baseline.path.clone(),
                    sha256: artifact.sha256,
                });
            }
        }
        Ok(verified)
    }

    /// Consume the short-lived per-result context stored by the
    /// `*_with_context` call path. Every terminal path must drain this map so
    /// a later result reusing the id cannot inherit stale args/retry metadata.
    fn take_tool_result_context(&self, call_id: &str) -> Option<ToolTerminalContext> {
        match self.tool_result_contexts.lock() {
            Ok(mut contexts) => contexts.remove(call_id),
            Err(poisoned) => {
                tracing::warn!(
                    error = %poisoned,
                    "Tool-result context lock was poisoned while settling a result"
                );
                poisoned.into_inner().remove(call_id)
            }
        }
    }

    fn take_deferred_tool_result(
        &self,
        call_id: String,
        name: &str,
        content: &str,
    ) -> DeferredToolResult {
        let explicit_context = self.take_tool_result_context(&call_id);
        let active_context = self.active_tool_call(&call_id).map(|active| ToolTerminalContext {
            args: active.args,
            input: active.input,
            retry: active.retry,
        });
        DeferredToolResult {
            call_id,
            name: name.to_owned(),
            content: content.to_owned(),
            context: explicit_context.or(active_context),
        }
    }

    fn emit_deferred_tool_result_event(
        &self,
        terminal: DeferredToolResult,
        is_error: bool,
        output: String,
        artifacts: Vec<PersistedArtifact>,
        recovery_source: Option<&ArtifactRecoverySource>,
    ) -> Result<(), String> {
        let data = Self::deferred_tool_result_event_data(terminal, is_error, output, artifacts);
        if !data.artifacts.is_empty()
            && let Some(source) = recovery_source
        {
            self.prepare_deferred_tool_result_recovery(&data, source)?;
        }
        if self
            .event_tx
            .send(AgentStreamEvent::ToolCall(data.clone()))
            .is_err()
        {
            self.rollback_deferred_event_artifacts(std::slice::from_ref(&data));
            return Err("artifact terminal event had no live relay receiver".to_owned());
        }
        Ok(())
    }

    fn deferred_tool_result_event_data(
        terminal: DeferredToolResult,
        is_error: bool,
        output: String,
        artifacts: Vec<PersistedArtifact>,
    ) -> ToolCallEventData {
        let status = if is_error {
            ToolCallStatus::Error
        } else {
            ToolCallStatus::Completed
        };
        tracing::info!(
            call_id = %terminal.call_id,
            tool = terminal.name,
            status = ?status,
            artifact_count = artifacts.len(),
            "Emitting nomi tool_result event"
        );
        let context = terminal.context;
        ToolCallEventData {
            call_id: terminal.call_id,
            name: terminal.name,
            args: context
                .as_ref()
                .map(|context| context.args.clone())
                .unwrap_or(serde_json::Value::Null),
            status,
            input: context.as_ref().and_then(|context| context.input.clone()),
            output: (!output.is_empty()).then_some(output),
            description: None,
            retry: context.and_then(|context| context.retry),
            artifacts,
        }
    }

    fn prepare_deferred_tool_result_recovery(
        &self,
        data: &ToolCallEventData,
        source: &ArtifactRecoverySource,
    ) -> Result<(), String> {
        if data.artifacts.is_empty() {
            return Ok(());
        }
        let store = self.artifact_store.as_ref().ok_or_else(|| {
            "artifact recovery lost its workspace store before event publication".to_owned()
        })?;
        let envelope = ArtifactRecoveryEnvelope {
            conversation_id: source.conversation_id.clone(),
            wire_msg_id: source.wire_msg_id.clone(),
            event_kind: "tool_call".to_owned(),
            event_json: serde_json::to_string(data)
                .map_err(|error| format!("artifact recovery event serialization failed: {error}"))?,
        };
        store
            .prepare_recovery_receipts(&data.artifacts, &envelope)
            .map_err(|error| format!("artifact recovery envelope could not be committed: {error}"))
    }

    fn rollback_deferred_event_artifacts(&self, events: &[ToolCallEventData]) {
        let receipts = events
            .iter()
            .flat_map(|data| data.artifacts.iter().cloned())
            .collect::<Vec<_>>();
        if receipts.is_empty() {
            return;
        }
        if let Some(store) = self.artifact_store.as_ref() {
            let _ = store.rollback_owned_receipts(&receipts);
        }
    }

    fn emit_terminal_tool_result(
        &self,
        call_id: String,
        name: &str,
        is_error: bool,
        content: &str,
        artifacts: Vec<PersistedArtifact>,
    ) -> Result<(), String> {
        let terminal = self.take_deferred_tool_result(call_id.clone(), name, content);
        self.settle_artifact_obligation(&call_id, name, is_error, &artifacts);
        self.forget_active_tool_call(&call_id);
        let recovery_source = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recovery_source
            .clone();
        self.emit_deferred_tool_result_event(
            terminal,
            is_error,
            content.to_owned(),
            artifacts,
            recovery_source.as_ref(),
        )
    }

    fn retry_data(retry: &ToolCallRetryContext) -> Option<ToolCallRetryData> {
        let retry_group_id = Self::internal_call_id(&retry.retry_group_id)?;
        let retry_of_call_id = match retry.retry_of_call_id.as_deref() {
            Some(call_id) => Some(Self::internal_call_id(call_id)?),
            None => None,
        };
        Some(ToolCallRetryData {
            retry_group_id,
            attempt_no: retry.attempt_no,
            retry_of_call_id,
        })
    }

    fn internal_call_id(tool_use_id: &str) -> Option<String> {
        let id = tool_use_id.trim();
        if id.is_empty() || id != tool_use_id {
            None
        } else {
            Some(format!("nomi-{id}"))
        }
    }

    fn remember_active_tool_call(
        &self,
        call_id: String,
        name: String,
        artifact_identity: String,
        args: serde_json::Value,
        input: Option<serde_json::Value>,
        retry: Option<ToolCallRetryData>,
    ) {
        let artifact_path_baselines = if is_context_only_image_tool(&artifact_identity) {
            ArtifactPathBaselines::default()
        } else {
            self.capture_artifact_path_baselines(&artifact_identity, &args)
        };
        let (mut contract, contract_error) =
            match artifact_contract_with_input(&artifact_identity, &args) {
                Ok(contract) => (contract, None),
                Err(error) => (
                    artifact_contract(&artifact_identity),
                    Some(format!("invalid artifact contract input: {error}")),
                ),
            };
        if contract.is_none() && artifact_path_baselines.declares_artifact() {
            contract = Some(any_artifact_contract());
        }
        self.register_artifact_obligation(&call_id, &name, contract);
        if let Some(error) = contract_error.as_deref() {
            self.fail_artifact_obligation(&call_id, error);
        }
        match self.active_tool_calls.lock() {
            Ok(mut active) => {
                active.insert(
                    call_id.clone(),
                    ActiveToolCall {
                        call_id,
                        name,
                        artifact_identity,
                        args,
                        input,
                        contract,
                        contract_error,
                        artifact_path_baselines,
                        retry,
                    },
                );
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Failed to record active tool call for continuation cleanup"
                );
            }
        }
    }

    fn forget_active_tool_call(&self, call_id: &str) {
        match self.active_tool_calls.lock() {
            Ok(mut active) => {
                active.remove(call_id);
            }
            Err(poisoned) => {
                tracing::warn!(
                    error = %poisoned,
                    "Active tool-call lock was poisoned while settling a result"
                );
                poisoned.into_inner().remove(call_id);
            }
        }
    }

    fn active_tool_call(&self, call_id: &str) -> Option<ActiveToolCall> {
        match self.active_tool_calls.lock() {
            Ok(active) => active.get(call_id).cloned(),
            Err(poisoned) => {
                tracing::warn!(
                    error = %poisoned,
                    "Active tool-call lock was poisoned while verifying an artifact path"
                );
                poisoned.into_inner().get(call_id).cloned()
            }
        }
    }

    fn terminate_active_tool_calls(
        &self,
        status: ToolCallStatus,
        output: String,
        description: &str,
        lock_failure_context: &str,
    ) {
        // No result from an earlier stream may lend retry/argument metadata to
        // a later call that happens to reuse an id. Active calls carry their
        // own immutable copy for the terminal correction frames below.
        match self.tool_result_contexts.lock() {
            Ok(mut contexts) => contexts.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
        let interrupted: Vec<ActiveToolCall> = match self.active_tool_calls.lock() {
            Ok(mut active) => active.drain().map(|(_, data)| data).collect(),
            Err(poisoned) => {
                tracing::warn!(
                    error = %poisoned,
                    "{lock_failure_context}"
                );
                poisoned.into_inner().drain().map(|(_, data)| data).collect()
            }
        };

        for active in interrupted {
            self.fail_artifact_obligation(&active.call_id, &output);
            let _ = self.event_tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
                call_id: active.call_id,
                name: active.name,
                args: active.args,
                status,
                input: active.input,
                output: Some(output.clone()),
                description: Some(description.to_owned()),
                retry: active.retry,
                artifacts: Vec::new(),
            }));
        }
    }

    /// Fail every tool call already announced to the frontend but still lacking
    /// a real result. Provider/engine failures must not leave a permanent
    /// `Running` card that a later continuation can accidentally recover.
    pub(crate) fn fail_active_tool_calls(&self, reason: &str) {
        self.terminate_active_tool_calls(
            ToolCallStatus::Error,
            reason.to_owned(),
            "Tool call failed",
            "Failed to resolve active tool calls after turn failure",
        );
    }

    /// Cancel every tool call already announced to the frontend. The protocol
    /// currently has no `Cancelled` tool status, so cancellation uses the only
    /// non-success terminal status (`Error`) and carries the distinction in the
    /// description/output text.
    pub(crate) fn cancel_active_tool_calls(&self, reason: &str) {
        self.terminate_active_tool_calls(
            ToolCallStatus::Error,
            reason.to_owned(),
            "Tool call cancelled",
            "Failed to resolve active tool calls after turn cancellation",
        );
        self.abort_artifact_delivery_turn_with_reason(reason);
    }

    /// Citation reflow: parse the `<nomi-mem-citation>` block from the turn's
    /// final assistant text and bump each cited memory file's usage stats.
    /// Silent on every failure — a stale citation or unreadable file must
    /// never disrupt the turn.
    fn reflow_citations(&self, full_text: &str) {
        let Some(dir) = self.distill_dir.as_ref() else {
            return;
        };
        let now = chrono::Utc::now();
        for fname in nomi_memory::distill::parse_citation_filenames(full_text) {
            if let Err(e) = nomi_memory::store::bump_memory_usage(dir, &fname, now) {
                tracing::debug!(file = %fname, error = %e, "citation reflow bump failed");
            }
        }
    }
}

impl OutputSink for BackendOutputSink {
    fn emit_text_delta(&self, text: &str, _msg_id: &str) {
        {
            let mut turn = self
                .artifact_delivery_turn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if turn.active && turn.hold_text_until_verified {
                turn.held_text.push_str(text);
                return;
            }
        }
        // Accumulate for end-of-turn citation reflow (only when participating).
        if self.distill_dir.is_some()
            && let Ok(mut buf) = self.turn_text.lock()
        {
            buf.push_str(text);
        }
        let _ = self.event_tx.send(AgentStreamEvent::Text(TextEventData {
            content: text.to_owned(),
        }));
    }

    fn emit_thinking(&self, text: &str, _msg_id: &str) {
        {
            let turn = self
                .artifact_delivery_turn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if turn.active && turn.hold_text_until_verified {
                return;
            }
        }
        let _ = self.event_tx.send(AgentStreamEvent::Thinking(ThinkingEventData {
            content: text.to_owned(),
            subject: None,
            duration: None,
            status: None,
        }));
    }

    fn emit_model_activity(&self, _msg_id: &str, status: &str) {
        let _ = self
            .event_tx
            .send(AgentStreamEvent::AgentStatus(AgentStatusEventData {
                backend: "nomi".to_owned(),
                status: status.to_owned(),
                agent_name: Some("Nomi".to_owned()),
                session_id: None,
            }));
    }

    fn emit_tool_call(&self, tool_use_id: &str, name: &str, input: &str) {
        self.emit_tool_call_with_artifact_identity(tool_use_id, name, name, input);
    }

    fn emit_tool_call_with_artifact_identity(
        &self,
        tool_use_id: &str,
        name: &str,
        artifact_identity: &str,
        input: &str,
    ) {
        let parsed_input = serde_json::from_str(input)
            .unwrap_or(serde_json::Value::String(input.to_owned()));
        let Some(call_id) = Self::internal_call_id(tool_use_id) else {
            let (mut contract, contract_error) =
                match artifact_contract_with_input(artifact_identity, &parsed_input) {
                    Ok(contract) => (contract, None),
                    Err(error) => (
                        artifact_contract(artifact_identity),
                        Some(format!("invalid artifact contract input: {error}")),
                    ),
                };
            if contract.is_none() && input_artifact_paths(&parsed_input, false).saw_explicit_key {
                contract = Some(any_artifact_contract());
            }
            let reason = contract_error.as_deref().unwrap_or(
                "tool call has an empty or non-canonical call id",
            );
            self.record_unidentified_artifact_failure(
                name,
                contract,
                reason,
            );
            tracing::error!(
                tool = name,
                artifact_identity,
                "Cannot emit tool_call with empty or non-canonical tool_use_id"
            );
            return;
        };
        let retry = match self.tool_result_contexts.lock() {
            Ok(contexts) => contexts
                .get(&call_id)
                .and_then(|context| context.retry.clone()),
            Err(poisoned) => poisoned
                .into_inner()
                .get(&call_id)
                .and_then(|context| context.retry.clone()),
        };

        tracing::debug!(
            tool_use_id = %tool_use_id,
            call_id = %call_id,
            tool = name,
            status = ?ToolCallStatus::Running,
            "Derived internal tool_call id from nomi tool_use_id"
        );
        tracing::info!(
            tool_use_id = %tool_use_id,
            call_id = %call_id,
            tool = name,
            status = ?ToolCallStatus::Running,
            "Emitting nomi tool_call event"
        );

        self.remember_active_tool_call(
            call_id.clone(),
            name.to_owned(),
            artifact_identity.to_owned(),
            parsed_input.clone(),
            Some(parsed_input.clone()),
            retry.clone(),
        );

        let _ = self.event_tx.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id,
            name: name.to_owned(),
            args: parsed_input.clone(),
            status: ToolCallStatus::Running,
            input: Some(parsed_input),
            output: None,
            description: None,
            retry,
            artifacts: Vec::new(),
        }));
    }

    fn emit_tool_call_with_context(
        &self,
        tool_use_id: &str,
        name: &str,
        artifact_identity: &str,
        input: &str,
        context: &ToolCallExecutionContext,
    ) {
        if let Some(call_id) = Self::internal_call_id(tool_use_id) {
            let terminal_context = ToolTerminalContext {
                args: context.input.clone(),
                input: Some(context.input.clone()),
                retry: Self::retry_data(&context.retry),
            };
            match self.tool_result_contexts.lock() {
                Ok(mut contexts) => {
                    contexts.insert(call_id, terminal_context);
                }
                Err(poisoned) => {
                    poisoned.into_inner().insert(call_id, terminal_context);
                }
            }
        }
        self.emit_tool_call_with_artifact_identity(
            tool_use_id,
            name,
            artifact_identity,
            input,
        );
    }

    fn emit_tool_result(&self, tool_use_id: &str, name: &str, is_error: bool, content: &str) {
        let _ = self.emit_tool_result_with_images_and_artifact_identity(
            tool_use_id,
            name,
            name,
            is_error,
            content,
            &[],
        );
    }

    fn emit_tool_result_with_images(
        &self,
        tool_use_id: &str,
        name: &str,
        is_error: bool,
        content: &str,
        images: &[ToolImage],
    ) -> ToolMediaDelivery {
        self.emit_tool_result_with_images_and_artifact_identity(
            tool_use_id,
            name,
            name,
            is_error,
            content,
            images,
        )
    }

    fn emit_tool_result_with_images_and_artifact_identity(
        &self,
        tool_use_id: &str,
        name: &str,
        artifact_identity: &str,
        is_error: bool,
        content: &str,
        images: &[ToolImage],
    ) -> ToolMediaDelivery {
        let Some(call_id) = Self::internal_call_id(tool_use_id) else {
            let mut explicit_output = output_artifact_paths(content, false);
            let mut contract = artifact_contract(artifact_identity);
            explicit_output.enforce_resource_limits_if_artifact_expected(contract.is_some());
            if contract.is_none()
                && (!images.is_empty()
                    || explicit_output.saw_explicit_key
                    || !explicit_output.paths.is_empty()
                    || !explicit_output.errors.is_empty())
            {
                contract = Some(any_artifact_contract());
            }
            self.record_unidentified_artifact_failure(
                name,
                contract,
                "tool result has an empty or non-canonical call id",
            );
            tracing::error!(
                tool = name,
                "Cannot emit tool result with empty or non-canonical tool_use_id"
            );
            return ToolMediaDelivery::Failed {
                error: "tool result has no canonical call id; artifact was not written".to_owned(),
            };
        };

        // update_plan special case: emit a Plan event so the frontend renders
        // the checklist (MessagePlan) instead of a raw JSON tool card. This
        // lives in the shared result funnel so every emit_tool_result* path —
        // legacy, with-images, and the engine's with-context path — projects
        // the plan identically. Unparsable output falls through to a normal
        // tool result, and a result carrying inline images is never a plan
        // declaration: it keeps the funnel's fail-closed artifact accounting.
        if name == "update_plan"
            && !is_error
            && images.is_empty()
            && let Some(entries) = parse_plan_entries(content)
        {
            // Settle the full call lifecycle exactly like a terminal tool
            // result: drain the per-result context, fail any artifact
            // obligation (a plan declaration never delivers artifacts, and a
            // Running obligation would otherwise error the whole turn at
            // finish_artifact_delivery_turn), and forget the active call.
            let _ = self.take_tool_result_context(&call_id);
            self.fail_artifact_obligation(
                &call_id,
                "update_plan projected a plan checklist; it does not deliver artifacts",
            );
            self.forget_active_tool_call(&call_id);
            let _ = self.event_tx.send(AgentStreamEvent::Plan(PlanEventData {
                session_id: Some("update_plan".to_string()),
                source_call_id: Some(call_id),
                entries,
            }));
            return ToolMediaDelivery::Unmanaged;
        }

        // Failed tools may return diagnostic images. They remain transient
        // model context: never persist or publish them as successful artifacts.
        if is_error {
            let _ = self.emit_terminal_tool_result(call_id, name, true, content, Vec::new());
            return ToolMediaDelivery::Unmanaged;
        }

        let active = self.active_tool_call(&call_id);
        let effective_identity = active
            .as_ref()
            .map(|call| call.artifact_identity.as_str())
            .unwrap_or(artifact_identity);

        // Browser/computer screenshots are observational context, not durable
        // user-requested output. Do not create files or artifact receipts.
        if is_context_only_image_tool(effective_identity) {
            let _ = self.emit_terminal_tool_result(call_id, name, false, content, Vec::new());
            return ToolMediaDelivery::Unmanaged;
        }

        if let Some(error) = active
            .as_ref()
            .and_then(|call| call.contract_error.as_deref())
        {
            let error = error.to_owned();
            let output = Self::append_delivery_context(
                content,
                &format!("Artifact delivery failed: {error}"),
            );
            let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
            return ToolMediaDelivery::Failed { error };
        }

        let mut explicit_output = output_artifact_paths(content, false);
        let observed_contract = artifact_contract(artifact_identity);
        let mut contract = match (
            active.as_ref().and_then(|call| call.contract),
            observed_contract,
        ) {
            (Some(existing), Some(observed)) => match existing.merge(observed) {
                Ok(contract) => Some(contract),
                Err(error) => {
                    let error = format!("conflicting tool artifact identities: {error}");
                    self.fail_artifact_obligation(&call_id, &error);
                    let output = Self::append_delivery_context(
                        content,
                        &format!("Artifact delivery failed: {error}"),
                    );
                    let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
                    return ToolMediaDelivery::Failed { error };
                }
            },
            (Some(contract), None) | (None, Some(contract)) => Some(contract),
            (None, None) => None,
        };
        explicit_output.enforce_resource_limits_if_artifact_expected(contract.is_some());
        if contract.is_none()
            && (!images.is_empty()
                || explicit_output.saw_explicit_key
                || !explicit_output.paths.is_empty()
                || !explicit_output.errors.is_empty())
        {
            contract = Some(any_artifact_contract());
        }
        self.record_artifact_obligation(&call_id, name, contract);

        let mut declared_output = if contract.is_some() {
            output_artifact_paths(content, true)
        } else {
            explicit_output
        };
        declared_output.enforce_resource_limits_if_artifact_expected(contract.is_some());

        if images.is_empty()
            && contract.is_none()
            && !declared_output.saw_explicit_key
            && declared_output.paths.is_empty()
            && declared_output.errors.is_empty()
        {
            let _ = self.emit_terminal_tool_result(call_id, name, false, content, Vec::new());
            return ToolMediaDelivery::Unmanaged;
        }

        let contract = contract.unwrap_or_else(any_artifact_contract);

        // Some native and third-party generators write directly into the
        // workspace and return a structured/human-readable output path instead
        // of inline bytes. Accept only paths captured before the call and only
        // when an absent path appeared or an existing file's content hash
        // changed. Preflight these paths before writing inline bytes so mixed
        // output cannot leave a partial artifact batch.
        let existing_sources = match self.preflight_declared_path_artifacts(
            active.as_ref(),
            contract,
            &declared_output,
        ) {
            Ok(artifacts) => artifacts,
            Err(error) => {
                let output = Self::append_delivery_context(
                    content,
                    &format!("Artifact delivery failed: {error}"),
                );
                let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
                return ToolMediaDelivery::Failed { error };
            }
        };

        if let Some((index, artifact)) = images
            .iter()
            .enumerate()
            .find(|(_, artifact)| !contract.accepts_mime(&artifact.media_type))
        {
            let error = format!(
                "artifact-producing tool returned no {} satisfying the contract; inline artifact {index} has MIME {}",
                contract.label(),
                artifact.media_type,
            );
            let output = Self::append_delivery_context(
                content,
                &format!("Artifact delivery failed: {error}"),
            );
            let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
            return ToolMediaDelivery::Failed { error };
        }

        let actual_count = existing_sources.len().saturating_add(images.len());
        if actual_count < contract.expected_count() {
            let error = if actual_count == 0 && contract.expected_count() == 1 {
                format!("artifact-producing tool returned no {}", contract.label())
            } else {
                format!(
                    "artifact-producing tool returned {actual_count} verified candidate(s), expected at least {} {}(s)",
                    contract.expected_count(),
                    contract.label()
                )
            };
            let output = Self::append_delivery_context(
                content,
                &format!("Artifact delivery failed: {error}"),
            );
            let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
            return ToolMediaDelivery::Failed { error };
        }

        let Some(store) = self.artifact_store.as_ref() else {
            let error = "session has no workspace artifact store".to_owned();
            let output = Self::append_delivery_context(content, &format!("Artifact delivery failed: {error}"));
            let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
            return ToolMediaDelivery::Failed { error };
        };

        if self.should_defer_artifact_terminal() {
            let durable_workspace_targets = existing_sources
                .iter()
                .map(|source| source.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            return match self.queue_deferred_artifact_delivery(
                call_id.clone(),
                name,
                contract,
                content,
                images,
                existing_sources,
            ) {
                Ok(context) => ToolMediaDelivery::Delivered {
                    context,
                    durable_workspace_targets,
                },
                Err(error) => {
                    let output = Self::append_delivery_context(
                        content,
                        &format!("Artifact delivery failed: {error}"),
                    );
                    let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
                    ToolMediaDelivery::Failed { error }
                }
            };
        }

        let recovery_source = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recovery_source
            .clone();
        let inline = images.iter().map(|artifact| {
            (
                Self::inline_artifact_kind(&artifact.media_type),
                &artifact.media_type,
                &artifact.data,
            )
        });
        match if let Some(source) = recovery_source.as_ref() {
            store.persist_inline_and_verified_existing_batch_recoverable(
                inline,
                &existing_sources,
                source,
            )
        } else {
            store.persist_inline_and_verified_existing_batch(inline, &existing_sources)
        } {
            Ok(artifacts) => {
                let context = Self::delivery_context(&artifacts);
                let output = Self::append_delivery_context(content, &context);
                match self.emit_terminal_tool_result(call_id, name, false, &output, artifacts) {
                    Ok(()) => ToolMediaDelivery::Delivered {
                        context,
                        durable_workspace_targets: existing_sources
                            .iter()
                            .map(|source| source.path.to_string_lossy().into_owned())
                            .collect(),
                    },
                    Err(error) => ToolMediaDelivery::Failed { error },
                }
            }
            Err(error) => {
                let error = error.to_string();
                let output = Self::append_delivery_context(content, &format!("Artifact delivery failed: {error}"));
                let _ = self.emit_terminal_tool_result(call_id, name, true, &output, Vec::new());
                ToolMediaDelivery::Failed { error }
            }
        }
    }

    fn emit_tool_result_with_images_and_context(
        &self,
        tool_use_id: &str,
        name: &str,
        artifact_identity: &str,
        is_error: bool,
        content: &str,
        images: &[ToolImage],
        context: &ToolCallExecutionContext,
    ) -> ToolMediaDelivery {
        if let Some(call_id) = Self::internal_call_id(tool_use_id) {
            let terminal_context = ToolTerminalContext {
                args: context.input.clone(),
                input: Some(context.input.clone()),
                retry: Self::retry_data(&context.retry),
            };
            match self.tool_result_contexts.lock() {
                Ok(mut contexts) => {
                    contexts.insert(call_id, terminal_context);
                }
                Err(poisoned) => {
                    poisoned.into_inner().insert(call_id, terminal_context);
                }
            }
        }
        self.emit_tool_result_with_images_and_artifact_identity(
            tool_use_id,
            name,
            artifact_identity,
            is_error,
            content,
            images,
        )
    }

    fn emit_stream_start(&self, _msg_id: &str) {
        // A fresh stream is a lifecycle boundary. Normally the manager has
        // already resolved the prior pass (including MaxTokens auto-continue),
        // but fail any survivor defensively so it cannot be resurrected by a
        // later continuation.
        self.fail_active_tool_calls(
            "A new model stream started before the previous tool call reached a terminal state.",
        );
        let turn_text_len = self
            .turn_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let held_text_len = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .held_text
            .len();
        let checkpoint = SinkOutputCheckpoint {
            turn_text_len,
            held_text_len,
        };
        *self
            .attempt_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(checkpoint);
        self.accepted_turn_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(checkpoint);
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Start(StartEventData { session_id: None }));
    }

    fn emit_output_checkpoint(&self, _msg_id: &str) {
        let turn_text_len = self
            .turn_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let held_text_len = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .held_text
            .len();
        let checkpoint = SinkOutputCheckpoint {
            turn_text_len,
            held_text_len,
        };
        *self
            .attempt_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(checkpoint);
        self.accepted_turn_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(checkpoint);
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Start(StartEventData { session_id: None }));
    }

    fn emit_output_discarded(&self, _msg_id: &str, restart_attempt: u32) {
        let checkpoint = if restart_attempt == 0 {
            *self
                .accepted_turn_output_checkpoint
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        } else {
            *self
                .attempt_output_checkpoint
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        };
        let Some(checkpoint) = checkpoint else {
            let has_turn_text = !self
                .turn_text
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty();
            let has_held_text = !self
                .artifact_delivery_turn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .held_text
                .is_empty();
            let has_active_tool_calls = !self
                .active_tool_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty();
            if !has_turn_text && !has_held_text && !has_active_tool_calls {
                // A host may reject/cancel before the provider emits Start.
                // With no provisional output there is nothing to retract and
                // emitting a competing protocol Error would obscure the host's
                // authoritative terminal failure.
                return;
            }
            self.fail_active_tool_calls(
                "The model output was discarded before the tool call reached a terminal state.",
            );
            let _ = self.event_tx.send(AgentStreamEvent::Error(
                ErrorEventData::legacy(
                    "The model discarded provisional output without a stream-start checkpoint",
                    None,
                ),
            ));
            return;
        };
        self.fail_active_tool_calls(
            "The model output was discarded before the tool call reached a terminal state.",
        );
        let mut turn_text = self
            .turn_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut turn = self
            .artifact_delivery_turn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if checkpoint.turn_text_len > turn_text.len()
            || !turn_text.is_char_boundary(checkpoint.turn_text_len)
            || checkpoint.held_text_len > turn.held_text.len()
            || !turn.held_text.is_char_boundary(checkpoint.held_text_len)
        {
            drop(turn_text);
            drop(turn);
            let _ = self.event_tx.send(AgentStreamEvent::Error(
                ErrorEventData::legacy(
                    "The output checkpoint no longer matched the provisional text",
                    None,
                ),
            ));
            return;
        }
        turn_text.truncate(checkpoint.turn_text_len);
        turn.held_text.truncate(checkpoint.held_text_len);
        let retained = SinkOutputCheckpoint {
            turn_text_len: turn_text.len(),
            held_text_len: turn.held_text.len(),
        };
        drop(turn_text);
        drop(turn);
        *self
            .attempt_output_checkpoint
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(retained);
        let _ = self.event_tx.send(AgentStreamEvent::OutputDiscarded(
            OutputDiscardedEventData { restart_attempt },
        ));
    }

    fn emit_stream_end(
        &self,
        _msg_id: &str,
        _turns: usize,
        _input_tokens: u64,
        _output_tokens: u64,
        _cache_creation_tokens: u64,
        _cache_read_tokens: u64,
    ) {
        // Citation reflow: parse the accumulated assistant text and bump the
        // cited memory files. Take the buffer so it doesn't linger.
        if self.distill_dir.is_some() {
            let full = self
                .turn_text
                .lock()
                .map(|mut b| std::mem::take(&mut *b))
                .unwrap_or_default();
            if !full.is_empty() {
                self.reflow_citations(&full);
            }
        }
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Finish(FinishEventData {
                session_id: None,
                stop_reason: None,
            }));
    }

    fn emit_error(&self, msg: &str) {
        let _ = self
            .event_tx
            .send(AgentStreamEvent::Error(ErrorEventData::legacy(msg, None)));
    }

    fn emit_info(&self, msg: &str) {
        let _ = self.event_tx.send(AgentStreamEvent::Tips(TipsEventData {
            content: msg.to_owned(),
            tip_type: TipType::Success,
        }));
    }

    fn emit_warning(&self, msg: &str) {
        // Benign, non-fatal diagnostic: emit as Tips{Warning} on the broadcast —
        // NOT an Error — so the AutoWork runner does not read
        // an otherwise-successful turn as failed. See OutputSink::emit_warning.
        let _ = self.event_tx.send(AgentStreamEvent::Tips(TipsEventData {
            content: msg.to_owned(),
            tip_type: TipType::Warning,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sink() -> (BackendOutputSink, broadcast::Receiver<AgentStreamEvent>) {
        let (tx, rx) = broadcast::channel(16);
        (BackendOutputSink::new(tx), rx)
    }

    #[test]
    fn artifact_baseline_hash_budget_is_bounded_per_file_and_batch() {
        assert_eq!(
            reserve_artifact_baseline_hash_bytes(0, MAX_BASELINE_ARTIFACT_FILE_BYTES)
                .unwrap(),
            MAX_BASELINE_ARTIFACT_FILE_BYTES
        );
        assert_eq!(
            reserve_artifact_baseline_hash_bytes(
                MAX_BASELINE_ARTIFACT_BATCH_BYTES - MAX_BASELINE_ARTIFACT_FILE_BYTES,
                MAX_BASELINE_ARTIFACT_FILE_BYTES,
            )
            .unwrap(),
            MAX_BASELINE_ARTIFACT_BATCH_BYTES
        );

        let per_file = reserve_artifact_baseline_hash_bytes(
            0,
            MAX_BASELINE_ARTIFACT_FILE_BYTES + 1,
        )
        .unwrap_err();
        assert!(per_file.contains("per-file hash limit"), "{per_file}");

        let aggregate = reserve_artifact_baseline_hash_bytes(
            MAX_BASELINE_ARTIFACT_BATCH_BYTES - MAX_BASELINE_ARTIFACT_FILE_BYTES + 1,
            MAX_BASELINE_ARTIFACT_FILE_BYTES,
        )
        .unwrap_err();
        assert!(aggregate.contains("aggregate hash limit"), "{aggregate}");
    }

    #[test]
    fn sparse_oversized_artifact_baseline_is_rejected_before_hashing() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("existing-large.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_BASELINE_ARTIFACT_FILE_BYTES + 1)
            .unwrap();
        drop(file);

        let mut batch_hash_bytes = 0;
        let error = capture_path_fingerprint(&path, &mut batch_hash_bytes).unwrap_err();

        assert!(error.contains("per-file hash limit"), "{error}");
        assert_eq!(batch_hash_bytes, 0, "a rejected file must not consume the batch budget");
    }

    #[test]
    fn artifact_baseline_identity_rejects_a_same_size_rename_replacement() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("report.md");
        let displaced = workspace.path().join("report.before.md");
        std::fs::write(&path, b"old").unwrap();
        let original = File::open(&path).unwrap();
        ensure_artifact_baseline_path_identity(&path, &original).unwrap();

        std::fs::rename(&path, &displaced).unwrap();
        std::fs::write(&path, b"new").unwrap();

        let error = ensure_artifact_baseline_path_identity(&path, &original).unwrap_err();
        assert!(error.contains("replaced"), "{error}");
    }

    #[test]
    fn absent_and_small_artifact_baselines_keep_existing_behavior() {
        let workspace = tempfile::tempdir().unwrap();
        let absent = workspace.path().join("future-report.md");
        let existing = workspace.path().join("existing-report.md");
        let body = b"# Existing report\n";
        std::fs::write(&existing, body).unwrap();
        let mut batch_hash_bytes = 0;

        assert!(matches!(
            capture_path_fingerprint(&absent, &mut batch_hash_bytes).unwrap(),
            ArtifactPathFingerprint::Absent
        ));
        assert_eq!(batch_hash_bytes, 0, "an absent output performs no synchronous hashing");

        match capture_path_fingerprint(&existing, &mut batch_hash_bytes).unwrap() {
            ArtifactPathFingerprint::Present { size_bytes, sha256 } => {
                assert_eq!(size_bytes, body.len() as u64);
                assert_eq!(sha256, hex::encode(Sha256::digest(body)));
            }
            ArtifactPathFingerprint::Absent => panic!("the existing file lost its baseline"),
        }
        assert_eq!(batch_hash_bytes, body.len() as u64);
    }

    #[test]
    fn emit_text_delta_sends_text_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_text_delta("hello", "msg-1");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Text(data) => assert_eq!(data.content, "hello"),
            other => panic!("Expected Text, got {:?}", other),
        }
    }

    #[test]
    fn emit_thinking_sends_thinking_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_thinking("analyzing...", "msg-1");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Thinking(data) => assert_eq!(data.content, "analyzing..."),
            other => panic!("Expected Thinking, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_call_sends_running_tool_call() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_call("call_read_1", "Read", r#"{"path":"/tmp/a.txt"}"#);
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.name, "Read");
                assert_eq!(data.status, ToolCallStatus::Running);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn retry_identity_uses_the_same_internal_call_id_domain_as_events() {
        let (sink, mut rx) = make_sink();
        let first = ToolCallExecutionContext {
            input: serde_json::json!({ "tasks": ["invalid"] }),
            retry: ToolCallRetryContext {
                retry_group_id: "call-1".to_owned(),
                attempt_no: 1,
                retry_of_call_id: None,
            },
        };
        sink.emit_tool_call_with_context(
            "call-1",
            "nomi_delegate",
            "nomi_delegate",
            r#"{"tasks":["invalid"]}"#,
            &first,
        );
        let first_running = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data,
            other => panic!("Expected ToolCall, got {other:?}"),
        };
        assert_eq!(first_running.call_id, "nomi-call-1");
        assert_eq!(
            first_running.retry.as_ref().unwrap().retry_group_id,
            first_running.call_id
        );

        sink.emit_tool_result_with_images_and_context(
            "call-1",
            "nomi_delegate",
            "nomi_delegate",
            true,
            "invalid arguments",
            &[],
            &first,
        );
        let first_terminal = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data,
            other => panic!("Expected ToolCall, got {other:?}"),
        };
        assert_eq!(first_terminal.args, first.input);
        assert_eq!(first_terminal.retry, first_running.retry);

        let second = ToolCallExecutionContext {
            input: serde_json::json!({ "tasks": [{ "title": "valid" }] }),
            retry: ToolCallRetryContext {
                retry_group_id: "call-1".to_owned(),
                attempt_no: 2,
                retry_of_call_id: Some("call-1".to_owned()),
            },
        };
        sink.emit_tool_call_with_context(
            "call-2",
            "nomi_delegate",
            "nomi_delegate",
            r#"{"tasks":[{"title":"valid"}]}"#,
            &second,
        );
        let second_running = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data,
            other => panic!("Expected ToolCall, got {other:?}"),
        };
        let retry = second_running.retry.unwrap();
        assert_eq!(retry.retry_group_id, "nomi-call-1");
        assert_eq!(retry.retry_of_call_id.as_deref(), Some("nomi-call-1"));
        assert_eq!(retry.attempt_no, 2);
    }

    #[test]
    fn preflight_failure_preserves_rejected_args_and_retry_identity() {
        let (sink, mut rx) = make_sink();
        let context = ToolCallExecutionContext {
            input: serde_json::json!({ "tasks": ["invalid"] }),
            retry: ToolCallRetryContext {
                retry_group_id: "invalid-1".to_owned(),
                attempt_no: 1,
                retry_of_call_id: None,
            },
        };

        sink.emit_tool_result_with_images_and_context(
            "invalid-1",
            "nomi_delegate",
            "nomi_delegate",
            true,
            "invalid arguments",
            &[],
            &context,
        );

        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.args, context.input);
                assert_eq!(data.input, Some(context.input));
                assert_eq!(data.retry.unwrap().retry_group_id, data.call_id);
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn stream_termination_clears_short_lived_result_contexts() {
        let (sink, mut rx) = make_sink();
        let context = ToolCallExecutionContext {
            input: serde_json::json!({ "path": "a" }),
            retry: ToolCallRetryContext {
                retry_group_id: "reused".to_owned(),
                attempt_no: 1,
                retry_of_call_id: None,
            },
        };
        sink.emit_tool_call_with_context(
            "reused",
            "Read",
            "Read",
            r#"{"path":"a"}"#,
            &context,
        );
        let _running = rx.try_recv().unwrap();
        sink.fail_active_tool_calls("interrupted");
        let interrupted = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data,
            other => panic!("Expected ToolCall, got {other:?}"),
        };
        assert!(interrupted.retry.is_some());

        sink.emit_tool_result("reused", "Read", true, "late legacy result");
        let late = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data,
            other => panic!("Expected ToolCall, got {other:?}"),
        };
        assert!(late.retry.is_none());
        assert_eq!(late.args, serde_json::Value::Null);
    }

    #[test]
    fn fail_active_tool_calls_marks_pending_tool_error_and_drains_it() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_call(
            "call_write_1",
            "Write",
            r#"{"file_path":"/tmp/index.html"}"#,
        );
        let _running = rx.try_recv().unwrap();

        sink.fail_active_tool_calls("provider rejected the structured arguments");

        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.call_id, "nomi-call_write_1");
                assert_eq!(data.status, ToolCallStatus::Error);
                assert_eq!(data.description.as_deref(), Some("Tool call failed"));
                assert_eq!(
                    data.output.as_deref(),
                    Some("provider rejected the structured arguments")
                );
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }

        sink.fail_active_tool_calls("a second attempt at the same call");
        assert!(rx.try_recv().is_err(), "a failed call must not be recovered twice");
    }

    #[test]
    fn stream_start_fails_stale_tool_before_emitting_start() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_call("stale", "Write", "{}");
        let _running = rx.try_recv().unwrap();

        sink.emit_stream_start("next-msg");

        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.call_id, "nomi-stale");
                assert_eq!(data.status, ToolCallStatus::Error);
            }
            other => panic!("Expected stale ToolCall cleanup, got {:?}", other),
        }
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
    }

    #[test]
    fn emit_model_activity_sends_agent_status() {
        let (sink, mut rx) = make_sink();
        sink.emit_model_activity("msg-1", "preparing");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::AgentStatus(data) => {
                assert_eq!(data.backend, "nomi");
                assert_eq!(data.status, "preparing");
                assert_eq!(data.agent_name.as_deref(), Some("Nomi"));
            }
            other => panic!("Expected AgentStatus, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_result_success_sends_completed() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_result("call_read_1", "Read", false, "file content here");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.name, "Read");
                assert_eq!(data.status, ToolCallStatus::Completed);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_result_error_sends_error_status() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_result("call_bash_1", "Bash", true, "command failed");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.name, "Bash");
                assert_eq!(data.status, ToolCallStatus::Error);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn emit_warning_is_a_non_failing_tip_not_an_error_event() {
        // Benign, non-fatal diagnostics (autocompact failure, session save/index
        // hiccup, MCP-init failure, /compact failure) must reach the stream as a
        // non-failing Tips{Warning} — NOT an Error. The AutoWork / requirement
        // AutoWork runner classifies any non-retryable Error stream event as a FAILED
        // turn, so routing a benign warning through emit_error would re-pend the
        // requirement / burn an attempt / pause the tag on an otherwise-successful
        // turn (the regression this guards against).
        let (sink, mut rx) = make_sink();
        sink.emit_warning("Failed to save session: disk full");
        match rx.try_recv().expect("a warning event should be emitted") {
            AgentStreamEvent::Tips(data) => {
                assert_eq!(data.tip_type, TipType::Warning);
                assert!(data.content.contains("Failed to save session"));
            }
            other => panic!("emit_warning must be a non-failing Tips(Warning), got {:?}", other),
        }
    }

    #[test]
    fn duplicate_tool_names_use_distinct_internal_call_ids() {
        let (sink, mut rx) = make_sink();

        sink.emit_tool_call("call_a", "Glob", r#"{"pattern":"*.rs"}"#);
        sink.emit_tool_call("call_b", "Glob", r#"{"pattern":"*.toml"}"#);
        sink.emit_tool_result("call_a", "Glob", false, "first");
        sink.emit_tool_result("call_b", "Glob", false, "second");

        let events = (0..4).map(|_| rx.try_recv().unwrap()).collect::<Vec<_>>();

        let mut call_ids = vec![];
        for event in events {
            match event {
                AgentStreamEvent::ToolCall(data) => call_ids.push((data.call_id, data.status)),
                other => panic!("Expected ToolCall, got {:?}", other),
            }
        }

        assert_eq!(call_ids[0].0, "nomi-call_a");
        assert_eq!(call_ids[1].0, "nomi-call_b");
        assert_eq!(call_ids[2].0, "nomi-call_a");
        assert_eq!(call_ids[3].0, "nomi-call_b");
        assert_eq!(call_ids[2].1, ToolCallStatus::Completed);
        assert_eq!(call_ids[3].1, ToolCallStatus::Completed);
    }

    #[test]
    fn whitespace_variant_tool_ids_cannot_alias_a_canonical_active_call() {
        let (sink, mut rx) = make_sink();

        sink.emit_tool_call("x", "Read", r#"{"path":"a"}"#);
        let running = rx.try_recv().unwrap();
        assert!(matches!(
            running,
            AgentStreamEvent::ToolCall(ref data) if data.call_id == "nomi-x"
        ));

        sink.emit_tool_call(" x ", "Read", r#"{"path":"b"}"#);
        sink.emit_tool_call("\tx", "Read", "{}");
        sink.emit_tool_result("x ", "Read", false, "wrong call");
        assert!(
            rx.try_recv().is_err(),
            "non-canonical IDs must not emit or settle a colliding lifecycle"
        );

        sink.emit_tool_result("x", "Read", false, "ok");
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.call_id, "nomi-x");
                assert_eq!(data.status, ToolCallStatus::Completed);
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn emit_stream_start_sends_start_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_stream_start("msg-1");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Start(_) => {}
            other => panic!("Expected Start, got {:?}", other),
        }
    }

    #[test]
    fn output_discard_restores_held_text_checkpoint_and_clears_tool_survivors() {
        let (sink, mut rx) = make_sink();
        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_stream_start("msg-1");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta("retained prefix", "msg-1");
        assert!(rx.try_recv().is_err(), "receipt-gated text stays provisional");
        sink.turn_text.lock().unwrap().push_str("citation prefix");
        // A same-execute provider pass establishes the boundary without
        // erasing accepted receipt-gated or citation-reflow text.
        sink.emit_output_checkpoint("msg-1");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta("discard me", "msg-1");
        sink.turn_text.lock().unwrap().push_str("citation draft");
        sink.emit_tool_call("running", "Read", "{}");
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::ToolCall(ToolCallEventData {
                status: ToolCallStatus::Running,
                ..
            })
        ));

        sink.emit_output_discarded("msg-1", 2);

        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::ToolCall(ToolCallEventData {
                status: ToolCallStatus::Error,
                ..
            })
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                restart_attempt: 2,
            })
        ));
        assert!(sink.active_tool_calls.lock().unwrap().is_empty());
        assert_eq!(*sink.turn_text.lock().unwrap(), "citation prefix");
        let turn = sink.artifact_delivery_turn.lock().unwrap();
        assert!(turn.active);
        assert!(turn.required_contract.is_some());
        assert!(turn.hold_text_until_verified);
        assert_eq!(turn.held_text, "retained prefix");
    }

    #[test]
    fn accepted_turn_discard_restores_the_immutable_first_start_across_race_tail() {
        let (sink, mut rx) = make_sink();
        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();

        sink.emit_stream_start("msg-root");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta("pass A", "msg-root");
        sink.turn_text.lock().unwrap().push_str("citation A");

        // A host race-tail execute emits another ordinary Start. It advances
        // the rolling attempt boundary but must not replace the accepted root.
        sink.emit_stream_start("msg-root");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta(" + pass B", "msg-root");
        sink.turn_text.lock().unwrap().push_str(" + citation B");

        sink.emit_accepted_turn_output_discarded("msg-root");
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                restart_attempt: 0
            })
        ));
        assert!(sink.turn_text.lock().unwrap().is_empty());
        assert!(
            sink.artifact_delivery_turn
                .lock()
                .unwrap()
                .held_text
                .is_empty()
        );
    }

    #[test]
    fn accepted_turn_discard_before_start_is_a_noop_without_provisional_output() {
        let (sink, mut rx) = make_sink();
        sink.begin_artifact_delivery_turn();

        sink.emit_accepted_turn_output_discarded("msg-before-start");

        assert!(
            rx.try_recv().is_err(),
            "an empty pre-Start rollback must leave the host terminal authoritative"
        );
        assert!(sink.turn_text.lock().unwrap().is_empty());
        assert!(
            sink.artifact_delivery_turn
                .lock()
                .unwrap()
                .held_text
                .is_empty()
        );
        assert!(sink.active_tool_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn discard_before_start_fails_closed_for_each_kind_of_provisional_output() {
        let assert_legacy_error = |mut rx: broadcast::Receiver<AgentStreamEvent>| {
            assert!(matches!(
                rx.try_recv().unwrap(),
                AgentStreamEvent::Error(ErrorEventData { ref message, .. })
                    if message.contains("without a stream-start checkpoint")
            ));
        };

        let (turn_text_sink, turn_text_rx) = make_sink();
        turn_text_sink
            .turn_text
            .lock()
            .unwrap()
            .push_str("draft citation text");
        turn_text_sink.emit_accepted_turn_output_discarded("turn-text");
        assert_legacy_error(turn_text_rx);

        let (held_text_sink, held_text_rx) = make_sink();
        held_text_sink
            .artifact_delivery_turn
            .lock()
            .unwrap()
            .held_text
            .push_str("receipt-gated draft");
        held_text_sink.emit_accepted_turn_output_discarded("held-text");
        assert_legacy_error(held_text_rx);

        let (active_tool_sink, mut active_tool_rx) = make_sink();
        active_tool_sink.emit_tool_call("running", "Read", "{}");
        assert!(matches!(
            active_tool_rx.try_recv().unwrap(),
            AgentStreamEvent::ToolCall(ToolCallEventData {
                status: ToolCallStatus::Running,
                ..
            })
        ));
        active_tool_sink.emit_accepted_turn_output_discarded("active-tool");
        assert!(matches!(
            active_tool_rx.try_recv().unwrap(),
            AgentStreamEvent::ToolCall(ToolCallEventData {
                status: ToolCallStatus::Error,
                ..
            })
        ));
        assert_legacy_error(active_tool_rx);
    }

    #[test]
    fn emit_stream_end_sends_finish_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_stream_end("msg-1", 3, 1000, 500, 100, 200);
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Finish(_) => {}
            other => panic!("Expected Finish, got {:?}", other),
        }
    }

    #[test]
    fn emit_error_sends_error_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_error("something went wrong");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Error(data) => assert_eq!(data.message, "something went wrong"),
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn emit_info_sends_tips_event() {
        let (sink, mut rx) = make_sink();
        sink.emit_info("operation completed");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::Tips(data) => {
                assert_eq!(data.content, "operation completed");
                assert_eq!(data.tip_type, TipType::Success);
            }
            other => panic!("Expected Tips, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_call_carries_input() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_call("call_glob_1", "Glob", r#"{"pattern":"**/*.rs"}"#);
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.name, "Glob");
                assert_eq!(data.status, ToolCallStatus::Running);
                assert!(data.input.is_some());
                assert_eq!(data.input.unwrap()["pattern"], "**/*.rs");
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_result_carries_output_and_matching_call_id() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_call("call_glob_1", "Glob", r#"{"pattern":"**/*.rs"}"#);
        let start_event = rx.try_recv().unwrap();
        let start_call_id = match &start_event {
            AgentStreamEvent::ToolCall(data) => data.call_id.clone(),
            _ => panic!("Expected ToolCall"),
        };

        sink.emit_tool_result("call_glob_1", "Glob", false, "src/main.rs\nsrc/lib.rs");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.name, "Glob");
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.call_id, start_call_id);
                assert_eq!(data.output.as_deref(), Some("src/main.rs\nsrc/lib.rs"));
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn emit_tool_result_empty_content_omits_output() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_result("call_glob_1", "Glob", false, "");
        let event = rx.try_recv().unwrap();
        match event {
            AgentStreamEvent::ToolCall(data) => {
                assert!(data.output.is_none());
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn valid_image_is_verified_persisted_and_attached_before_completed() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call("image-1", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "image-1",
            "image_gen",
            false,
            "",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );

        let ToolMediaDelivery::Delivered {
            durable_workspace_targets,
            ..
        } = delivery
        else {
            panic!("a verified inline image was not delivered");
        };
        assert!(
            durable_workspace_targets.is_empty(),
            "inline-only artifacts are immutable store receipts, not workspace-path effects"
        );
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(std::path::Path::new(&data.artifacts[0].path).is_file());
                assert_eq!(data.artifacts[0].mime_type, "image/png");
                assert!(data.output.unwrap().contains("Verified artifacts saved to:"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn generic_audio_and_resource_descriptor_are_verified_before_completed() {
        use base64::Engine as _;

        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call("export-1", "mcp__reports__export", "{}");
        let _running = rx.try_recv().unwrap();
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&38_u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 1, 0]);
        wav.extend_from_slice(&8_000_u32.to_le_bytes());
        wav.extend_from_slice(&8_000_u32.to_le_bytes());
        wav.extend_from_slice(&[1, 0, 8, 0]);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1_u32.to_le_bytes());
        wav.extend_from_slice(&[128, 0]);

        let delivery = sink.emit_tool_result_with_images(
            "export-1",
            "mcp__reports__export",
            false,
            "Resource link: report — https://example.test/report.pdf",
            &[
                ToolImage {
                    media_type: "audio/wav".into(),
                    data: base64::engine::general_purpose::STANDARD.encode(wav),
                },
                ToolImage {
                    media_type: "application/json".into(),
                    data: "eyJ1cmkiOiJodHRwczovL2UifQ==".into(),
                },
            ],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Delivered { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 2);
                assert_eq!(data.artifacts[0].kind, ArtifactKind::Audio);
                assert_eq!(data.artifacts[1].kind, ArtifactKind::Text);
                assert!(data.artifacts.iter().all(|artifact| {
                    std::path::Path::new(&artifact.path).is_file()
                        && artifact.size_bytes > 0
                        && !artifact.sha256.is_empty()
                }));
                let output = data.output.unwrap();
                assert!(output.contains("https://example.test/report.pdf"));
                assert!(output.contains("Verified artifacts saved to:"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn image_generator_cannot_complete_with_only_a_file_artifact() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let delivery = sink.emit_tool_result_with_images(
            "image-1",
            "image_gen",
            false,
            "generated",
            &[ToolImage {
                media_type: "text/plain".into(),
                data: "bm90IGFuIGltYWdl".into(),
            }],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("no image artifact"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn image_generator_without_an_image_is_failed_not_completed() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let delivery = sink.emit_tool_result_with_images("image-1", "image_gen", false, "done", &[]);

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("returned no image artifact"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn report_export_without_a_file_is_failed_not_completed() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let delivery = sink.emit_tool_result_with_images(
            "report-1",
            "mcp__reports__export_report",
            false,
            "Report generated successfully",
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("returned no file artifact"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn exact_format_tools_reject_wrong_format_receipts_before_persistence() {
        const SAMPLE_BASE64: &str =
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let cases = [
            ("renderPng", "image/jpeg", "PNG image artifact"),
            ("generateMp3", "audio/wav", "MP3 audio artifact"),
            ("exportMp4", "video/webm", "MP4 video artifact"),
            ("exportPdf", "text/plain", "PDF artifact"),
        ];

        for (index, (tool_name, wrong_mime, expected_label)) in cases.into_iter().enumerate() {
            let workspace = tempfile::tempdir().unwrap();
            let (tx, mut rx) = broadcast::channel(8);
            let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
            let call_id = format!("wrong-format-{index}");
            sink.begin_artifact_delivery_turn();
            sink.emit_tool_call(&call_id, tool_name, "{}");
            let _running = rx.try_recv().unwrap();

            let delivery = sink.emit_tool_result_with_images(
                &call_id,
                tool_name,
                false,
                "claimed success",
                &[ToolImage {
                    media_type: wrong_mime.to_owned(),
                    data: SAMPLE_BASE64.to_owned(),
                }],
            );

            let ToolMediaDelivery::Failed { error } = delivery else {
                panic!("{tool_name} accepted {wrong_mime}");
            };
            assert!(error.contains(expected_label), "{tool_name}: {error}");
            match rx.try_recv().unwrap() {
                AgentStreamEvent::ToolCall(data) => {
                    assert_eq!(data.status, ToolCallStatus::Error);
                    assert!(data.artifacts.is_empty());
                }
                other => panic!("Expected ToolCall, got {other:?}"),
            }
            assert!(sink.finish_artifact_delivery_turn().is_err());
            assert!(!workspace.path().join("nomifun-artifacts").exists());
        }
    }

    #[test]
    fn requested_image_count_is_a_minimum_verified_receipt_contract() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.emit_tool_call("count-short", "image_gen", r#"{"n":4}"#);
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images(
            "count-short",
            "image_gen",
            false,
            "generated",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        let ToolMediaDelivery::Failed { error } = delivery else {
            panic!("one receipt incorrectly satisfied n=4");
        };
        assert!(error.contains("expected at least 4"));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());

        sink.emit_tool_call("count-good", "image_gen", r#"{"num_images":4}"#);
        let _running = rx.try_recv().unwrap();
        let images = (0..4)
            .map(|_| ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "count-good",
                "image_gen",
                false,
                "generated",
                &images,
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 4);
                assert!(data.artifacts.iter().all(|artifact| {
                    artifact.mime_type == "image/png"
                        && std::path::Path::new(&artifact.path).is_file()
                }));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn long_mcp_identity_cannot_lose_export_pdf_obligation_to_display_hashing() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        let display_name = "mcp__very_long_server__7f6e5d4c";
        let artifact_identity = format!("{}__export_pdf", "very_long_server_segment_".repeat(20));
        assert!(artifact_contract(display_name).is_none());
        assert_eq!(
            artifact_contract(&artifact_identity).unwrap().requirement,
            ArtifactRequirement::Pdf
        );

        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call_with_artifact_identity(
            "long-pdf",
            display_name,
            &artifact_identity,
            "{}",
        );
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images_and_artifact_identity(
            "long-pdf",
            display_name,
            &artifact_identity,
            false,
            "PDF exported successfully",
            &[],
        );

        let ToolMediaDelivery::Failed { error } = delivery else {
            panic!("hashed display name bypassed its PDF contract");
        };
        assert!(error.contains("PDF artifact"));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(sink.finish_artifact_delivery_turn().is_err());
    }

    #[test]
    fn freshly_written_declared_output_path_is_verified_and_attached() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call(
            "report-path",
            "mcp__reports__export_report",
            r#"{"options":{"outputPath":"report.md"}}"#,
        );
        let _running = rx.try_recv().unwrap();
        let report_path = workspace.path().join("report.md");
        std::fs::write(&report_path, "# Generated report\n").unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "report-path",
            "mcp__reports__export_report",
            false,
            r#"{"path":"report.md"}"#,
            &[],
        );

        let ToolMediaDelivery::Delivered {
            durable_workspace_targets,
            ..
        } = delivery
        else {
            panic!("a verified declared path was not delivered");
        };
        assert_eq!(
            durable_workspace_targets,
            vec![std::fs::canonicalize(&report_path)
                .unwrap()
                .to_string_lossy()
                .into_owned()],
            "only the verified caller-owned workspace source is completion evidence"
        );
        let artifact = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(
                    data.artifacts[0]
                        .relative_path
                        .starts_with("nomifun-artifacts/artifact-")
                );
                assert_eq!(data.artifacts[0].mime_type, "text/markdown");
                data.artifacts[0].clone()
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        };

        // The published receipt is an immutable snapshot. A later tool in the
        // same accepted turn may overwrite or delete the caller-owned path,
        // but that must not invalidate a green terminal delivery.
        std::fs::write(workspace.path().join("report.md"), "# Replaced later\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&artifact.path).unwrap(),
            "# Generated report\n"
        );
        assert_ne!(artifact.path, workspace.path().join("report.md").to_string_lossy());
    }

    #[test]
    fn artifact_path_contract_over_limit_fails_instead_of_truncating() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        let paths = (0..=MAX_DECLARED_ARTIFACT_PATHS)
            .map(|index| format!("result-{index}.md"))
            .collect::<Vec<_>>();
        let contract = serde_json::json!({ "outputPaths": paths }).to_string();

        sink.emit_tool_call("too-many-paths", "exportReport", &contract);
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images(
            "too-many-paths",
            "exportReport",
            false,
            &contract,
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("more than 32 distinct paths"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn ordinary_large_execution_json_is_not_misclassified_as_an_artifact() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        let steps = (0..600)
            .map(|index| {
                serde_json::json!({
                    "id": format!("step-{index}"),
                    "status": "completed",
                    "attempts": [{
                        "sequence": 1,
                        "status": "completed",
                        "output_files": if index == 0 {
                            serde_json::json!(["reports/from-prior-agent.md"])
                        } else {
                            serde_json::json!([])
                        },
                    }],
                    "dependencies": [],
                })
            })
            .collect::<Vec<_>>();
        let content = serde_json::json!({
            "execution": {
                "id": "019f8fcb-1e47-7893-82db-c03aab79a2c4",
                "status": "running",
                "steps": steps,
            }
        })
        .to_string();

        assert!(artifact_contract("nomi_execution_get").is_none());
        sink.emit_tool_call(
            "execution-get",
            "nomi_execution_get",
            r#"{"execution_id":"019f8fcb-1e47-7893-82db-c03aab79a2c4"}"#,
        );
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images(
            "execution-get",
            "nomi_execution_get",
            false,
            &content,
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Unmanaged));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert!(data.artifacts.is_empty());
                assert_eq!(data.output.as_deref(), Some(content.as_str()));
                assert!(!data.output.unwrap().contains("Artifact delivery failed"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn nested_input_output_path_still_creates_a_pre_call_declaration() {
        let declared = input_artifact_paths(
            &serde_json::json!({
                "options": {
                    "outputPath": "reports/generated.md"
                }
            }),
            false,
        );

        assert!(declared.saw_explicit_key);
        assert_eq!(declared.paths, vec!["reports/generated.md"]);
        assert!(declared.errors.is_empty());
    }

    #[test]
    fn read_model_input_scopes_never_create_artifact_declarations() {
        let declared = input_artifact_paths(
            &serde_json::json!({
                "filter": { "output_file": "reports/prior.md" },
                "query": { "where": { "output_path": "reports/prior.md" } },
                "history": { "attempts": [{ "output_file": "reports/prior.md" }] }
            }),
            true,
        );

        assert!(!declared.saw_explicit_key);
        assert!(declared.paths.is_empty());
        assert!(declared.errors.is_empty());
    }

    #[test]
    fn read_only_mcp_filter_output_file_is_not_an_artifact_obligation() {
        let workspace = tempfile::tempdir().unwrap();
        let prior = workspace.path().join("reports").join("prior.md");
        std::fs::create_dir_all(prior.parent().unwrap()).unwrap();
        std::fs::write(&prior, "# Prior run\n").unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        let tool = "mcp__runs__search_runs";
        assert!(artifact_contract(tool).is_none());

        sink.emit_tool_call(
            "search-runs",
            tool,
            r#"{"filter":{"output_file":"reports/prior.md"}}"#,
        );
        let _running = rx.try_recv().unwrap();
        let content = r#"{"runs":[]}"#;
        let delivery =
            sink.emit_tool_result_with_images("search-runs", tool, false, content, &[]);

        assert!(matches!(delivery, ToolMediaDelivery::Unmanaged));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert!(data.artifacts.is_empty());
                assert_eq!(data.output.as_deref(), Some(content));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn output_path_inference_requires_root_or_result_scope() {
        let ordinary = output_artifact_paths(
            r#"{"execution":{"attempts":[{"output_files":["prior.md"]}]}}"#,
            false,
        );
        assert!(!ordinary.saw_explicit_key);
        assert!(ordinary.paths.is_empty());

        let nested_read_model = output_artifact_paths(
            r#"{"result":{"attempts":[{"output_files":["prior.md"]}]}}"#,
            false,
        );
        assert!(!nested_read_model.saw_explicit_key);
        assert!(nested_read_model.paths.is_empty());

        let root_array_history = output_artifact_paths(
            r#"[{"output_files":["prior.md"]}]"#,
            false,
        );
        assert!(!root_array_history.saw_explicit_key);
        assert!(root_array_history.paths.is_empty());

        let root_array_declaration = output_artifact_paths(
            r#"[{"outputPath":"generated.md"}]"#,
            false,
        );
        assert!(root_array_declaration.saw_explicit_key);
        assert_eq!(root_array_declaration.paths, vec!["generated.md"]);

        let declared = output_artifact_paths(
            r#"{"result":{"output_files":["generated.md"]}}"#,
            false,
        );
        assert!(declared.saw_explicit_key);
        assert_eq!(declared.paths, vec!["generated.md"]);
    }

    #[test]
    fn explicit_artifact_declaration_after_json_limit_still_fails_closed() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call(
            "large-explicit-artifact",
            "mcp__vendor__worker",
            "{}",
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("report.md"), "# Generated\n").unwrap();

        let mut records = (0..600)
            .map(|index| serde_json::json!({ "index": index, "status": "completed" }))
            .collect::<Vec<_>>();
        records.push(serde_json::json!({ "outputPath": "report.md" }));
        let content = serde_json::Value::Array(records).to_string();
        let delivery = sink.emit_tool_result_with_images(
            "large-explicit-artifact",
            "mcp__vendor__worker",
            false,
            &content,
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(
                    data.output
                        .unwrap()
                        .contains("artifact contract JSON exceeds 512 nodes")
                );
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn existing_artifact_contract_keeps_large_json_limit_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        assert!(artifact_contract("exportPdf").is_some());
        sink.emit_tool_call("large-pdf-result", "exportPdf", "{}");
        let _running = rx.try_recv().unwrap();
        let content = serde_json::Value::Array(
            (0..600)
                .map(|index| serde_json::json!({ "index": index, "status": "completed" }))
                .collect(),
        )
        .to_string();
        let delivery = sink.emit_tool_result_with_images(
            "large-pdf-result",
            "exportPdf",
            false,
            &content,
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(
                    data.output
                        .unwrap()
                        .contains("artifact contract JSON exceeds 512 nodes")
                );
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn result_only_declared_path_without_pre_call_baseline_fails_closed() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("old-report.md"), "# Old report\n").unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let delivery = sink.emit_tool_result_with_images(
            "report-path",
            "mcp__reports__export_report",
            false,
            r#"{"outputPath":"old-report.md"}"#,
            &[],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("no pre-call baseline"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn unchanged_preexisting_artifact_and_missing_artifact_both_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("old-report.md"), "# Old report\n").unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.emit_tool_call(
            "old",
            "exportReport",
            r#"{"outputPath":"old-report.md"}"#,
        );
        let _running = rx.try_recv().unwrap();
        let old_delivery = sink.emit_tool_result_with_images(
            "old",
            "exportReport",
            false,
            r#"{"outputPath":"old-report.md"}"#,
            &[],
        );
        assert!(matches!(old_delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.output.unwrap().contains("unchanged from its pre-call fingerprint"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }

        sink.emit_tool_call(
            "missing",
            "exportReport",
            r#"{"output_path":"never-created.md"}"#,
        );
        let _running = rx.try_recv().unwrap();
        let missing_delivery = sink.emit_tool_result_with_images(
            "missing",
            "exportReport",
            false,
            r#"{"result":{"path":"never-created.md"}}"#,
            &[],
        );
        assert!(matches!(missing_delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.output.unwrap().contains("still missing"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_with_explicit_output_path_becomes_any_artifact_contract() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call(
            "custom",
            "mcp__vendor__worker",
            r#"{"artifacts_paths":["custom.bin"]}"#,
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("custom.bin"), b"generated bytes").unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "custom",
            "mcp__vendor__worker",
            false,
            r#"{"resultsFiles":["custom.bin"]}"#,
            &[],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Delivered { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(
                    data.artifacts[0]
                        .relative_path
                        .starts_with("nomifun-artifacts/artifact-")
                );
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }

        std::fs::write(workspace.path().join("untracked.bin"), b"old bytes").unwrap();
        let result_only = sink.emit_tool_result_with_images(
            "untracked",
            "mcp__vendor__worker",
            false,
            r#"{"artifactPath":"untracked.bin"}"#,
            &[],
        );
        assert!(matches!(result_only, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.output.unwrap().contains("no pre-call baseline"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn nested_source_paths_are_never_published_as_generated_outputs() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("source.md"), "# Source\n").unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call(
            "source-output",
            "exportReport",
            r#"{"input":{"path":"source.md"},"output_path":"report.md"}"#,
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("report.md"), "# Generated\n").unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "source-output",
            "exportReport",
            false,
            r#"{"source":{"path":"source.md"},"output":{"path":"report.md"}}"#,
            &[],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Delivered { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(
                    data.artifacts[0]
                        .relative_path
                        .starts_with("nomifun-artifacts/artifact-")
                );
                assert_eq!(
                    std::fs::read_to_string(&data.artifacts[0].path).unwrap(),
                    "# Generated\n"
                );
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn failed_images_and_context_screenshots_are_never_persisted() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.emit_tool_call("failed-image", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        let failed = sink.emit_tool_result_with_images(
            "failed-image",
            "image_gen",
            true,
            "provider failed",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        assert_eq!(failed, ToolMediaDelivery::Unmanaged);
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }

        sink.emit_tool_call("screenshot", "browserScreenshot", "{}");
        let _running = rx.try_recv().unwrap();
        let screenshot = sink.emit_tool_result_with_images(
            "screenshot",
            "browserScreenshot",
            false,
            "captured",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        assert_eq!(screenshot, ToolMediaDelivery::Unmanaged);
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn accepted_turn_requires_each_artifact_call_to_complete_with_its_own_receipt() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(32);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call("good", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "good",
                "image_gen",
                false,
                "done",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        let _completed = rx.try_recv().unwrap();
        assert!(sink.finish_artifact_delivery_turn().is_ok());

        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call("first-failed", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        let _ = sink.emit_tool_result_with_images(
            "first-failed",
            "image_gen",
            false,
            "claimed success",
            &[],
        );
        let _failed = rx.try_recv().unwrap();
        sink.emit_tool_call("later-good", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        let _ = sink.emit_tool_result_with_images(
            "later-good",
            "image_gen",
            false,
            "done",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        let _completed = rx.try_recv().unwrap();
        let error = sink.finish_artifact_delivery_turn().unwrap_err();
        assert!(error.contains("first-failed") || error.contains("image_gen"));

        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call("still-running", "exportReport", r#"{"output_path":"x.md"}"#);
        let _running = rx.try_recv().unwrap();
        sink.emit_stream_start("continuation");
        let _failed = rx.try_recv().unwrap();
        let _start = rx.try_recv().unwrap();
        assert!(sink.finish_artifact_delivery_turn().is_err());
    }

    #[test]
    fn routed_image_turn_requires_a_real_image_receipt_not_text_or_screenshot() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(32);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        let text_only = sink.finish_artifact_delivery_turn().unwrap_err();
        assert!(text_only.contains("no matching receipt was committed"));

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("screenshot-only", "browserScreenshot", "{}");
        let _running = rx.try_recv().unwrap();
        assert_eq!(
            sink.emit_tool_result_with_images(
                "screenshot-only",
                "browserScreenshot",
                false,
                "captured",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Unmanaged
        );
        let _completed_without_receipt = rx.try_recv().unwrap();
        let screenshot_only = sink.finish_artifact_delivery_turn().unwrap_err();
        assert!(screenshot_only.contains("no matching receipt was committed"));

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("native-image", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "native-image",
                "image_gen",
                false,
                "generated",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        let _completed_with_receipt = rx.try_recv().unwrap();
        assert!(sink.finish_artifact_delivery_turn().is_ok());
    }

    #[test]
    fn routed_image_turn_never_publishes_success_prose_ahead_of_projection_commit() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_text_delta("The image was generated successfully.", "msg-failed");
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(sink.finish_artifact_delivery_turn().is_err());
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("native-image", "image_gen", "{}");
        let _running = rx.try_recv().unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "native-image",
                "image_gen",
                false,
                "generated",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        let _completed_with_receipt = rx.try_recv().unwrap();
        sink.emit_text_delta("The image was generated successfully.", "msg-success");
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(sink.finish_artifact_delivery_turn().is_ok());
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn artifact_delivery_guard_persists_off_callback_and_publishes_only_after_cas() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_deferred_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("native-deferred", "image_gen", r#"{"prompt":"fox"}"#);
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
        let delivery = sink.emit_tool_result_with_images(
            "native-deferred",
            "image_gen",
            false,
            "generated",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Delivered { .. }));
        assert!(!workspace.path().join("nomifun-artifacts").exists());
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        sink.emit_text_delta("suppressed image completion prose", "native-deferred");
        assert_eq!(
            sink.artifact_delivery_turn.lock().unwrap().held_text,
            "suppressed image completion prose"
        );

        let cancellation = CancellationToken::new();
        assert!(matches!(
            sink.verify_artifact_delivery_turn_async(&cancellation).await,
            AsyncArtifactDeliveryOutcome::Verified(_)
        ));
        assert!(workspace.path().join("nomifun-artifacts").is_dir());
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        let verified = match sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            other => panic!("expected verified delivery, got {other:?}"),
        };
        sink.finish_verified_artifact_delivery_turn(verified)
            .unwrap();
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(Path::new(&data.artifacts[0].path).is_file());
            }
            other => panic!("expected committed image card, got {other:?}"),
        }
        assert!(sink.artifact_delivery_turn.lock().unwrap().held_text.is_empty());
    }

    #[tokio::test]
    async fn deferred_workspace_artifact_is_hidden_until_commit_and_abort_has_no_success_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        let tool = "mcp__reports__export_report";

        sink.begin_deferred_artifact_delivery_turn();
        sink.emit_tool_call(
            "report-abort",
            tool,
            r#"{"options":{"outputPath":"abort.md"}}"#,
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("abort.md"), "# Provisional\n").unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "report-abort",
                tool,
                false,
                r#"{"path":"abort.md"}"#,
                &[],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(
            !workspace.path().join("nomifun-artifacts").exists(),
            "pre-commit callback must not persist or publish a provisional artifact"
        );

        // Models this successful artifact tool being followed by an A2/provider
        // failure. The only visible terminal is Error and it carries no receipt.
        sink.abort_artifact_delivery_turn();
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("expected aborted artifact terminal, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());

        sink.begin_deferred_artifact_delivery_turn();
        sink.emit_tool_call(
            "report-commit",
            tool,
            r#"{"options":{"outputPath":"commit.md"}}"#,
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("commit.md"), "# Committed\n").unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "report-commit",
                tool,
                false,
                r#"{"path":"commit.md"}"#,
                &[],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let cancellation = CancellationToken::new();
        let verified = match sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            other => panic!("expected verified workspace artifact, got {other:?}"),
        };
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        sink.finish_verified_artifact_delivery_turn(verified)
            .unwrap();
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Completed);
                assert_eq!(data.artifacts.len(), 1);
                assert!(Path::new(&data.artifacts[0].path).is_file());
            }
            other => panic!("expected committed artifact terminal, got {other:?}"),
        }
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn recovery_regression_no_receiver_rolls_back_every_prepared_image_receipt() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.begin_deferred_artifact_delivery_turn_for("conversation-no-receiver", "wire-no-receiver")
            .unwrap();
        sink.require_image_artifact_for_turn().unwrap();

        for call_id in ["image-first", "image-second"] {
            sink.emit_tool_call(call_id, "image_gen", r#"{"prompt":"fox"}"#);
            assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
            assert!(matches!(
                sink.emit_tool_result_with_images(
                    call_id,
                    "image_gen",
                    false,
                    "generated",
                    &[ToolImage {
                        media_type: "image/png".into(),
                        data: PNG.into(),
                    }],
                ),
                ToolMediaDelivery::Delivered { .. }
            ));
        }

        let cancellation = CancellationToken::new();
        let verified = match sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            other => panic!("expected verified delivery, got {other:?}"),
        };
        let store = ArtifactStore::new(workspace.path());
        let records = store.recovery_records().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| matches!(
            record.state,
            crate::artifact_store::ArtifactRecoveryState::PersistedUnprepared
        )));
        let paths = records
            .iter()
            .map(|record| PathBuf::from(&record.receipt.path))
            .collect::<Vec<_>>();
        assert!(paths.iter().all(|path| path.is_file()));

        drop(rx);
        let error = sink
            .finish_verified_artifact_delivery_turn(verified)
            .unwrap_err();
        assert!(error.contains("no live relay receiver"));
        assert!(store.recovery_records().unwrap().is_empty());
        assert!(
            paths.iter().all(|path| !path.exists()),
            "a receipt that was never exposed to a relay has no durable owner and must be rolled back"
        );
    }

    #[tokio::test]
    async fn recovery_regression_receiver_loss_after_one_send_retains_only_the_exposed_journal() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.begin_deferred_artifact_delivery_turn_for("conversation-partial-send", "wire-partial-send")
            .unwrap();

        for call_id in ["image-first", "image-second"] {
            sink.emit_tool_call(call_id, "image_gen", r#"{"prompt":"fox"}"#);
            assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
            assert!(matches!(
                sink.emit_tool_result_with_images(
                    call_id,
                    "image_gen",
                    false,
                    "generated",
                    &[ToolImage {
                        media_type: "image/png".into(),
                        data: PNG.into(),
                    }],
                ),
                ToolMediaDelivery::Delivered { .. }
            ));
        }

        let cancellation = CancellationToken::new();
        let verified = match sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            other => panic!("expected verified delivery, got {other:?}"),
        };
        let store = ArtifactStore::new(workspace.path());
        let initial_records = store.recovery_records().unwrap();
        assert_eq!(initial_records.len(), 2);
        let paths = initial_records
            .iter()
            .map(|record| PathBuf::from(&record.receipt.path))
            .collect::<Vec<_>>();

        // Deterministic seam: the first broadcast succeeds, then its sole raw
        // receiver disappears before the second send. In production the relay
        // exposes only a receipt-free provisional card; Finish is the green
        // commit barrier, so DeliveryFailed remains rollbackable and honest.
        let mut receiver = Some(rx);
        let error = sink
            .finish_verified_artifact_delivery_turn_with(verified, |sent_index| {
                if sent_index == 0 {
                    drop(receiver.take());
                }
            })
            .unwrap_err();
        assert!(error.contains("no live relay receiver"), "{error}");

        let retained = store.recovery_records().unwrap();
        assert_eq!(retained.len(), 1);
        assert!(matches!(
            retained[0].state,
            crate::artifact_store::ArtifactRecoveryState::Prepared { .. }
        ));
        assert!(Path::new(&retained[0].receipt.path).is_file());
        assert_eq!(
            paths.iter().filter(|path| path.is_file()).count(),
            1,
            "only the possibly observed provisional event keeps recovery ownership"
        );
    }

    #[tokio::test]
    async fn cancelled_artifact_verification_preserves_nonzero_output_checkpoint_until_discard() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.begin_deferred_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("cancel-deferred", "image_gen", r#"{"prompt":"fox"}"#);
        let _running = rx.try_recv().unwrap();
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "cancel-deferred",
                "image_gen",
                false,
                "generated successfully",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        sink.emit_text_delta("retained prefix", "artifact-cancel");
        sink.emit_output_checkpoint("artifact-cancel");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta(" discarded draft", "artifact-cancel");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            sink.verify_artifact_delivery_turn_async(&cancellation).await,
            AsyncArtifactDeliveryOutcome::Cancelled
        ));
        assert!(!workspace.path().join("nomifun-artifacts").exists());

        {
            let turn = sink.artifact_delivery_turn.lock().unwrap();
            assert!(turn.active, "verification must leave rollback ownership live");
            assert_eq!(turn.held_text, "retained prefix discarded draft");
        }
        sink.emit_output_discarded("artifact-cancel", 1);
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                restart_attempt: 1
            })
        ));
        assert_eq!(
            sink.artifact_delivery_turn.lock().unwrap().held_text,
            "retained prefix"
        );

        // Mirrors the manager's ordered root restore: output is retracted while
        // its checkpoint is still valid, then the provisional artifact ledger
        // is retired with a non-success tool terminal.
        sink.abort_artifact_delivery_turn();
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(!data.output.unwrap_or_default().contains("successfully"));
            }
            other => panic!("expected cancellation terminal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_artifact_reverification_preserves_nonzero_output_checkpoint_until_discard() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.begin_deferred_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("failed-reverify", "image_gen", r#"{"prompt":"fox"}"#);
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "failed-reverify",
                "image_gen",
                false,
                "generated successfully",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        sink.emit_text_delta("retained prefix", "artifact-failure");
        sink.emit_output_checkpoint("artifact-failure");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta(" discarded draft", "artifact-failure");

        let cancellation = CancellationToken::new();
        assert!(matches!(
            sink.verify_artifact_delivery_turn_async(&cancellation).await,
            AsyncArtifactDeliveryOutcome::Verified(_)
        ));
        let artifact_path = {
            let turn = sink.artifact_delivery_turn.lock().unwrap();
            turn.calls
                .values()
                .find_map(|obligation| match &obligation.status {
                    ArtifactCallDeliveryStatus::CompletedVerified { artifacts, .. } => {
                        artifacts.first().map(|artifact| PathBuf::from(&artifact.path))
                    }
                    _ => None,
                })
                .expect("deferred persistence must install one verified receipt")
        };
        std::fs::remove_file(&artifact_path).unwrap();

        let failure = sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await;
        assert!(matches!(failure, AsyncArtifactDeliveryOutcome::Failed(_)));
        {
            let turn = sink.artifact_delivery_turn.lock().unwrap();
            assert!(turn.active, "verification failure must preserve rollback ownership");
            assert_eq!(turn.held_text, "retained prefix discarded draft");
        }

        sink.emit_output_discarded("artifact-failure", 2);
        assert!(matches!(
            rx.try_recv().unwrap(),
            AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                restart_attempt: 2
            })
        ));
        assert_eq!(
            sink.artifact_delivery_turn.lock().unwrap().held_text,
            "retained prefix"
        );
        sink.abort_artifact_delivery_turn();
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("expected failed artifact terminal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_final_artifact_publish_preserves_nonzero_output_checkpoint_until_discard() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.begin_deferred_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_tool_call("failed-publish", "image_gen", r#"{"prompt":"fox"}"#);
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
        assert!(matches!(
            sink.emit_tool_result_with_images(
                "failed-publish",
                "image_gen",
                false,
                "generated successfully",
                &[ToolImage {
                    media_type: "image/png".into(),
                    data: PNG.into(),
                }],
            ),
            ToolMediaDelivery::Delivered { .. }
        ));
        sink.emit_text_delta("retained prefix", "artifact-publish");
        sink.emit_output_checkpoint("artifact-publish");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Start(_)));
        sink.emit_text_delta(" discarded draft", "artifact-publish");

        let cancellation = CancellationToken::new();
        let verified = match sink
            .verify_artifact_delivery_turn_async(&cancellation)
            .await
        {
            AsyncArtifactDeliveryOutcome::Verified(verified) => verified,
            other => panic!("expected verified delivery, got {other:?}"),
        };
        drop(rx);
        let error = sink
            .finish_verified_artifact_delivery_turn(verified)
            .unwrap_err();
        assert!(error.contains("no live relay receiver"), "{error}");
        assert_eq!(
            sink.artifact_delivery_turn.lock().unwrap().held_text,
            "retained prefix discarded draft",
            "a failed final publish must leave the sealed generation retractable"
        );

        let mut rollback_rx = sink.event_tx.subscribe();
        sink.emit_output_discarded("artifact-publish", 3);
        assert!(matches!(
            rollback_rx.try_recv().unwrap(),
            AgentStreamEvent::OutputDiscarded(OutputDiscardedEventData {
                restart_attempt: 3
            })
        ));
        assert_eq!(
            sink.artifact_delivery_turn.lock().unwrap().held_text,
            "retained prefix"
        );
        sink.abort_artifact_delivery_turn();
        assert!(sink.artifact_delivery_turn.lock().unwrap().held_text.is_empty());
    }

    #[test]
    fn artifact_delivery_guard_suppresses_thinking_until_outcome_is_known() {
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx);
        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_thinking("The image was generated successfully.", "image-thinking");
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        sink.abort_artifact_delivery_turn();

        sink.begin_artifact_delivery_turn();
        sink.emit_thinking("analyzing", "ordinary-thinking");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Thinking(_)));
    }

    #[test]
    fn cancelling_routed_image_turn_discards_held_text_before_the_next_turn() {
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx);

        sink.begin_artifact_delivery_turn();
        sink.require_image_artifact_for_turn().unwrap();
        sink.emit_text_delta("stale generated-success claim", "cancelled");
        sink.cancel_active_tool_calls("cancelled by user");

        sink.begin_artifact_delivery_turn();
        sink.emit_text_delta("next turn", "next");
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Text(data) => assert_eq!(data.content, "next turn"),
            other => panic!("Expected only next-turn text after cancellation, got {other:?}"),
        }
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn image_tool_obligation_arms_text_gate_even_without_host_intent_route() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(8);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call("dynamic-image", "image_gen", r#"{"prompt":"fox"}"#);
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images(
            "dynamic-image",
            "image_gen",
            false,
            "generated successfully",
            &[],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        let _failed = rx.try_recv().unwrap();
        sink.emit_text_delta("The image was generated successfully.", "dynamic-text");
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(sink.finish_artifact_delivery_turn().is_err());
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn cancelled_or_failed_turn_rolls_back_store_owned_provisional_images() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let deliver = |sink: &BackendOutputSink, rx: &mut broadcast::Receiver<AgentStreamEvent>, id: &str| {
            sink.emit_tool_call(id, "image_gen", r#"{"prompt":"fox"}"#);
            let _running = rx.try_recv().unwrap();
            assert!(matches!(
                sink.emit_tool_result_with_images(
                    id,
                    "image_gen",
                    false,
                    "generated",
                    &[ToolImage {
                        media_type: "image/png".into(),
                        data: PNG.into(),
                    }],
                ),
                ToolMediaDelivery::Delivered { .. }
            ));
            match rx.try_recv().unwrap() {
                AgentStreamEvent::ToolCall(data) => PathBuf::from(&data.artifacts[0].path),
                other => panic!("Expected completed artifact, got {other:?}"),
            }
        };

        sink.begin_artifact_delivery_turn();
        let cancelled_path = deliver(&sink, &mut rx, "cancelled-image");
        assert!(cancelled_path.is_file());
        sink.cancel_active_tool_calls("cancelled");
        assert!(!cancelled_path.exists());

        sink.begin_artifact_delivery_turn();
        let failed_path = deliver(&sink, &mut rx, "good-before-failure");
        sink.emit_tool_call("failed-image", "image_gen", r#"{"prompt":"bad"}"#);
        let _running = rx.try_recv().unwrap();
        let _ = sink.emit_tool_result_with_images(
            "failed-image",
            "image_gen",
            false,
            "no bytes",
            &[],
        );
        let _failed = rx.try_recv().unwrap();
        assert!(sink.finish_artifact_delivery_turn().is_err());
        assert!(!failed_path.exists());
    }

    #[test]
    fn accepted_turn_reverifies_receipts_after_all_later_tools_finish() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        sink.begin_artifact_delivery_turn();
        // `flux` deliberately has no classifier-derived pre-call expectation;
        // the actual published receipt must still be enrolled in the ledger.
        sink.emit_tool_call("image-delete", "flux", "{}");
        let _running = rx.try_recv().unwrap();
        let delivery = sink.emit_tool_result_with_images(
            "image-delete",
            "flux",
            false,
            "done",
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Delivered { .. }));
        let receipt_path = match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => data.artifacts[0].path.clone(),
            other => panic!("Expected ToolCall, got {other:?}"),
        };

        // Simulate a later shell tool deleting the locator after the image
        // call completed but before the accepted user turn reached Finish.
        std::fs::remove_file(receipt_path).unwrap();
        let error = sink.finish_artifact_delivery_turn().unwrap_err();
        assert!(error.contains("failed final verification"));
    }

    #[test]
    fn invalid_declared_path_prevents_partial_inline_persistence() {
        const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call("image-mixed", "mcp__openai__image_gen", "{}");
        let _running = rx.try_recv().unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "image-mixed",
            "mcp__openai__image_gen",
            false,
            r#"{"artifactPath":"missing.png"}"#,
            &[ToolImage {
                media_type: "image/png".into(),
                data: PNG.into(),
            }],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        assert!(!workspace.path().join("nomifun-artifacts").exists());
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn invalid_inline_member_rolls_back_valid_path_snapshot_batch() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());
        sink.emit_tool_call(
            "mixed-invalid-inline",
            "exportReport",
            r#"{"outputPath":"report.md"}"#,
        );
        let _running = rx.try_recv().unwrap();
        std::fs::write(workspace.path().join("report.md"), "# Valid report\n").unwrap();

        let delivery = sink.emit_tool_result_with_images(
            "mixed-invalid-inline",
            "exportReport",
            false,
            r#"{"outputPath":"report.md"}"#,
            &[ToolImage {
                media_type: "image/png".into(),
                data: "bm90IGFuIGltYWdl".into(),
            }],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        assert!(!workspace.path().join("nomifun-artifacts").exists());
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn invalid_image_bytes_fail_delivery_without_creating_a_receipt() {
        let workspace = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_artifact_workspace(workspace.path());

        let delivery = sink.emit_tool_result_with_images(
            "image-1",
            "image_gen",
            false,
            "provider said success",
            &[ToolImage {
                media_type: "image/png".into(),
                data: "bm90IGFuIGltYWdl".into(),
            }],
        );

        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
                assert!(data.artifacts.is_empty());
                assert!(data.output.unwrap().contains("Artifact delivery failed"));
            }
            other => panic!("Expected ToolCall, got {other:?}"),
        }
        assert!(!workspace.path().join("nomifun-artifacts").exists());
    }

    #[test]
    fn no_panic_when_no_receivers() {
        let (tx, _) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx);
        sink.emit_text_delta("hello", "msg-1");
        sink.emit_thinking("thought", "msg-1");
        sink.emit_tool_call("call_read_1", "Read", "{}");
        sink.emit_tool_result("call_read_1", "Read", false, "ok");
        sink.emit_stream_start("msg-1");
        sink.emit_stream_end("msg-1", 1, 100, 50, 0, 0);
        sink.emit_error("err");
        sink.emit_info("info");
    }

    #[test]
    fn update_plan_result_emits_plan_event() {
        let (sink, mut rx) = make_sink();
        let content = r#"{"kind":"plan_update","explanation":null,"entries":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress"}]}"#;
        sink.emit_tool_call("call_1", "update_plan", r#"{"plan":[]}"#);
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
        sink.emit_tool_result("call_1", "update_plan", false, content);
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Plan(data) => {
                assert_eq!(data.session_id.as_deref(), Some("update_plan"));
                assert_eq!(data.source_call_id.as_deref(), Some("nomi-call_1"));
                assert_eq!(data.entries.len(), 2);
                assert_eq!(data.entries[1]["status"], "in_progress");
            }
            other => panic!("expected Plan, got {other:?}"),
        }
        sink.fail_active_tool_calls("a later lifecycle boundary");
        // The successful plan result must settle the source tool without
        // emitting a synthetic continuation recovery later.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn update_plan_with_warning_prefix_still_parses() {
        let (sink, mut rx) = make_sink();
        let content = "[note] 2 steps are in_progress; convention is exactly one. Plan rendered as submitted.\n{\"kind\":\"plan_update\",\"explanation\":null,\"entries\":[{\"content\":\"a\",\"status\":\"in_progress\"}]}";
        sink.emit_tool_result("call_1", "update_plan", false, content);
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Plan(data) => assert_eq!(data.entries.len(), 1),
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn update_plan_unparsable_falls_through_to_toolcall() {
        let (sink, mut rx) = make_sink();
        sink.emit_tool_result("call_1", "update_plan", false, "not json");
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));
    }

    #[test]
    fn update_plan_result_via_context_path_emits_plan_event() {
        // The engine's real tool-result path is
        // emit_tool_result_with_images_and_context; a successful update_plan
        // arriving there must project a Plan event exactly like the legacy
        // emit_tool_result path, or the frontend checklist never renders.
        let (sink, mut rx) = make_sink();
        let content = r#"{"kind":"plan_update","explanation":null,"entries":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress"}]}"#;
        let context = ToolCallExecutionContext {
            input: serde_json::json!({"plan":[{"step":"a","status":"completed"},{"step":"b","status":"in_progress"}]}),
            retry: ToolCallRetryContext {
                retry_group_id: "call_plan_1".to_owned(),
                attempt_no: 1,
                retry_of_call_id: None,
            },
        };
        sink.emit_tool_call_with_context(
            "call_plan_1",
            "update_plan",
            "update_plan",
            r#"{"plan":[{"step":"a","status":"completed"},{"step":"b","status":"in_progress"}]}"#,
            &context,
        );
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));

        sink.emit_tool_result_with_images_and_context(
            "call_plan_1",
            "update_plan",
            "update_plan",
            false,
            content,
            &[],
            &context,
        );
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Plan(data) => {
                assert_eq!(data.session_id.as_deref(), Some("update_plan"));
                assert_eq!(data.source_call_id.as_deref(), Some("nomi-call_plan_1"));
                assert_eq!(data.entries.len(), 2);
                assert_eq!(data.entries[1]["status"], "in_progress");
            }
            other => panic!("expected Plan, got {other:?}"),
        }

        // The short-lived result context stored by the context path must be
        // drained: a later result reusing the id may not inherit its args.
        // Checked before any lifecycle truncation, which would clear the
        // whole context map and make this assertion vacuous.
        sink.emit_tool_result("call_plan_1", "update_plan", true, "late result");
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.args, serde_json::Value::Null);
                assert!(data.retry.is_none());
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }

        // The plan result must settle the active call so no synthetic
        // recovery frame is emitted at a later lifecycle boundary.
        sink.fail_active_tool_calls("a later lifecycle boundary");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn update_plan_result_via_images_path_emits_plan_event() {
        let (sink, mut rx) = make_sink();
        let content = "[progress] Plan updated.\n{\"kind\":\"plan_update\",\"explanation\":null,\"entries\":[{\"content\":\"a\",\"status\":\"in_progress\"}]}";
        sink.emit_tool_result_with_images("call_plan_2", "update_plan", false, content, &[]);
        match rx.try_recv().unwrap() {
            AgentStreamEvent::Plan(data) => {
                assert_eq!(data.source_call_id.as_deref(), Some("nomi-call_plan_2"));
                assert_eq!(data.entries.len(), 1);
            }
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn update_plan_error_via_context_path_stays_a_tool_error() {
        let (sink, mut rx) = make_sink();
        let context = ToolCallExecutionContext {
            input: serde_json::json!({"plan":"nope"}),
            retry: ToolCallRetryContext {
                retry_group_id: "call_plan_3".to_owned(),
                attempt_no: 1,
                retry_of_call_id: None,
            },
        };
        sink.emit_tool_result_with_images_and_context(
            "call_plan_3",
            "update_plan",
            "update_plan",
            true,
            "update_plan: invalid arguments",
            &[],
            &context,
        );
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn update_plan_with_inline_images_keeps_fail_closed_artifact_accounting() {
        // A plan declaration never carries media. If a result named
        // update_plan arrives with inline images, it must not short-circuit
        // into a Plan event: the funnel's fail-closed artifact path owns it.
        let (sink, mut rx) = make_sink();
        let content = r#"{"kind":"plan_update","explanation":null,"entries":[{"content":"a","status":"in_progress"}]}"#;
        let delivery = sink.emit_tool_result_with_images(
            "call_plan_img",
            "update_plan",
            false,
            content,
            &[ToolImage {
                media_type: "image/png".into(),
                data: "bytes".into(),
            }],
        );
        assert!(matches!(delivery, ToolMediaDelivery::Failed { .. }));
        match rx.try_recv().unwrap() {
            AgentStreamEvent::ToolCall(data) => {
                assert_eq!(data.status, ToolCallStatus::Error);
            }
            other => panic!("expected fail-closed ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn update_plan_settles_a_stray_artifact_obligation_instead_of_leaking_running() {
        // A stray artifact-path key in update_plan args registers an
        // obligation at call time. The plan projection must settle it as
        // failed (a plan never delivers artifacts) rather than leave it
        // Running, which would report the misleading "ended without a
        // verified artifact receipt" at turn end.
        let (sink, mut rx) = make_sink();
        sink.begin_artifact_delivery_turn();
        sink.emit_tool_call(
            "call_plan_stray",
            "update_plan",
            r#"{"plan":[{"step":"a","status":"in_progress"}],"output_path":"foo.png"}"#,
        );
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::ToolCall(_)));

        let content = r#"{"kind":"plan_update","explanation":null,"entries":[{"content":"a","status":"in_progress"}]}"#;
        sink.emit_tool_result("call_plan_stray", "update_plan", false, content);
        assert!(matches!(rx.try_recv().unwrap(), AgentStreamEvent::Plan(_)));

        let sealed = sink.finish_artifact_delivery_turn();
        let error = sealed.expect_err("a stray artifact declaration must fail closed");
        assert!(
            error.contains("does not deliver artifacts"),
            "expected the explicit plan-projection reason, got: {error}"
        );
        assert!(!error.contains("ended without a verified artifact receipt"));
    }

    // -- citation reflow ------------------------------------------------------

    #[test]
    fn citation_reflow_bumps_cited_file_on_stream_end() {
        use nomi_memory::store::{read_memory, write_memory};
        use nomi_memory::types::{MemoryEntry, MemoryType};

        let tmp = tempfile::tempdir().unwrap();
        let entry = MemoryEntry::build("role", "user role", MemoryType::User, "senior dev");
        let path = write_memory(tmp.path(), &entry).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap().to_owned();

        let (tx, _rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx).with_distill_dir(Some(tmp.path().to_path_buf()));

        sink.emit_stream_start("m1");
        sink.emit_text_delta("Here is the answer.\n\n<nomi-mem-citation>\n", "m1");
        sink.emit_text_delta(&format!("{filename}|note=[used role]\n"), "m1");
        sink.emit_text_delta("</nomi-mem-citation>", "m1");
        sink.emit_stream_end("m1", 1, 10, 5, 0, 0);

        let read_back = read_memory(&path).unwrap();
        assert_eq!(read_back.frontmatter.usage_count, Some(1));
        assert!(read_back.frontmatter.last_used.is_some());
    }

    #[test]
    fn no_distill_dir_means_no_reflow_and_no_accumulation() {
        // Without a distill dir, the sink must not touch any file (and the
        // text buffer is never used).
        let (tx, _rx) = broadcast::channel(16);
        let sink = BackendOutputSink::new(tx); // distill_dir = None
        sink.emit_stream_start("m1");
        sink.emit_text_delta("<nomi-mem-citation>\nuser_role.md|note=[x]\n</nomi-mem-citation>", "m1");
        sink.emit_stream_end("m1", 1, 10, 5, 0, 0);
        // Nothing to assert beyond "did not panic / did not write" — the
        // turn_text buffer stays empty because distill_dir is None.
        assert!(sink.turn_text.lock().unwrap().is_empty());
    }
}
