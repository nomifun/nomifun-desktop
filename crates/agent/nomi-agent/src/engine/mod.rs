use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomi_config::compact::CompactConfig;
use nomi_config::config::Config;
use nomi_config::hooks::HookEngine;
use nomi_protocol::events::ToolCategory;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{
    ContentBlock, Message, Role, StopReason, TokenUsage, clear_provider_round_ids,
};
use nomi_types::skill_types::{ContextModifier, PlanModeTransition, effort_to_string};
use serde_json::Value;
use tracing::Instrument;
use sha2::{Digest, Sha256};

use crate::cache_diagnostics::{CacheBreakDetector, CacheDiagnostic, CacheStats};
use crate::compact::state::CompactState;
use crate::compact::{auto, emergency, estimate, micro};
use crate::tool_execution::{
    ProviderToolAuthority, SKIPPED_AFTER_PRIOR_ERROR, execute_tool_calls_scoped,
    execute_tool_calls_with_protocol,
};
use crate::output::{OutputSink, ToolCallExecutionContext, ToolCallRetryContext};
use crate::plan::prompt as plan_prompt;
use crate::plan::state::PlanState;
use crate::round;
use crate::session::{AcceptedTurnRoot, EditableTurnCheckpoint, Session, SessionManager};

mod runtime_authority;
use runtime_authority::AcceptedTurnRuntimeAuthority;

/// Trusted accepted-turn scope supplied by a host that decorates provider
/// input or performs same-turn race-tail re-runs.
///
/// `requirement` is the original user-authored content, before RAG/knowledge
/// prefixes. `prior_durable_effect_targets` carries machine evidence from an
/// earlier engine pass of the same accepted turn so a late steering re-run
/// cannot erase work that is already visible in the shared workspace.
const MAX_COMPLETION_EVIDENCE_TARGETS: usize = 32;
const MAX_COMPLETION_EVIDENCE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_COMPLETION_EVIDENCE_TOTAL_HASH_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionEvidenceMode {
    /// Ordinary engines can prove the final parent-workspace state directly.
    LocalFingerprint,
    /// SSH tools mutate a remote namespace. Only correlated successful atomic
    /// file-tool receipts are accepted; local paths must never stand in for
    /// the remote host.
    RemoteExactReceipts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionTargetFingerprint {
    Absent,
    Present {
        size_bytes: u64,
        modified_nanos: Option<u128>,
        /// Large files and targets beyond the bounded accepted-turn hashing
        /// budget retain a metadata fingerprint instead of becoming a silent
        /// adjudication bypass.
        sha256: Option<[u8; 32]>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionFingerprintProbe {
    State(CompletionTargetFingerprint),
    /// The lexical target resolves through a symlink, outside the workspace,
    /// or to a non-regular object. It is outside the local hard-verdict domain.
    Unsafe,
    /// A transient I/O failure prevented a point-in-time final observation.
    /// Unlike `Unsafe`, this does not silently exempt a positive claim.
    Unavailable,
    /// Accepted-turn resource bounds prevented a content fingerprint. A clear
    /// completion claim remains failed unless a correlated terminal exact
    /// receipt supplies the narrow final-presence fallback.
    ResourceLimited,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionEvidenceContext {
    pub requirement: Vec<ContentBlock>,
    pub prior_durable_effect_targets: Vec<String>,
    /// One corrective completion-evidence pass per accepted user turn, even
    /// when the host performs additional same-turn race-tail executions.
    pub corrective_nudge_used: bool,
    target_baselines: HashMap<String, CompletionTargetFingerprint>,
    baseline_unavailable_targets: HashSet<String>,
    unsafe_targets: HashSet<String>,
    known_requirement_targets: HashSet<String>,
    /// Targets whose pre-effect content could not be fingerprinted (late
    /// steering, resource cap, or probe failure). They remain in the hard gate;
    /// only a still-terminal exact receipt may provide the narrow fallback.
    unfingerprinted_targets: HashSet<String>,
    baseline_hashed_bytes: u64,
    /// A successful state-changing call observed anywhere in this accepted
    /// turn. A coincidental external filesystem change cannot by itself back a
    /// model completion claim.
    successful_mutation_observed: bool,
    /// Ordered terminal proof for SSH-bound sessions. A later successful
    /// opaque remote mutation invalidates earlier path receipts because the
    /// host cannot probe the remote final state.
    terminal_exact_receipts: Vec<String>,
    turn_root: Option<AcceptedTurnRoot>,
    /// Hidden in-memory authority as it stood before this accepted turn's first
    /// provider await. Captured exactly once, in the same guarded step as
    /// `turn_root`, so a host race-tail pass restores the state that preceded
    /// pass A rather than whatever pass A left behind.
    turn_authority: Option<AcceptedTurnRuntimeAuthority>,
    /// Shared with the desktop termination guard. It becomes true only after
    /// an accepted-turn recovery root is durably present and stays true through
    /// the host-terminal phase. An abnormal unwind can then publish the typed
    /// session-consistency error that makes Conversation perform an exact
    /// persisted rewind instead of treating the provisional transcript as a
    /// normal cancellation.
    host_recovery_required: Arc<AtomicBool>,
}

impl CompletionEvidenceContext {
    pub fn new(requirement: Vec<ContentBlock>) -> Self {
        Self {
            requirement,
            ..Self::default()
        }
    }

    pub fn with_host_recovery_signal(
        requirement: Vec<ContentBlock>,
        host_recovery_required: Arc<AtomicBool>,
    ) -> Self {
        Self {
            requirement,
            host_recovery_required,
            ..Self::default()
        }
    }

    pub fn host_recovery_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.host_recovery_required)
    }

    /// Whether the engine has crossed the accepted-turn transaction boundary.
    /// A biased host cancellation may win before the engine future is first
    /// polled; in that case there is no marker or transcript suffix to restore.
    pub fn turn_root_captured(&self) -> bool {
        self.turn_root.is_some()
    }

    fn capture_turn_root(
        &mut self,
        source_message_id: &str,
        messages: &[Message],
        editable_turn: &Option<EditableTurnCheckpoint>,
        host_context: &BTreeMap<String, String>,
        activated_deferred_tools: Vec<String>,
        runtime_authority: AcceptedTurnRuntimeAuthority,
    ) -> Result<(), AgentError> {
        if let Some(root) = &self.turn_root {
            if root.source_message_id != source_message_id {
                return Err(AgentError::ApiError(
                    "completion evidence context was reused for a different accepted turn"
                        .to_owned(),
                ));
            }
            return Ok(());
        }
        self.turn_root = Some(AcceptedTurnRoot {
            source_message_id: source_message_id.to_owned(),
            messages: messages.to_vec(),
            editable_turn: editable_turn.clone(),
            host_context: host_context.clone(),
            activated_deferred_tools,
        });
        // Same guarded step as the transcript root, so the two can never
        // disagree about which pass of an accepted turn is the rollback floor.
        // A host race-tail pass returns early above and keeps pass A's snapshot.
        self.turn_authority = Some(runtime_authority);
        Ok(())
    }

    async fn ensure_target_baselines(
        &mut self,
        workspace_root: &std::path::Path,
        mode: CompletionEvidenceMode,
    ) {
        let mut pending = Vec::new();
        for target in required_state_change_targets(&self.requirement, workspace_root, mode) {
            if self.known_requirement_targets.insert(target.clone())
                && self.successful_mutation_observed
            {
                self.unfingerprinted_targets.insert(target.clone());
            }
            if mode == CompletionEvidenceMode::RemoteExactReceipts
                || self.unfingerprinted_targets.contains(&target)
            {
                continue;
            }
            if self.target_baselines.contains_key(&target)
                || self.baseline_unavailable_targets.contains(&target)
                || self.unsafe_targets.contains(&target)
            {
                continue;
            }
            if self.target_baselines.len()
                + self.baseline_unavailable_targets.len()
                + self.unsafe_targets.len()
                >= MAX_COMPLETION_EVIDENCE_TARGETS
            {
                self.unfingerprinted_targets.insert(target);
                continue;
            }
            pending.push(target);
        }
        if pending.is_empty() {
            return;
        }
        let root = workspace_root.to_path_buf();
        let remaining = MAX_COMPLETION_EVIDENCE_TOTAL_HASH_BYTES
            .saturating_sub(self.baseline_hashed_bytes);
        let pending_for_failure = pending.clone();
        let probes = match tokio::task::spawn_blocking(move || {
            completion_target_fingerprints(&root, pending, remaining)
        })
        .await
        {
            Ok(probes) => probes,
            Err(_) => {
                self.unfingerprinted_targets
                    .extend(pending_for_failure);
                return;
            }
        };
        for (target, probe, bytes_hashed) in probes {
            self.baseline_hashed_bytes =
                self.baseline_hashed_bytes.saturating_add(bytes_hashed);
            match probe {
                CompletionFingerprintProbe::State(fingerprint) => {
                    self.target_baselines.insert(target, fingerprint);
                }
                CompletionFingerprintProbe::Unsafe => {
                    self.unsafe_targets.insert(target);
                }
                CompletionFingerprintProbe::Unavailable => {
                    self.baseline_unavailable_targets.insert(target);
                }
                CompletionFingerprintProbe::ResourceLimited => {
                    self.unfingerprinted_targets.insert(target);
                }
            }
        }
    }

    async fn supported_targets(
        &self,
        answer: &str,
        workspace_root: &std::path::Path,
        mode: CompletionEvidenceMode,
        exact_receipts: &[String],
    ) -> Vec<String> {
        let required = required_state_change_targets(&self.requirement, workspace_root, mode);
        let claimed = effective_claimed_state_change_targets(
            answer,
            &required,
            workspace_root,
            mode,
        );
        let candidates = required
            .into_iter()
            .filter(|target| {
                claimed.contains(target)
                    && (!self.unfingerprinted_targets.contains(target)
                        || exact_receipts.contains(target))
            })
            .collect::<Vec<_>>();
        if mode == CompletionEvidenceMode::RemoteExactReceipts {
            return candidates
                .iter()
                .filter(|target| exact_receipts.contains(target))
                .cloned()
                .collect::<Vec<_>>();
        }

        let probe_targets = candidates
            .iter()
            .filter(|target| !self.unsafe_targets.contains(*target))
            .cloned()
            .collect::<Vec<_>>();
        let root = workspace_root.to_path_buf();
        // Each terminal evaluation receives its own bounded final budget. A
        // corrective pass must not turn a previously adjudicable target into a
        // bypass merely because the first evaluation consumed the budget.
        let probes = tokio::task::spawn_blocking(move || {
            completion_target_fingerprints(
                &root,
                probe_targets,
                MAX_COMPLETION_EVIDENCE_TOTAL_HASH_BYTES,
            )
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(target, probe, _)| (target, probe))
        .collect::<HashMap<_, _>>();

        let mut supported = Vec::new();
        for target in candidates {
            if self.unsafe_targets.contains(&target) {
                continue;
            }
            let exact = exact_receipts.contains(&target);
            let Some(final_probe) = probes.get(&target) else {
                continue;
            };
            let final_state = match final_probe {
                CompletionFingerprintProbe::State(state) => state,
                CompletionFingerprintProbe::Unsafe => continue,
                CompletionFingerprintProbe::Unavailable => continue,
                CompletionFingerprintProbe::ResourceLimited
                    if exact
                        && matches!(
                            self.target_baselines.get(&target),
                            Some(CompletionTargetFingerprint::Absent)
                        ) =>
                {
                    supported.push(target);
                    continue;
                }
                CompletionFingerprintProbe::ResourceLimited => continue,
            };
            let proves_change = match (self.target_baselines.get(&target), final_state) {
                (
                    Some(CompletionTargetFingerprint::Absent),
                    CompletionTargetFingerprint::Present { .. },
                ) => true,
                (
                    Some(CompletionTargetFingerprint::Present {
                        size_bytes: before_size,
                        modified_nanos: before_modified,
                        sha256: before_hash,
                    }),
                    CompletionTargetFingerprint::Present {
                        size_bytes: after_size,
                        modified_nanos: after_modified,
                        sha256: after_hash,
                    },
                ) => {
                    if let (Some(before_hash), Some(after_hash)) = (before_hash, after_hash) {
                        before_size != after_size || before_hash != after_hash
                    } else {
                        exact
                            && (before_size != after_size
                                || before_modified != after_modified)
                    }
                }
                // No final regular file is never durable creation/update proof.
                (_, CompletionTargetFingerprint::Absent) => false,
                // Without a pre-effect state, even two exact writes can restore
                // the original bytes. Path presence is not a delta proof.
                (None, CompletionTargetFingerprint::Present { .. }) => false,
            };
            if proves_change && (self.successful_mutation_observed || exact) {
                supported.push(target);
            }
        }
        supported
    }
}

fn apply_terminal_effect_evidence(
    context: &mut CompletionEvidenceContext,
    ledger: &mut round::RoundLedger,
    invocation_may_mutate: bool,
    is_error: bool,
    exact_targets: Vec<String>,
    deleted_targets: Vec<String>,
) {
    if invocation_may_mutate && (is_error || exact_targets.is_empty()) {
        // Failed state-changing calls are not known to be side-effect free (a
        // shell can delete then exit 1). A successful opaque call is equally
        // capable of reverting an earlier exact write. In either case, all
        // monotonic path receipts cease to be terminal proof, including the
        // host-race-tail seed.
        context.terminal_exact_receipts.clear();
        context.prior_durable_effect_targets.clear();
        ledger.durable_effect_targets.clear();
        return;
    }
    if is_error {
        return;
    }
    for deleted in deleted_targets {
        context
            .terminal_exact_receipts
            .retain(|target| target != &deleted);
        context
            .prior_durable_effect_targets
            .retain(|target| target != &deleted);
        ledger
            .durable_effect_targets
            .retain(|target| target != &deleted);
    }
    ledger.record_durable_effect_targets(exact_targets.iter().cloned());
    for target in exact_targets {
        if !context.prior_durable_effect_targets.contains(&target) {
            context.prior_durable_effect_targets.push(target.clone());
        }
        if !context.terminal_exact_receipts.contains(&target) {
            context.terminal_exact_receipts.push(target);
        }
    }
}

/// Decide how a prompt-cache-break diagnostic should surface to the user.
/// Returns the info-level message to emit, or `None` to stay silent.
///
/// All diagnostics — including a `FullMiss` — are gated behind the opt-in
/// `cache_diagnostics` flag and are INFO, never errors. A full miss is not a
/// failure: the prompt cache merely lapsed, most often a benign server-side TTL
/// expiry during the idle gap between turns (e.g. between AutoWork tasks).
/// Emitting it as an error previously made the AutoWork runner treat a
/// perfectly good turn as failed (re-pend, and eventually a tag pause).
fn cache_diagnostic_message(diag: &CacheDiagnostic, diagnostics_enabled: bool) -> Option<String> {
    if !diagnostics_enabled {
        return None;
    }
    Some(match diag {
        CacheDiagnostic::FullMiss { cause } => format!("Cache full miss: {cause:?}"),
        CacheDiagnostic::PartialMiss { hit_rate, cause } => {
            format!("Cache: {:.0}% hit rate (cause: {cause:?})", hit_rate * 100.0)
        }
        CacheDiagnostic::Healthy { hit_rate } => {
            format!("Cache: {:.0}% hit rate", hit_rate * 100.0)
        }
    })
}

/// Maximum characters kept from a single tool-result body in the distillation
/// transcript. Tool outputs can be huge (file dumps, search results); the
/// distiller only needs a hint of what happened, not the full payload.
const TRANSCRIPT_TOOL_RESULT_MAX: usize = 600;

/// If the provider stream stays alive but silent for this long, surface a
/// lightweight progress event so the UI does not look frozen while the model is
/// generating a large tool-call argument.
const STREAM_IDLE_ACTIVITY_AFTER: Duration = Duration::from_millis(1_200);

/// Hard limit for complete structured tool calls emitted by one provider
/// turn. The engine consumes the entire provider turn before dispatching any
/// call, so rejecting the first call beyond this bound keeps the oversized
const MAX_PROVIDER_TURN_TOOL_CALLS: usize = 128;
const MAX_PROVIDER_ROUND_ID_BYTES: usize = 512;

#[derive(Debug, Clone)]
struct InvalidArgumentRetryCandidate {
    name: String,
    call_id: String,
    retry_group_id: String,
    attempt_no: u32,
    same_name_call_count: usize,
}

/// Tracks only the immediately preceding provider round's unambiguous,
/// pre-dispatch schema failures. It deliberately does not infer retries from
/// timing, output text, or argument similarity.
#[derive(Debug, Default)]
struct ToolRetryTracker {
    pending: Vec<InvalidArgumentRetryCandidate>,
    /// Provider tool-use ids are event identities, not merely per-round
    /// labels. Reusing one anywhere in the same root user turn would make a
    /// later event overwrite the earlier lifecycle and can create a self-retry.
    seen_call_ids: HashSet<String>,
}

impl ToolRetryTracker {
    fn assign(
        &mut self,
        calls: &[ContentBlock],
    ) -> Result<HashMap<String, ToolCallExecutionContext>, String> {
        // Validate the whole round before mutating either set. The stream path
        // already rejects duplicates inside one provider round; keeping this
        // invariant here makes the durable cross-round identity boundary
        // independently testable and fail-closed.
        let mut round_call_ids = HashSet::new();
        for call in calls {
            if let ContentBlock::ToolUse { id, .. } = call
                && (!round_call_ids.insert(id.clone()) || self.seen_call_ids.contains(id))
            {
                return Err(id.clone());
            }
        }
        self.seen_call_ids.extend(round_call_ids);

        let previous = std::mem::take(&mut self.pending);
        let mut previous_counts = HashMap::<&str, usize>::new();
        for candidate in &previous {
            *previous_counts.entry(candidate.name.as_str()).or_default() += 1;
        }
        let mut current_counts = HashMap::<&str, usize>::new();
        for call in calls {
            if let ContentBlock::ToolUse { name, .. } = call {
                *current_counts.entry(name.as_str()).or_default() += 1;
            }
        }

        Ok(calls
            .iter()
            .filter_map(|call| {
                let ContentBlock::ToolUse {
                    id, name, input, ..
                } = call
                else {
                    return None;
                };
                let inherited = (current_counts.get(name.as_str()) == Some(&1)
                    && previous_counts.get(name.as_str()) == Some(&1))
                    .then(|| previous.iter().find(|candidate| candidate.name == *name))
                    .flatten()
                    .filter(|candidate| candidate.same_name_call_count == 1);
                let retry = match inherited {
                    Some(candidate) => ToolCallRetryContext {
                        retry_group_id: candidate.retry_group_id.clone(),
                        attempt_no: candidate.attempt_no.saturating_add(1),
                        retry_of_call_id: Some(candidate.call_id.clone()),
                    },
                    None => ToolCallRetryContext {
                        retry_group_id: id.clone(),
                        attempt_no: 1,
                        retry_of_call_id: None,
                    },
                };
                Some((
                    id.clone(),
                    ToolCallExecutionContext {
                        input: input.clone(),
                        retry,
                    },
                ))
            })
            .collect())
    }

    fn observe_invalid_arguments(
        &mut self,
        calls: &[ContentBlock],
        contexts: &HashMap<String, ToolCallExecutionContext>,
        invalid_call_ids: &HashSet<String>,
    ) {
        let mut call_counts = HashMap::<&str, usize>::new();
        for call in calls {
            if let ContentBlock::ToolUse { name, .. } = call {
                *call_counts.entry(name.as_str()).or_default() += 1;
            }
        }
        self.pending = calls
            .iter()
            .filter_map(|call| {
                let ContentBlock::ToolUse { id, name, .. } = call else {
                    return None;
                };
                if !invalid_call_ids.contains(id) {
                    return None;
                }
                let context = contexts.get(id)?;
                Some(InvalidArgumentRetryCandidate {
                    name: name.clone(),
                    call_id: id.clone(),
                    retry_group_id: context.retry.retry_group_id.clone(),
                    attempt_no: context.retry.attempt_no,
                    same_name_call_count: call_counts
                        .get(name.as_str())
                        .copied()
                        .unwrap_or_default(),
                })
            })
            .collect();
    }

    fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Confirm which locally schema-invalid calls actually reached the
/// pre-dispatch validation gate. The executor returns results in call order and
/// installs an error barrier: after the first real error, later non-concurrent
/// calls are structured skipped results. Schema-invalid calls are never
/// concurrency-safe, so a schema-invalid error is genuine only when no earlier
/// result closed that barrier.
///
/// This intentionally does not inspect human-readable error text. The evidence
/// is the intersection of the exact preflight set and the structured outcome
/// ordering/error bits.
fn confirmed_predispatch_schema_invalid_call_ids(
    preflight_invalid_call_ids: &HashSet<String>,
    results: &[ContentBlock],
) -> HashSet<String> {
    let mut error_barrier_closed = false;
    let mut confirmed = HashSet::new();

    for result in results {
        let ContentBlock::ToolResult {
            tool_use_id,
            is_error,
            ..
        } = result
        else {
            continue;
        };
        if *is_error
            && !error_barrier_closed
            && preflight_invalid_call_ids.contains(tool_use_id)
        {
            confirmed.insert(tool_use_id.clone());
        }
        error_barrier_closed |= *is_error;
    }

    confirmed
}

#[cfg(test)]
mod tool_retry_tracker_tests;

const SYSTEM_RESOURCE_CONTEXT_HEADER: &str =
    "## System resource notifications (trusted host state)";
const REQUEST_SCOPED_TOOL_AUTHORITY_HEADER: &str = "## Request-scoped tool authority";
const REQUEST_SCOPED_TOOL_AUTHORITY_RULE: &str = "The host's `tools` field on this exact provider request is the complete and authoritative tool surface. Call only a tool declared there. If another system instruction, prior message, or retrieved context mentions a tool that is not declared on this request, that tool is unavailable now and MUST NOT be called; continue using only the declared tools, or explain the capability limitation.";

/// Add host-generated resource state to the provider's top-level system
/// context. These notices are deliberately ephemeral and never become
/// conversation messages, so they cannot be mistaken for user input or leak
/// into the durable transcript.
fn append_system_resource_context(mut system: String, notices: Vec<String>) -> String {
    let notices = notices
        .into_iter()
        .filter_map(|notice| {
            let notice = notice.trim();
            (!notice.is_empty()).then(|| notice.to_owned())
        })
        .collect::<Vec<_>>();
    if notices.is_empty() {
        return system;
    }
    if !system.is_empty() {
        system.push_str("\n\n");
    }
    system.push_str(SYSTEM_RESOURCE_CONTEXT_HEADER);
    system.push_str(
        "\nThe following entries are authoritative runtime state from the host, not user messages:\n",
    );
    for notice in notices {
        system.push_str("- ");
        system.push_str(&notice);
        system.push('\n');
    }
    system
}

/// Durable transcript marker used after the current turn has finished seeing
/// an attached image. Keeping the text marker preserves conversational meaning
/// without re-sending a large base64 payload on every later provider request.
const USER_IMAGE_HISTORY_PLACEHOLDER: &str = "[Image attachment omitted after processing.]";

/// Render the conversation history as a role-tagged plain-text transcript for
/// post-session memory distillation.
///
/// Rules (mirroring codex `serialize_filtered_rollout_response_items`):
/// - User / assistant text blocks are kept with a `[role]` prefix.
/// - Tool calls become `[tool <name>] <compact args>` (args truncated).
/// - Tool results become `[tool result(<err?>)] <body>` (body truncated to
///   `TRANSCRIPT_TOOL_RESULT_MAX`).
/// - Thinking blocks are dropped entirely.
fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
            Role::Tool => "tool",
        };
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => {
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    out.push_str(&format!("[{role}] {text}\n"));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    let args = input.to_string();
                    let args = truncate_chars(&args, TRANSCRIPT_TOOL_RESULT_MAX);
                    out.push_str(&format!("[tool {name}] {args}\n"));
                }
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let body = truncate_chars(content.trim(), TRANSCRIPT_TOOL_RESULT_MAX);
                    let tag = if *is_error { " error" } else { "" };
                    out.push_str(&format!("[tool result{tag}] {body}\n"));
                }
                // Drop thinking: it's reasoning scratch, not durable signal.
                ContentBlock::Thinking { .. } => {}
                // Images have no textual representation in the transcript.
                ContentBlock::Image { .. } => {}
            }
        }
    }
    out
}

/// Truncate `s` to at most `max` characters (char-boundary safe), appending an
/// ellipsis marker when truncation occurred.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}…(truncated)")
}

/// Hard safety-net turn cap applied when the session does not configure
/// `max_turns` (the production default is `None`). Without this, a model stuck
/// in a tool-call loop runs forever, burning tokens and appearing "stuck" to
/// the user. A user-configured `Some(n)` is always respected as-is — this only
/// bounds the otherwise-unbounded `None` case. Mirrors Claude Code's ~200-turn
/// guard. See docs/superpowers/specs/2026-06-21-nomi-agent-overhaul-design.md §5 F0.3.
const DEFAULT_SAFETY_MAX_TURNS: usize = 200;

/// Strictest image-count limit among the supported message providers. Amazon
/// Bedrock Claude Anthropic Messages rejects a request containing more than 20 images.
const MAX_PROVIDER_REQUEST_IMAGES: usize = 20;

/// Bound the cumulative base64 image data replayed with one provider request.
/// This matches the padded base64 size of the existing 5 MiB decoded-image
/// limit used by Read and MCP tools. A count-only limit can still create a
/// multi-megabyte request after a Computer screenshot loop.
const MAX_SINGLE_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_PROVIDER_REQUEST_IMAGE_DATA_BYTES: usize = MAX_SINGLE_IMAGE_BYTES.div_ceil(3) * 4;

#[derive(Debug, Default, PartialEq, Eq)]
struct ToolEfficiencyStats {
    model_turn_attempts: usize,
    model_turns_with_tools: usize,
    total_tool_calls: usize,
    max_calls_in_model_turn: usize,
    exec_command_script_calls: usize,
    batch_read_files_requested: usize,
    /// Model turns whose only tool call was a single-file `Read`. Distinguishes
    /// "read one file because that is all that was needed" (fine, low count)
    /// from walking a codebase one file per provider round trip (expensive).
    lone_single_file_reads: usize,
    error_results: usize,
    skipped_after_prior_error: usize,
}

impl ToolEfficiencyStats {
    fn observe_model_turn_attempt(&mut self) {
        self.model_turn_attempts = self.model_turn_attempts.saturating_add(1);
    }

    fn observe_calls(&mut self, _registry: &ToolRegistry, blocks: &[ContentBlock]) {
        let calls = blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { name, input, .. } => Some((name.as_str(), input)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if calls.is_empty() {
            return;
        }

        self.model_turns_with_tools = self.model_turns_with_tools.saturating_add(1);
        self.total_tool_calls = self.total_tool_calls.saturating_add(calls.len());
        self.max_calls_in_model_turn = self.max_calls_in_model_turn.max(calls.len());
        if let [(name, input)] = calls.as_slice()
            && *name == "Read"
            && input.get("file_paths").is_none()
        {
            self.lone_single_file_reads = self.lone_single_file_reads.saturating_add(1);
        }
        for (name, input) in &calls {
            if *name == "exec_command" && input.get("script").is_some() {
                self.exec_command_script_calls =
                    self.exec_command_script_calls.saturating_add(1);
            }
            if *name == "Read"
                && let Some(paths) = input.get("file_paths").and_then(Value::as_array)
            {
                self.batch_read_files_requested = self
                    .batch_read_files_requested
                    .saturating_add(paths.len());
            }
        }
    }

    fn terminal_dimensions(
        &self,
        result: &Result<AgentResult, AgentError>,
    ) -> (&'static str, &'static str, &'static str, usize) {
        match result {
            Ok(result) => {
                let stop_reason = match result.stop_reason {
                    StopReason::EndTurn => "end_turn",
                    StopReason::ToolUse => "tool_use",
                    StopReason::MaxTokens => "max_tokens",
                    StopReason::MaxTurns => "max_turns",
                    StopReason::Refusal => "refusal",
                };
                if let Some(issue) = &result.completion_adjudication {
                    ("error", stop_reason, issue.kind(), result.turns)
                } else {
                    match result.stop_reason {
                        StopReason::EndTurn => ("ok", stop_reason, "none", result.turns),
                        StopReason::MaxTokens => {
                            ("error", stop_reason, "output_truncated", result.turns)
                        }
                        StopReason::MaxTurns => (
                            "error",
                            stop_reason,
                            "turn_requests_exhausted",
                            result.turns,
                        ),
                        StopReason::Refusal => {
                            ("error", stop_reason, "model_refused", result.turns)
                        }
                        StopReason::ToolUse => {
                            ("error", stop_reason, "protocol_error", result.turns)
                        }
                    }
                }
            }
            Err(error) => (
                "error",
                "error",
                match error {
                    AgentError::ApiError(_) => "api_error",
                    AgentError::Provider(_) => "provider_error",
                    AgentError::UserAborted => "user_aborted",
                    AgentError::ContextTooLong { .. } => "context_too_long",
                    AgentError::Stagnation(_) => "tool_stagnation",
                },
                self.model_turn_attempts,
            ),
        }
    }

    fn observe_results(&mut self, blocks: &[ContentBlock]) {
        for block in blocks {
            let ContentBlock::ToolResult {
                content, is_error, ..
            } = block
            else {
                continue;
            };
            if *is_error {
                self.error_results = self.error_results.saturating_add(1);
            }
            if *is_error && content == SKIPPED_AFTER_PRIOR_ERROR {
                self.skipped_after_prior_error =
                    self.skipped_after_prior_error.saturating_add(1);
            }
        }
    }

    fn log(
        &self,
        session_id: &str,
        msg_id: &str,
        result: &Result<AgentResult, AgentError>,
    ) {
        let (terminal, stop_reason, error_kind, agent_turns) =
            self.terminal_dimensions(result);
        tracing::info!(
            target: "nomi_agent::tool_efficiency",
            session_id,
            msg_id,
            agent_turns,
            stop_reason,
            terminal,
            error_kind,
            model_turn_attempts = self.model_turn_attempts,
            model_turns_with_tools = self.model_turns_with_tools,
            tool_calls_total = self.total_tool_calls,
            max_calls_in_model_turn = self.max_calls_in_model_turn,
            exec_command_script_calls = self.exec_command_script_calls,
            batch_read_files_requested = self.batch_read_files_requested,
            lone_single_file_reads = self.lone_single_file_reads,
            tool_error_results = self.error_results,
            skipped_after_prior_error = self.skipped_after_prior_error,
            "agent tool efficiency summary"
        );
    }
}

/// Consecutive turns with the identical tool-call signature that trip the
/// stagnation nudge. 3 is already a degenerate loop (same action, same args,
/// thrice) — well clear of legitimate retries/polling.
pub(crate) const STAGNATION_THRESHOLD: usize = 3;

pub struct AgentEngine {
    provider: Arc<dyn LlmProvider>,
    tools: ToolRegistry,
    /// Workspace namespace used to turn absolute atomic-tool paths into the
    /// same relative identifiers used by user requirements and nested Agents.
    workspace_root: PathBuf,
    completion_evidence_mode: CompletionEvidenceMode,
    messages: Vec<Message>,
    system_prompt: String,
    model: String,
    output_max_tokens: Option<u32>,
    max_turns: Option<usize>,
    total_usage: TokenUsage,
    thinking: Option<nomi_types::llm::ThinkingConfig>,
    /// Resolved provider compat settings (for capability validation)
    compat: nomi_config::compat::ProviderCompat,
    hooks: Option<HookEngine>,
    session_manager: Option<SessionManager>,
    current_session: Option<Session>,
    output: Arc<dyn OutputSink>,
    current_msg_id: String,
    protocol_writer: Option<Arc<dyn nomi_protocol::writer::ProtocolEmitter>>,
    /// Persisted reasoning effort, updated by skill context modifiers.
    /// Carried into each turn's LlmRequest.reasoning_effort.
    current_reasoning_effort: Option<String>,
    /// Compaction configuration (thresholds, enabled flag, etc.)
    compact_config: CompactConfig,
    /// Runtime compaction state (circuit breaker, last input tokens)
    compact_state: CompactState,
    /// Runtime plan mode state (active flag and plan file path).
    plan_state: PlanState,
    /// Shared flag read by EnterPlanMode/ExitPlanMode tools to validate transitions.
    /// Updated by the engine when processing PlanModeTransition modifiers.
    plan_active_flag: Option<Arc<AtomicBool>>,
    /// Prompt cache break detector for diagnostics.
    cache_detector: CacheBreakDetector,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
    /// How many recent image-bearing tool results keep their images.
    max_recent_images: usize,
    commands: crate::commands::CommandRegistry,
    /// Opt-in goal-driven continuation. `None` (the default) means the engine
    /// behaves exactly as before — no continuation, no `update_goal` tool.
    goal: Option<crate::goal::runtime::GoalRuntime>,
    /// Detects degenerate loops (the identical tool call repeated turn after
    /// turn) and triggers a one-time corrective nudge. Always on — a safety net
    /// alongside the hard `max_turns` cap. (Loop-agent robustness)
    stagnation_guard: crate::loop_guard::StagnationGuard,
    /// Host-registered per-turn context sources (§3.5). Empty by default →
    /// system prompt unchanged; the backend registers contributors to inject
    /// dynamic context (knowledge RAG, memory, …) without the engine hard-coding
    /// each source.
    context_contributors: Vec<std::sync::Arc<dyn crate::context_contributor::ContextContributor>>,
    /// Optional steering inbox: a shared queue the host manager pushes
    /// mid-turn user interjections into. Drained at two loop boundaries
    /// (after a tool-result message, and when a turn would otherwise end)
    /// so the model sees the interjection on its next step without a turn
    /// restart.
    steering_inbox: Option<Arc<Mutex<std::collections::VecDeque<String>>>>,
    /// Trusted host-side resource notifications (terminal closed, resource
    /// revoked, etc.). Unlike `steering_inbox`, these are never represented as
    /// user messages: they are drained into the top-level system context at the
    /// next provider boundary. Keeping the shared queue on the host means a
    /// notice received while the runtime is idle does not create a turn and is
    /// still visible before the next model call.
    system_resource_inbox: Option<Arc<Mutex<std::collections::VecDeque<String>>>>,
    /// Owns every supervised command launched by this engine's command tools.
    /// Bootstrap installs it; direct/test constructors leave it empty.
    process_supervisor: Option<Arc<nomi_process_runtime::ProcessSupervisor>>,
    /// Persisted boundary for the latest editable root user turn.
    ///
    /// The durable source message id keeps automatic continuations and
    /// provider retries from moving the boundary into the middle of one
    /// logical user turn. Compaction and context clearing invalidate it.
    editable_turn: Option<EditableTurnCheckpoint>,
    /// Opaque host-owned routing state. Mirrored into `Session.host_context`
    /// when durable sessions are enabled, but retained in memory for direct or
    /// test engines too.
    host_context: BTreeMap<String, String>,
}

impl AgentEngine {
    /// Create an engine with an externally provided provider for delegated Agents.
    pub fn new_with_provider(
        provider: Arc<dyn LlmProvider>,
        config: Config,
        tools: ToolRegistry,
        output: Arc<dyn OutputSink>,
        cwd: PathBuf,
    ) -> Self {
        let system_prompt = config.system_prompt.clone().unwrap_or_default();

        let session_manager = if config.session.enabled {
            Some(SessionManager::new(
                config.session.directory.clone().into(),
                config.session.max_sessions,
            ))
        } else {
            None
        };

        let compact_config = config.compact.clone();

        Self {
            provider,
            tools,
            workspace_root: cwd.clone(),
            completion_evidence_mode: CompletionEvidenceMode::LocalFingerprint,
            messages: Vec::new(),
            system_prompt,
            model: config.model,
            output_max_tokens: config.output_max_tokens,
            max_turns: config.max_turns,
            total_usage: TokenUsage::default(),
            thinking: config.thinking,
            compat: config.compat.clone(),
            hooks: Some(HookEngine::new(config.hooks.clone(), cwd.clone())),
            session_manager,
            current_session: None,
            output,
            current_msg_id: String::new(),
            protocol_writer: None,
            current_reasoning_effort: None,
            compact_config,
            compact_state: CompactState::new(),
            plan_state: PlanState::default(),
            plan_active_flag: None,
            cache_detector: CacheBreakDetector::new(),
            compaction_level: config.compact.compaction,
            toon_enabled: config.compact.toon,
            max_recent_images: config.tools.max_recent_images,
            commands: crate::commands::default_registry(),
            goal: None,
            stagnation_guard: crate::loop_guard::StagnationGuard::new(crate::engine::STAGNATION_THRESHOLD),
            context_contributors: Vec::new(),
            steering_inbox: None,
            system_resource_inbox: None,
            process_supervisor: None,
            editable_turn: None,
            host_context: BTreeMap::new(),
        }
    }

    /// Create from a resumed session with an externally-provided provider
    pub fn resume_with_provider(
        provider: Arc<dyn LlmProvider>,
        config: Config,
        tools: ToolRegistry,
        output: Arc<dyn OutputSink>,
        session: Session,
        cwd: PathBuf,
    ) -> Self {
        let system_prompt = config.system_prompt.clone().unwrap_or_default();

        let session_manager = if config.session.enabled {
            Some(SessionManager::new(
                config.session.directory.clone().into(),
                config.session.max_sessions,
            ))
        } else {
            None
        };

        let compact_config = config.compact.clone();

        for identity in &session.activated_deferred_tools {
            tools.restore_deferred_tool_activation(identity);
        }

        let editable_turn = session
            .editable_turn
            .clone()
            .filter(|checkpoint| {
                !checkpoint.source_message_id.is_empty()
                    && checkpoint.start_len <= session.messages.len()
            });
        let host_context = session.host_context.clone();

        Self {
            provider,
            tools,
            workspace_root: cwd.clone(),
            completion_evidence_mode: CompletionEvidenceMode::LocalFingerprint,
            messages: session.messages.clone(),
            system_prompt,
            model: config.model.clone(),
            output_max_tokens: config.output_max_tokens,
            max_turns: config.max_turns,
            total_usage: session.total_usage.clone(),
            thinking: config.thinking,
            compat: config.compat.clone(),
            hooks: Some(HookEngine::new(config.hooks.clone(), cwd)),
            session_manager,
            current_session: Some(session),
            output,
            current_msg_id: String::new(),
            protocol_writer: None,
            current_reasoning_effort: None,
            compact_config,
            compact_state: CompactState::new(),
            plan_state: PlanState::default(),
            plan_active_flag: None,
            cache_detector: CacheBreakDetector::new(),
            compaction_level: config.compact.compaction,
            toon_enabled: config.compact.toon,
            max_recent_images: config.tools.max_recent_images,
            commands: crate::commands::default_registry(),
            goal: None,
            stagnation_guard: crate::loop_guard::StagnationGuard::new(crate::engine::STAGNATION_THRESHOLD),
            context_contributors: Vec::new(),
            steering_inbox: None,
            system_resource_inbox: None,
            process_supervisor: None,
            editable_turn,
            host_context,
        }
    }

    pub fn set_process_supervisor(
        &mut self,
        supervisor: Arc<nomi_process_runtime::ProcessSupervisor>,
    ) {
        assert!(
            self.process_supervisor.is_none(),
            "process supervisor may only be installed once"
        );
        if let Some(hooks) = self.hooks.as_mut() {
            hooks.set_process_supervisor(Arc::clone(&supervisor));
        }
        self.process_supervisor = Some(supervisor);
    }

    /// Reusable exact turn boundary for every subprocess registered by this
    /// engine. The caller must not publish a terminal conversation state unless
    /// the returned report proves every process tree reaped.
    pub async fn quiesce_processes(
        &self,
    ) -> Option<nomi_process_runtime::QuiesceReport> {
        let supervisor = self.process_supervisor.as_ref()?;
        Some(supervisor.quiesce().await)
    }

    pub fn process_supervisor_handle(
        &self,
    ) -> Option<Arc<nomi_process_runtime::ProcessSupervisor>> {
        self.process_supervisor.as_ref().map(Arc::clone)
    }

    /// Explicitly wind down all command sessions owned by this engine.
    pub async fn shutdown_processes(&self) -> Option<nomi_process_runtime::ShutdownReport> {
        let supervisor = self.process_supervisor.as_ref()?;
        Some(supervisor.shutdown().await)
    }

    pub fn compaction_level(&self) -> nomi_compact::CompactionLevel {
        self.compaction_level
    }

    /// Get a reference to the shared provider
    pub fn provider(&self) -> &Arc<dyn LlmProvider> {
        &self.provider
    }

    /// Model selected for this live session. Hosts may snapshot this together
    /// with [`Self::provider`] for an isolated, side-effect-free classification
    /// request that must not enter the durable conversation transcript.
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Get a reference to the resolved compat settings
    pub fn compat(&self) -> &nomi_config::compat::ProviderCompat {
        &self.compat
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.tool_names()
    }

    pub fn registry_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tools
    }

    /// Enable goal-driven continuation (opt-in). Registers the `update_goal`
    /// tool and installs a `GoalRuntime` that injects a continuation prompt at
    /// each natural-termination point until the goal is proven complete /
    /// blocked, or the auto-continuation cap (or `max_turns`) is hit.
    pub fn set_goal(&mut self, objective: String, max_auto_continuations: usize) {
        let rt = crate::goal::runtime::GoalRuntime::new(objective, max_auto_continuations);
        self.tools
            .register(Box::new(crate::goal::tool::UpdateGoalTool::new(
                rt.shared_state(),
            )));
        self.goal = Some(rt);
    }

    /// Initialize a new session for this Agent engine.
    pub fn init_session(
        &mut self,
        provider_name: &str,
        cwd: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(mgr) = &self.session_manager {
            let session = mgr.create(provider_name, &self.model, cwd, session_id)?;
            tracing::info!(target: "nomi_agent", session_id = %session.id, provider = %provider_name, model = %self.model, "session started");
            self.host_context.clear();
            self.current_session = Some(session);
        }
        Ok(())
    }

    /// Get the current session ID (if sessions are enabled and initialized)
    pub fn current_session_id(&self) -> Option<String> {
        self.current_session.as_ref().map(|s| s.id.clone())
    }

    /// Read opaque host-owned state restored with the current session.
    pub fn host_context_value(&self, key: &str) -> Option<String> {
        self.host_context.get(key).cloned()
    }

    /// Persist or clear opaque host-owned routing state without exposing it to
    /// the model transcript.
    pub fn set_host_context_value(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => {
                self.host_context.insert(key.to_owned(), value.to_owned());
            }
            None => {
                self.host_context.remove(key);
            }
        }
        self.save_session();
    }

    /// Current context occupancy: the last request's prompt token count
    /// (system + all messages + tool results). 0 before the first model call.
    /// Numerator for the context-usage gauge.
    pub fn context_tokens(&self) -> u64 {
        self.compact_state.last_input_tokens
    }

    /// The engine's effective context budget (what it compacts against).
    /// Denominator for the gauge. = CompactConfig.context_window.
    pub fn context_window(&self) -> u64 {
        self.compact_config.context_window as u64
    }

    /// Install (or clear) the steering inbox used for mid-turn interjections.
    pub fn set_steering_inbox(
        &mut self,
        inbox: Option<Arc<Mutex<std::collections::VecDeque<String>>>>,
    ) {
        self.steering_inbox = inbox;
    }

    /// Install (or clear) the trusted host resource-notification inbox.
    ///
    /// Resource notices intentionally have a separate channel from user
    /// steering: the engine injects them into the provider's top-level system
    /// context, never into the conversation transcript as a user message.
    pub fn set_system_resource_inbox(
        &mut self,
        inbox: Option<Arc<Mutex<std::collections::VecDeque<String>>>>,
    ) {
        self.system_resource_inbox = inbox;
    }

    /// Take all currently-queued steering interjections (FIFO). Empty when
    /// no inbox is installed. Lock is held only for the drain; a poisoned
    /// lock degrades to its inner value rather than panicking the turn.
    fn drain_steering(&self) -> Vec<String> {
        match &self.steering_inbox {
            Some(inbox) => {
                let mut q = inbox.lock().unwrap_or_else(|e| e.into_inner());
                q.drain(..).collect()
            }
            None => Vec::new(),
        }
    }

    /// Take all trusted host resource notices currently queued (FIFO).
    fn drain_system_resource_notices(&self) -> Vec<String> {
        match &self.system_resource_inbox {
            Some(inbox) => {
                let mut q = inbox.lock().unwrap_or_else(|e| e.into_inner());
                q.drain(..).collect()
            }
            None => Vec::new(),
        }
    }

    /// Register a per-turn [`ContextContributor`] (§3.5). The backend uses this
    /// to inject dynamic context (knowledge RAG, memory, …) into the system
    /// prompt without the engine hard-coding the source. No-op effect on prompts
    /// until at least one is registered.
    pub fn register_context_contributor(
        &mut self,
        contributor: std::sync::Arc<dyn crate::context_contributor::ContextContributor>,
    ) {
        self.context_contributors.push(contributor);
    }

    /// Get a reference to the output sink
    pub fn output(&self) -> &dyn OutputSink {
        self.output.as_ref()
    }

    /// A readable transcript of the conversation history (role-tagged text,
    /// with tool use / tool results compressed and truncated; thinking blocks
    /// dropped). Used by post-session memory distillation as a read-only
    /// snapshot — it never mutates engine state.
    pub fn messages_transcript(&self) -> String {
        render_transcript(&self.messages)
    }

    pub fn set_protocol_writer(&mut self, writer: Arc<dyn nomi_protocol::writer::ProtocolEmitter>) {
        self.protocol_writer = Some(writer);
    }

    /// Set the initial reasoning effort override used by delegated Agent invocations.
    pub fn set_initial_reasoning_effort(&mut self, effort: Option<String>) {
        self.current_reasoning_effort = effort;
    }

    /// Set the shared plan-mode active flag.
    ///
    /// This flag is shared with EnterPlanMode/ExitPlanMode tools so they can
    /// validate transitions (e.g. reject double-entry).  The engine updates
    /// the flag when processing `PlanModeTransition` context modifiers.
    pub fn set_plan_active_flag(&mut self, flag: Arc<AtomicBool>) {
        self.plan_active_flag = Some(flag);
    }

    /// Bind completion evidence to the remote tool namespace. Remote sessions
    /// must never probe the host application's local cwd as proof of SSH work.
    pub(crate) fn set_remote_completion_evidence(&mut self) {
        self.completion_evidence_mode = CompletionEvidenceMode::RemoteExactReceipts;
    }

    /// Whether execution is currently constrained to plan-mode tools.
    ///
    /// This is deliberately read-only: host routers need to avoid creating an
    /// execution/artifact obligation that the plan-mode tool policy cannot
    /// possibly satisfy, while transitions remain owned by the plan tools.
    pub fn is_plan_mode_active(&self) -> bool {
        self.plan_state.is_active
    }

    /// Default thinking budget when "enabled" is requested without a specific budget.
    const DEFAULT_THINKING_BUDGET: u32 = 10_000;

    /// Apply a runtime config update received from the protocol layer.
    ///
    /// Returns a list of human-readable change descriptions for the Info event.
    /// Empty list means no fields were changed.
    pub fn apply_config_update(
        &mut self,
        model: Option<String>,
        thinking: Option<String>,
        thinking_budget: Option<u32>,
        effort: Option<String>,
        compaction: Option<String>,
    ) -> Vec<String> {
        let mut changes = Vec::new();

        if let Some(new_model) = model {
            let old = std::mem::replace(&mut self.model, new_model.clone());
            changes.push(format!("model: {old} → {new_model}"));
        }

        if let Some(thinking_str) = thinking {
            if !self.compat.supports_thinking() {
                changes.push("thinking: not supported by current provider".to_string());
            } else {
                match thinking_str.as_str() {
                    "enabled" => {
                        let budget = thinking_budget.unwrap_or(Self::DEFAULT_THINKING_BUDGET);
                        self.thinking = Some(nomi_types::llm::ThinkingConfig::Enabled {
                            budget_tokens: budget,
                        });
                        changes.push(format!("thinking: enabled (budget: {budget})"));
                    }
                    "disabled" => {
                        self.thinking = Some(nomi_types::llm::ThinkingConfig::Disabled);
                        changes.push("thinking: disabled".to_string());
                    }
                    other => {
                        changes.push(format!("thinking: ignored invalid value \"{other}\""));
                    }
                }
            }
        } else if let Some(new_budget) = thinking_budget
            && let Some(nomi_types::llm::ThinkingConfig::Enabled { budget_tokens }) =
                &mut self.thinking
        {
            *budget_tokens = new_budget;
            changes.push(format!("thinking budget: {new_budget}"));
        }

        if let Some(new_effort) = effort {
            if new_effort.is_empty() {
                self.current_reasoning_effort = None;
                changes.push("effort: cleared".to_string());
            } else if !self.compat.supports_effort() {
                changes.push("effort: not supported by current provider".to_string());
            } else {
                let levels = self.compat.effort_levels();
                if !levels.is_empty() && !levels.iter().any(|l| l == &new_effort) {
                    changes.push(format!(
                        "effort: invalid level \"{}\" (valid: {})",
                        new_effort,
                        levels.join(", ")
                    ));
                } else {
                    let old = self
                        .current_reasoning_effort
                        .replace(new_effort.clone())
                        .unwrap_or_else(|| "none".to_string());
                    changes.push(format!("effort: {old} → {new_effort}"));
                }
            }
        }

        if let Some(ref level_str) = compaction {
            match level_str.parse::<nomi_compact::CompactionLevel>() {
                Ok(new_level) => {
                    let old = self.compaction_level.to_string();
                    self.compaction_level = new_level;
                    changes.push(format!("compaction: {old} → {new_level}"));
                }
                Err(e) => {
                    changes.push(format!("compaction: invalid ({e})"));
                }
            }
        }

        changes
    }

    /// Handle a slash command. Returns `None` if input is not a recognized command.
    pub async fn handle_command(
        &mut self,
        input: &str,
    ) -> Option<Result<crate::commands::CommandResult, anyhow::Error>> {
        let input = input.trim();
        let without_slash = input.strip_prefix('/')?;
        let (name, args) = match without_slash.split_once(char::is_whitespace) {
            Some((n, rest)) => (n, rest.trim()),
            None => (without_slash, ""),
        };

        // find() returns a cloned Arc, so the registry borrow ends here and
        // self can be mutably borrowed for CommandContext below.
        let cmd = self.commands.find(name)?;

        let mut ctx = crate::commands::CommandContext {
            messages: &mut self.messages,
            compact_state: &mut self.compact_state,
            compact_config: &self.compact_config,
            provider: Arc::clone(&self.provider),
            model: &self.model,
            output: self.output.as_ref(),
            registry: &self.commands,
        };

        let result = cmd.execute(&mut ctx, args).await;
        if result.is_ok() && matches!(name, "clear" | "compact") {
            self.editable_turn = None;
            self.save_session();
        }
        Some(result)
    }

    /// Execute one Agent turn from plain user input.
    pub async fn execute_turn(
        &mut self,
        user_input: &str,
        msg_id: &str,
    ) -> Result<AgentResult, AgentError> {
        self.execute_turn_with_content(
            vec![ContentBlock::Text {
                text: user_input.to_string(),
            }],
            msg_id,
        )
        .await
    }

    /// Execute one Agent turn from pre-built user content.
    ///
    /// This is the multimodal counterpart to [`Self::execute_turn`]. Hosts may include
    /// text and already-validated, base64-encoded image blocks. Tool/thinking
    /// blocks are rejected so a caller cannot forge assistant or tool history.
    pub async fn execute_turn_with_content(
        &mut self,
        user_content: Vec<ContentBlock>,
        msg_id: &str,
    ) -> Result<AgentResult, AgentError> {
        self.execute_turn_with_content_for_source(user_content, msg_id, msg_id)
            .await
    }

    /// Execute an engine pass belonging to one durable root user message.
    ///
    /// Automatic continuation and provider-retry passes carry the same
    /// `source_message_id`, so they retain the root checkpoint rather than
    /// moving it into the middle of the logical turn.
    pub async fn execute_turn_with_content_for_source(
        &mut self,
        user_content: Vec<ContentBlock>,
        msg_id: &str,
        source_message_id: &str,
    ) -> Result<AgentResult, AgentError> {
        self.execute_turn_with_content_for_source_and_tool_allowlist(
            user_content,
            msg_id,
            source_message_id,
            None,
        )
        .await
    }

    /// Execute one durable root-user turn while restricting the exact tools
    /// advertised to the provider for every model pass in that turn.
    ///
    /// `None` preserves the normal session tool surface. `Some` is a strict,
    /// request-scoped allow-list; it neither mutates the registry nor persists
    /// into a later turn. Hosts use this for intent routes whose security and
    /// cost policy permits only a dedicated native tool (for example ordinary
    /// image generation). Dispatch remains fail-closed because the provider
    /// request's tool definitions are also the execution authority.
    pub async fn execute_turn_with_content_for_source_and_tool_allowlist(
        &mut self,
        user_content: Vec<ContentBlock>,
        msg_id: &str,
        source_message_id: &str,
        tool_allowlist: Option<&HashSet<String>>,
    ) -> Result<AgentResult, AgentError> {
        self.execute_turn_with_completion_evidence_context(
            user_content,
            msg_id,
            source_message_id,
            tool_allowlist,
            None,
        )
        .await
    }

    /// Execute one engine pass with an optional host-owned accepted-turn
    /// completion scope. Direct/CLI callers use the ordinary entrypoint above;
    /// decorated desktop turns use this seam to keep adjudication tied to the
    /// raw user request and to union evidence across steering race-tail passes.
    pub async fn execute_turn_with_completion_evidence_context(
        &mut self,
        user_content: Vec<ContentBlock>,
        msg_id: &str,
        source_message_id: &str,
        tool_allowlist: Option<&HashSet<String>>,
        completion_context: Option<&mut CompletionEvidenceContext>,
    ) -> Result<AgentResult, AgentError> {
        let host_owns_terminal_commit = completion_context.is_some();
        let mut local_completion_context = CompletionEvidenceContext::new(user_content.clone());
        let completion_context = completion_context.unwrap_or(&mut local_completion_context);
        let runtime_authority = self.snapshot_runtime_authority();
        completion_context.capture_turn_root(
            source_message_id,
            &self.messages,
            &self.editable_turn,
            &self.host_context,
            self.tools.session_deferred_tool_identities(),
            runtime_authority,
        )?;
        self.persist_accepted_turn_root(completion_context)?;
        if host_owns_terminal_commit
            && self.session_manager.is_some()
            && self.current_session.is_some()
        {
            completion_context
                .host_recovery_required
                .store(true, Ordering::Release);
        }
        completion_context
            .ensure_target_baselines(&self.workspace_root, self.completion_evidence_mode)
            .await;
        let first_new_message = self.messages.len();
        let session_id = self
            .current_session
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let span = tracing::info_span!(
            target: "nomi_agent",
            "turn_execution",
            session_id = %session_id,
            msg_id = %msg_id,
        );
        let mut efficiency = ToolEfficiencyStats::default();
        let mut safe_messages = self.messages.clone();
        let mut turn_started = false;
        let mut result = async {
            self.execute_turn_inner(
                    user_content,
                    msg_id,
                    source_message_id,
                    tool_allowlist,
                    completion_context,
                    &mut efficiency,
                    &mut safe_messages,
                    &mut turn_started,
                )
                .await
        }
        .instrument(span)
        .await;

        if !host_owns_terminal_commit
            && result.as_ref().is_ok_and(|agent_result| {
                agent_result.completion_adjudication.is_none()
            })
            && !self.finalize_committed_completion_turn(completion_context)
            && let Ok(agent_result) = &mut result
        {
            agent_result.completion_adjudication = Some(
                CompletionAdjudication::SessionCommitFailed {
                    detail: "the accepted Agent turn could not be durably committed".to_owned(),
                },
            );
        }

        if let Ok(agent_result) = &result {
            completion_context.successful_mutation_observed |= agent_result.effects_ok > 0;
            for target in &agent_result.durable_effect_targets {
                if !completion_context
                    .prior_durable_effect_targets
                    .contains(target)
                {
                    completion_context
                        .prior_durable_effect_targets
                        .push(target.clone());
                }
            }
        }

        // Keep the image available for every provider/tool iteration in this
        // turn execution, then remove it before the engine is reused. `execute_turn_inner`
        // has several success/error return paths and may already have persisted
        // the original turn, so perform the cleanup in this outer finally-like
        // wrapper and save the redacted transcript once more. If the host drops
        // this future during non-cooperative cancellation, `abort_current_turn`
        // performs the same cleanup explicitly.
        if result
            .as_ref()
            .is_ok_and(|result| result.completion_adjudication.is_some())
            && turn_started
        {
            if result.as_ref().is_ok_and(|agent_result| {
                matches!(
                    agent_result.completion_adjudication,
                    Some(CompletionAdjudication::SessionCommitFailed { .. })
                )
            }) {
                // Inner A2 verdicts already retract their provider pass before
                // returning. A direct-call session commit failure is detected
                // only here, after the text streamed, so publish the matching
                // rollback signal before restoring history.
                self.output
                    .emit_accepted_turn_output_discarded(&self.current_msg_id);
            }
            // Restore the accepted-turn root captured by the host-owned
            // context, not merely this execute call's pre-state. A host
            // race-tail pass may be the second engine call for one accepted
            // turn, and compaction may already have invalidated the ordinary
            // editable checkpoint.
            let rollback_persisted = if let Some(root) = completion_context.turn_root.clone() {
                // Hidden in-memory authority first: a rejected turn must not
                // fresh reload would not have.
                self.restore_runtime_authority(completion_context);
                self.tools
                    .replace_deferred_tool_activations(&root.activated_deferred_tools);
                self.messages = root.messages;
                self.editable_turn = root.editable_turn;
                self.host_context = root.host_context;
                if let Some(session) = &mut self.current_session {
                    session.accepted_turn_root = None;
                    session.pending_host_terminal_root = None;
                    session.last_interrupted_turn_source =
                        Some(root.source_message_id.clone());
                }
                self.try_save_session().is_ok()
            } else {
                false
            };
            if !rollback_persisted
                && let Ok(agent_result) = &mut result
                && let Some(issue) = &mut agent_result.completion_adjudication
            {
                issue.mark_history_rollback_failed();
            } else {
                completion_context
                    .host_recovery_required
                    .store(false, Ordering::Release);
            }
        } else if result.is_err() && turn_started {
            // A provider error (or a dropped/cancelled turn observed as one)
            // rewinds this attempt. Host-owned turns additionally restore the
            // exact root through `restore_uncommitted_completion_attempt`; both
            // paths must undo the authority this attempt granted itself.
            self.restore_runtime_authority(completion_context);
            self.messages = safe_messages;
            if matches!(
                &result,
                Err(AgentError::Provider(_)
                    | AgentError::ApiError(_)
                    | AgentError::ContextTooLong { .. })
            ) {
                self.strip_tool_images_after_provider_error();
            }
            if !host_owns_terminal_commit
                && let Some(session) = &mut self.current_session
            {
                session.accepted_turn_root = None;
            }
            self.save_session();
        } else if !host_owns_terminal_commit && !turn_started {
            // Slash commands and pre-provider validation do not create a
            // provider transaction. Do not leave their preflight marker to be
            // mistaken for a crash-interrupted turn on the next resume.
            let _ = self.finalize_committed_completion_turn(completion_context);
        } else if self.redact_user_images_since(first_new_message) {
            self.save_session();
        }
        efficiency.log(&session_id, msg_id, &result);
        result
    }

    /// Retract a completed engine pass whose host-side delivery transaction did
    /// not commit, and durably restore the exact accepted-turn root captured
    /// before provider work began. Hosts must treat `false` as a quarantining
    /// state-consistency failure: the in-memory root was restored, but the
    /// resumable session could not be made authoritative.
    pub fn restore_uncommitted_completion_turn(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> bool {
        self.output
            .emit_accepted_turn_output_discarded(&self.current_msg_id);
        self.restore_uncommitted_completion_root(completion_context)
    }

    /// Restore the exact transcript root while retracting only the current
    /// provider attempt from the visible failed-turn evidence. Provider errors
    /// and in-flight cancellation intentionally retain an earlier valid
    /// race-tail prefix; host transaction failures use the full-turn method.
    pub fn restore_uncommitted_completion_attempt(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> bool {
        self.output
            .emit_current_attempt_output_discarded(&self.current_msg_id, 1);
        self.restore_uncommitted_completion_root(completion_context)
    }

    fn restore_uncommitted_completion_root(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> bool {
        let Some(root) = completion_context.turn_root.clone() else {
            return false;
        };
        // Hidden in-memory authority first, so a failed session write below
        // still reports a state-consistency failure for a runtime that is no
        // longer carrying the rejected turn's grants.
        self.restore_runtime_authority(completion_context);
        self.tools
            .replace_deferred_tool_activations(&root.activated_deferred_tools);
        self.messages = root.messages;
        self.editable_turn = root.editable_turn;
        self.host_context = root.host_context;
        if let Some(session) = &mut self.current_session {
            session.accepted_turn_root = None;
            session.pending_host_terminal_root = None;
            session.last_interrupted_turn_source = Some(root.source_message_id.clone());
        }
        let restored = self.try_save_session().is_ok();
        if restored {
            completion_context
                .host_recovery_required
                .store(false, Ordering::Release);
        }
        restored
    }

    /// Commit the accepted-turn transcript after its owner has completed every
    /// post-model effect. Clearing the durable root marker and saving the
    /// current transcript is the session-side half of the terminal commit; a
    /// failed write must prevent the host from publishing `Finish`.
    pub fn finalize_committed_completion_turn(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> bool {
        let Some(expected_root) = completion_context.turn_root.as_ref() else {
            return false;
        };
        let Some(session) = &mut self.current_session else {
            return true;
        };
        match session.accepted_turn_root.as_ref() {
            Some(persisted)
                if persisted.source_message_id == expected_root.source_message_id =>
            {
                session.accepted_turn_root = None;
                session.pending_host_terminal_root = None;
                session.last_interrupted_turn_source = None;
                let committed = self.try_save_session().is_ok();
                if committed {
                    completion_context
                        .host_recovery_required
                        .store(false, Ordering::Release);
                }
                committed
            }
            // A marker-free in-memory-only engine has no resumable state to
            // commit. A persisted session reaching this branch is a lifecycle
            // mismatch and must fail closed.
            None if self.session_manager.is_none() => true,
            _ => false,
        }
    }

    /// Durably seal a desktop-hosted transcript before any verified artifact
    /// or terminal event becomes visible. Unlike a direct CLI commit, retain
    /// the exact root in a host-terminal phase: Conversation boot may still
    /// observe the authoritative receipt as Running if the process dies in
    /// the narrow no-await publish window. Ordinary session resume treats this
    /// phase as committed and never rolls it back on its own.
    pub fn seal_completion_for_host_terminal(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> bool {
        let Some(expected_root) = completion_context.turn_root.as_ref() else {
            return false;
        };
        let Some(session) = &mut self.current_session else {
            return true;
        };
        match session.accepted_turn_root.take() {
            Some(persisted)
                if persisted.source_message_id == expected_root.source_message_id =>
            {
                session.pending_host_terminal_root = Some(persisted);
                session.last_interrupted_turn_source = None;
                self.try_save_session().is_ok()
            }
            Some(persisted) => {
                session.accepted_turn_root = Some(persisted);
                false
            }
            None => false,
        }
    }

    /// Persist a deterministic host response as a normal text exchange without
    /// making a provider request. The host is responsible for publishing the
    /// corresponding stream events. This keeps engine/session history aligned
    /// when a capability route must fail fast before model execution (for
    /// example, image generation with no configured image model).
    pub fn record_host_text_turn(
        &mut self,
        user_text: impl Into<String>,
        assistant_text: impl Into<String>,
        source_message_id: &str,
    ) -> Result<(), AgentError> {
        let user_text = user_text.into();
        let assistant_text = assistant_text.into();
        if user_text.trim().is_empty() || assistant_text.trim().is_empty() {
            return Err(AgentError::ApiError(
                "host text turns require non-empty user and assistant text".to_owned(),
            ));
        }
        let prior_messages = self.messages.clone();
        let prior_editable_turn = self.editable_turn.clone();
        let prior_host_context = self.host_context.clone();
        let prior_session = self.current_session.clone();
        self.editable_turn = Some(EditableTurnCheckpoint {
            source_message_id: source_message_id.to_owned(),
            start_len: self.messages.len(),
            prior_host_context: self.host_context.clone(),
        });
        self.messages.push(Message::now(
            Role::User,
            vec![ContentBlock::Text { text: user_text }],
        ));
        self.messages.push(Message::now(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: assistant_text,
            }],
        ));
        if let Err(error) = self.try_save_session() {
            self.messages = prior_messages;
            self.editable_turn = prior_editable_turn;
            self.host_context = prior_host_context;
            self.current_session = prior_session;
            let rollback = self.try_save_session();
            return Err(AgentError::ApiError(match rollback {
                Ok(()) => format!(
                    "failed to persist the deterministic host turn; the prior session was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "failed to persist the deterministic host turn ({error}) and failed to restore the prior session ({rollback_error})"
                ),
            }));
        }
        Ok(())
    }

    /// Return metadata for all registered slash commands.
    pub fn slash_command_list(&self) -> Vec<(String, String)> {
        self.commands
            .all()
            .iter()
            .map(|cmd| (cmd.name().to_string(), cmd.description().to_string()))
            .collect()
    }

    async fn execute_turn_inner(
        &mut self,
        user_content: Vec<ContentBlock>,
        msg_id: &str,
        source_message_id: &str,
        tool_allowlist: Option<&HashSet<String>>,
        completion_context: &mut CompletionEvidenceContext,
        efficiency: &mut ToolEfficiencyStats,
        safe_messages: &mut Vec<Message>,
        turn_started: &mut bool,
    ) -> Result<AgentResult, AgentError> {
        if user_content.is_empty()
            || user_content
                .iter()
                .any(|block| !matches!(block, ContentBlock::Text { .. } | ContentBlock::Image { .. }))
        {
            return Err(AgentError::ApiError(
                "user content must contain only text or image blocks".to_string(),
            ));
        }

        // Slash command interception — before any LLM call. Commands remain
        // text-only; attaching an image makes the input an ordinary model turn.
        let command_input = match user_content.as_slice() {
            [ContentBlock::Text { text }] => Some(text.as_str()),
            _ => None,
        };
        if let Some(user_input) = command_input
            && let Some(result) = self.handle_command(user_input).await
        {
            let cmd_name = user_input.split_whitespace().next().unwrap_or(user_input);
            return match result {
                Ok(crate::commands::CommandResult::Exit) => {
                    tracing::info!(command = cmd_name, "Slash command executed: exit");
                    Err(AgentError::UserAborted)
                }
                Ok(crate::commands::CommandResult::Continue) => {
                    tracing::info!(command = cmd_name, "Slash command executed");
                    Ok(AgentResult {
                        text: String::new(),
                        stop_reason: StopReason::EndTurn,
                        usage: TokenUsage::default(),
                        turns: 0,
                        // A slash command runs no provider pass at all.
                        rounds: 1,
                        effects_ok: 0,
                        durable_effect_targets: Vec::new(),
                        cutoff_state_changing: 0,
                        state_changing_tools_advertised: false,
                        completion_adjudication: None,
                    })
                }
                Err(e) => {
                    tracing::error!(command = cmd_name, error = %e, "Slash command failed");
                    Err(AgentError::ApiError(e.to_string()))
                }
            };
        }

        // Stagnation is scoped to one user-request execution. A later user
        // instruction starts with a clean progress window.
        self.stagnation_guard.reset();
        self.current_msg_id = msg_id.to_string();
        self.output.emit_stream_start(msg_id);
        if self
            .editable_turn
            .as_ref()
            .is_none_or(|checkpoint| checkpoint.source_message_id != source_message_id)
        {
            self.editable_turn = Some(EditableTurnCheckpoint {
                source_message_id: source_message_id.to_owned(),
                start_len: self.messages.len(),
                prior_host_context: self.host_context.clone(),
            });
        }
        // The accepted requirement, captured BEFORE the push below moves
        // `user_content` into the transcript. This owned clone is the only thing
        // a restart can re-push, and holding it as a stack local — rather than
        // an index into `self.messages` — is what makes the anchor survive
        // autocompaction, which replaces the whole message vector (and can
        // reduce the root user message to a summary) from inside this very loop.
        let round_requirement = user_content.clone();
        let mut completion_requirement = completion_context.requirement.clone();
        self.messages.push(Message::now(Role::User, user_content));
        *turn_started = true;
        // Persist before the first provider await. A stop or process exit must
        // not discard rewind authority for the accepted user message.
        self.save_session();

        let mut round = round::RoundState::new(round_requirement);
        round.ledger.record_durable_effect_targets(
            completion_context
                .prior_durable_effect_targets
                .iter()
                .cloned(),
        );
        // Accumulated across every pass of this turn, so a later tool-less pass
        // cannot erase the fact that state-changing work was on the table.
        let mut state_changing_tools_advertised = false;
        let mut turn: usize = 0;
        let mut tool_retry_tracker = ToolRetryTracker::default();
        let mut routed_tool_calls_seen = 0usize;
        let mut artifact_retry_blocked = false;
        let mut completion_evidence_nudged = completion_context.corrective_nudge_used;
        let mut spec_recheck_nudged = false;
        let mut tool_error_budget_nudged = false;
        let mut batch_read_nudged = false;
        let mut provider_pass_started = false;
        loop {
            // Hard safety net: an unconfigured (`None`) max_turns still gets a
            // bounded cap so a runaway tool-call loop cannot run forever. A
            // user-configured limit is respected verbatim.
            let limit = self.max_turns.unwrap_or(DEFAULT_SAFETY_MAX_TURNS);
            if turn >= limit {
                self.save_session();
                return Ok(AgentResult {
                    text: String::new(),
                    stop_reason: StopReason::MaxTurns,
                    usage: self.total_usage.clone(),
                    turns: turn,
                    rounds: round.attempt,
                    effects_ok: round.ledger.effects_ok_total,
                    durable_effect_targets: round.ledger.durable_effect_targets.clone(),
                    cutoff_state_changing: round.ledger.cutoff_state_changing_total,
                    state_changing_tools_advertised,
                    completion_adjudication: None,
                });
            }
            // Enforce the per-request provider ceiling on preloaded/resumed
            // history as well as newly appended tool results. This must happen
            // before compaction because autocompaction can itself call the
            // provider with the current conversation.
            self.prune_old_tool_images();

            // Pre-send token estimate (§3.1): feed the CURRENT message size into
            // the compaction watermark so a turn that grew large (a big tool
            // result, or a large first message) compacts BEFORE the request
            // rather than failing with PromptTooLong and wasting a round-trip.
            // Only ever RAISES the watermark, and reuses the existing autocompact
            // thresholds + circuit breaker, so it cannot over-compact a small
            // context or loop.
            let pre_send_estimate =
                estimate::estimate_tokens_from_messages(&self.messages);
            self.compact_state.last_input_tokens =
                self.compact_state.last_input_tokens.max(pre_send_estimate);

            // Run multi-level compaction before each API call.
            self.run_compaction().await?;

            // Build tool list: filter based on plan mode state
            let route_allows = |tool: &dyn nomi_tools::Tool| {
                let route_matches = tool_allowlist.as_ref().map_or_else(
                    || !tool.requires_explicit_route(),
                    |allowed| allowed.contains(tool.name()),
                );
                let is_blocked_artifact = artifact_retry_blocked
                    && crate::output::artifact_contract(tool.artifact_identity()).is_some();
                route_matches && !is_blocked_artifact
            };
            let tools = if tool_allowlist.is_some() && routed_tool_calls_seen > 0 {
                // A strict route has already executed its single authorized
                // capability. The follow-up provider pass receives only the
                // compact verified result context, never another billable tool
                // schema it could try to invoke again.
                Vec::new()
            } else if self.plan_state.is_active {
                // Plan mode: only Info-category tools (excluding EnterPlanMode)
                self.tools.to_tool_defs_filtered(|t| {
                    t.category() == ToolCategory::Info
                        && t.name() != "EnterPlanMode"
                        && route_allows(t)
                })
            } else {
                // Normal mode: all tools except ExitPlanMode
                self.tools
                    .to_tool_defs_filtered(|t| t.name() != "ExitPlanMode" && route_allows(t))
            };
            // This exact request is the authority for what the provider may
            // call. Registry membership is broader (for example, plan mode
            // deliberately hides mutating tools), so dispatch must never use
            // the live registry as an implicit allow-list.
            let tool_authority = ProviderToolAuthority::from_request_tools(&tools);
            // What this exact request made possible, captured before `tools`
            // moves into the LlmRequest below.
            //
            // `tools_advertised` is per-pass and gates the resumable restart: a
            // request with no tools (a provider health check, a model-only
            // answer) has no tool work to resume, so restarting it would only
            // re-run the same generation against the same ceiling.
            //
            // `state_changing_tools_advertised` accumulates across every pass of
            // the turn and gates the no-progress verdict. It must be recovered
            // from the registry because `ToolDef` carries no category, and it
            // stays false for plan mode (Info-only) and for model-only runtimes
            // (`update_plan` alone, also Info) so a turn that COULD NOT have
            // produced a state-changing effect is never judged for failing to.
            let tools_advertised = !tools.is_empty();
            state_changing_tools_advertised |= tools.iter().any(|def| {
                self.tools
                    .get(&def.name)
                    .is_some_and(|tool| round::is_state_changing(tool.category()))
            });

            // Build system prompt: append plan mode instructions when active
            let system = if self.plan_state.is_active {
                format!(
                    "{}\n\n{}",
                    self.system_prompt,
                    plan_prompt::plan_mode_instructions()
                )
            } else {
                self.system_prompt.clone()
            };

            // §3.5: let registered contributors inject dynamic per-turn context
            // (knowledge RAG, memory, …). No-op when none are registered.
            let system = if self.context_contributors.is_empty() {
                system
            } else {
                let mut extras = Vec::new();
                for contributor in &self.context_contributors {
                    if let Some(extra) = contributor.pre_turn_context().await {
                        extras.push(extra);
                    }
                }
                crate::context_contributor::merge_pre_turn_context(system, extras)
            };

            let system = if tool_allowlist.is_some() {
                let declared_tools = if tools.is_empty() {
                    "(none)".to_owned()
                } else {
                    tools
                        .iter()
                        .map(|tool| format!("`{}`", tool.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!(
                    "{REQUEST_SCOPED_TOOL_AUTHORITY_HEADER}\n{REQUEST_SCOPED_TOOL_AUTHORITY_RULE}\nDeclared tools for this request: {declared_tools}\n\n{system}"
                )
            } else {
                system
            };

            // Host resource notifications use the trusted system channel, not
            // Role::User. Drain as late as possible before constructing the
            // request so idle notices and mid-turn notices both reach the next
            // provider boundary without creating a new Agent turn.
            let system = append_system_resource_context(
                system,
                self.drain_system_resource_notices(),
            );
            // Round facts last, so a restart's carried-forward ledger is the
            // closest thing to the request in the system channel. Reuses the
            // contributor merge: this is the same "append trimmed non-empty
            // blocks to the system prompt" contract, on the same channel.
            let system = crate::context_contributor::merge_pre_turn_context(
                system,
                round.take_section().into_iter().collect(),
            );

            // Record prompt state for cache diagnostics
            self.cache_detector.record_request(&system, &tools);

            let request = LlmRequest {
                model: self.model.clone(),
                system,
                messages: self.messages.clone(),
                tools,
                max_tokens: self.output_max_tokens,
                thinking: self.thinking.clone(),
                reasoning_effort: self.current_reasoning_effort.clone(),
                retain_provider_round: self.compat.chain_rounds(),
            };

            if provider_pass_started {
                self.output.emit_output_checkpoint(&self.current_msg_id);
            } else {
                provider_pass_started = true;
            }
            efficiency.observe_model_turn_attempt();
            let stream_start = std::time::Instant::now();
            let mut rx = self.provider.stream(&request).await?;
            let mut assistant_text = String::new();
            let mut thinking_text = String::new();
            let mut thinking_signature: Option<String> = None;
            let mut provider_round_id: Option<String> = None;
            let mut tool_calls: Vec<ContentBlock> = Vec::new();
            let mut previewed_tool_calls: BTreeMap<String, String> = BTreeMap::new();
            let mut stop_reason = StopReason::EndTurn;
            // Calls this pass's ceiling cut off. Declared beside `stop_reason`
            // so it resets on every provider pass: a cutoff belongs to the pass
            // that produced it, and carrying a stale one forward would render a
            // wrong fact into the next attempt's prompt.
            let mut truncated_calls: Vec<round::LedgerCutoff> = Vec::new();
            // Cursor safety is stricter than the restart ledger: even a
            // malformed/unadvertised truncated call means the retained remote
            // response may contain an unresolved function call and therefore
            // cannot be a legal parent.
            let mut saw_truncated_tool_use = false;
            let mut turn_usage = TokenUsage::default();
            let mut done_count = 0_u8;

            let mut idle_activity_active = false;
            let mut first_token_logged = false;
            loop {
                let event = tokio::time::timeout(STREAM_IDLE_ACTIVITY_AFTER, rx.recv()).await;
                let event = match event {
                    Ok(event) => event,
                    Err(_) => {
                        if !idle_activity_active {
                            self.output
                                .emit_model_activity(&self.current_msg_id, "preparing");
                            idle_activity_active = true;
                        }
                        continue;
                    }
                };
                if idle_activity_active {
                    self.output
                        .emit_model_activity(&self.current_msg_id, "prepared");
                    idle_activity_active = false;
                }
                let Some(event) = event else { break };
                if done_count != 0 {
                    // Done is the provider-turn commit point. Accepting any
                    // later event would make terminal reason validation depend
                    // on event ordering (and historically let a second Done
                    // overwrite MaxTokens), so fail before message insertion or
                    // tool dispatch.
                    efficiency.observe_calls(&self.tools, &tool_calls);
                    return Err(AgentError::ApiError(
                        "provider stream protocol violation: event emitted after terminal Done"
                            .to_string(),
                    ));
                }
                if provider_round_id.is_some()
                    && !matches!(&event, LlmEvent::Done { .. })
                {
                    // ProviderRoundId is a commit attachment, not stream
                    // content. Requiring it immediately before Done prevents
                    // a provider from attaching a parent identity and then
                    // appending local-only text/tool state that the linked
                    // remote response might not contain.
                    efficiency.observe_calls(&self.tools, &tool_calls);
                    return Err(AgentError::ApiError(
                        "provider stream protocol violation: provider round id was not immediately followed by terminal Done"
                            .to_string(),
                    ));
                }
                // Time-to-first-token: elapsed from issuing the request to the
                // first content-bearing event of the turn. Always logged at debug;
                // surfaced as INFO only when the user opted into cache diagnostics
                // (same gate as cache-break diagnostics). Purely observational.
                if !first_token_logged
                    && matches!(
                        &event,
                        LlmEvent::TextDelta(_)
                            | LlmEvent::ThinkingDelta(_)
                            | LlmEvent::ToolUse { .. }
                            | LlmEvent::ToolUseDelta { .. }
                    )
                {
                    first_token_logged = true;
                    let ttft_ms = stream_start.elapsed().as_millis();
                    tracing::debug!(
                        target: "nomi_agent",
                        ttft_ms,
                        turn = turn + 1,
                        "first token received"
                    );
                    if self.compact_config.cache_diagnostics {
                        self.output
                            .emit_info(&format!("TTFT: {ttft_ms} ms (turn {})", turn + 1));
                    }
                }
                match event {
                    LlmEvent::TextDelta(text) => {
                        self.output.emit_text_delta(&text, &self.current_msg_id);
                        assistant_text.push_str(&text);
                    }
                    LlmEvent::ToolUse {
                        id,
                        name,
                        input,
                        extra,
                    } => {
                        if tool_calls.len() >= MAX_PROVIDER_TURN_TOOL_CALLS {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: provider turn exceeded the maximum of {MAX_PROVIDER_TURN_TOOL_CALLS} complete tool calls"
                            )));
                        }
                        if id.trim().is_empty() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool '{name}' has an empty tool_use_id"
                            )));
                        }
                        if id.trim() != id.as_str() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(
                                "provider stream protocol violation: tool_use_id has leading or trailing whitespace"
                                    .to_string(),
                            ));
                        }
                        if name.trim().is_empty() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool call '{id}' has an empty name"
                            )));
                        }
                        if name.trim() != name.as_str() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool name for call '{id}' has leading or trailing whitespace"
                            )));
                        }
                        if !tool_authority.advertises(&name) {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool '{name}' ({id}) was not advertised in this request"
                            )));
                        }
                        if let Some(preview_name) = previewed_tool_calls.get(&id)
                            && preview_name != &name
                        {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: completed tool call '{id}' changed its previewed name from '{preview_name}' to '{name}'"
                            )));
                        }
                        if tool_calls.iter().any(|call| {
                            matches!(call, ContentBlock::ToolUse { id: existing, .. } if existing == &id)
                        }) {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: duplicate tool_use_id '{id}' in one turn"
                            )));
                        }
                        if !input.is_object() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool '{name}' ({id}) arguments are not a JSON object"
                            )));
                        }
                        debug_assert!(input.is_object());
                        tracing::debug!(
                            target: "nomi_agent",
                            tool_use_id = %id,
                            tool = %name,
                            "provider tool call received"
                        );
                        // Recover only schema-directed nested values that a
                        // provider stringified, then keep that validated value as
                        // hooks, and dispatch. Whole-object strings were rejected
                        // above; unknown fields and invalid union branches remain
                        // strict validation failures.
                        let input = if tool_authority.is_deferred(&name) {
                            input
                        } else {
                            let original = input;
                            match self.tools.prepare_input(&name, original.clone()) {
                                Ok(prepared) => {
                                    if prepared != original {
                                        tracing::debug!(
                                            target: "nomi_agent",
                                            tool_use_id = %id,
                                            tool = %name,
                                            "provider tool call arguments normalized against schema"
                                        );
                                    }
                                    prepared
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        target: "nomi_agent",
                                        tool_use_id = %id,
                                        tool = %name,
                                        error = %error,
                                        "provider tool call failed local schema validation and will remain unpublished"
                                    );
                                    original
                                }
                            }
                        };
                        tool_calls.push(ContentBlock::ToolUse {
                            id,
                            name,
                            input,
                            extra,
                        });
                    }
                    LlmEvent::ToolUseDelta { id, name, input: _ } => {
                        // Tool progress is uncommitted provider data. Validate
                        // and reconcile its identity, but never publish a
                        // Running lifecycle until a complete ToolUse passes its
                        // full schema at the commit boundary.
                        if id.trim().is_empty() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool progress for '{name}' has an empty tool_use_id"
                            )));
                        }
                        if id.trim() != id.as_str() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(
                                "provider stream protocol violation: tool progress tool_use_id has leading or trailing whitespace"
                                    .to_string(),
                            ));
                        }
                        if name.trim().is_empty() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool progress '{id}' has an empty name"
                            )));
                        }
                        if name.trim() != name.as_str() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool progress name for call '{id}' has leading or trailing whitespace"
                            )));
                        }
                        if !tool_authority.advertises(&name) {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool progress '{name}' ({id}) was not advertised in this request"
                            )));
                        }
                        if let Some(preview_name) = previewed_tool_calls.get(&id) {
                            if preview_name != &name {
                                efficiency.observe_calls(&self.tools, &tool_calls);
                                return Err(AgentError::ApiError(format!(
                                    "provider stream protocol violation: tool progress '{id}' changed name from '{preview_name}' to '{name}'"
                                )));
                            }
                        } else {
                            if previewed_tool_calls.len() >= MAX_PROVIDER_TURN_TOOL_CALLS {
                                efficiency.observe_calls(&self.tools, &tool_calls);
                                return Err(AgentError::ApiError(format!(
                                    "provider stream protocol violation: provider turn exceeded the maximum of {MAX_PROVIDER_TURN_TOOL_CALLS} distinct tool previews"
                                )));
                            }
                            previewed_tool_calls.insert(id.clone(), name.clone());
                        }
                    }
                    LlmEvent::ThinkingDelta(text) => {
                        self.output.emit_thinking(&text, &self.current_msg_id);
                        thinking_text.push_str(&text);
                    }
                    LlmEvent::ToolUseTruncated {
                        id,
                        name,
                        argument_bytes,
                    } => {
                        saw_truncated_tool_use = true;
                        // NOT a tool call. It is never dispatched, never enters
                        // `tool_calls`, and is never schema-validated — the
                        // provider is reporting that its output ceiling cut this
                        // call off mid-stream. Recorded as a fact so the next
                        // round can tell the model what it was reaching for.
                        //
                        // Filtered against this request's authority: a truncated
                        // call naming a tool this request never advertised is
                        // provider noise, and rendering that name into the next
                        // attempt's prompt would state a falsehood.
                        if tool_authority.advertises(&name) {
                            let state_changing = self
                                .tools
                                .get(&name)
                                .is_some_and(|tool| round::is_state_changing(tool.category()));
                            truncated_calls.push(round::LedgerCutoff {
                                tool: name,
                                argument_bytes,
                                state_changing,
                            });
                        }
                        // The provider has declared this call will never
                        // complete, so an unreconciled preview for it is an
                        // expected outcome rather than the protocol anomaly the
                        // Done-time reconciliation exists to catch.
                        previewed_tool_calls.remove(&id);
                    }
                    LlmEvent::ThinkingSignature(signature) => {
                        thinking_signature = Some(signature);
                    }
                    LlmEvent::ProviderRoundId(round_id) => {
                        if !request.retain_provider_round {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(
                                "provider stream protocol violation: provider round id was emitted for a non-retainable request"
                                    .to_string(),
                            ));
                        }
                        if round_id.is_empty()
                            || round_id.trim() != round_id
                            || round_id.len() > MAX_PROVIDER_ROUND_ID_BYTES
                        {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(
                                "provider stream protocol violation: provider round id is empty, untrimmed, or oversized"
                                    .to_string(),
                            ));
                        }
                        if provider_round_id.replace(round_id).is_some() {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(
                                "provider stream protocol violation: provider emitted more than one round id"
                                    .to_string(),
                            ));
                        }
                    }
                    LlmEvent::Done {
                        stop_reason: sr,
                        usage,
                    } => {
                        if matches!(sr, StopReason::ToolUse | StopReason::EndTurn)
                            && let Some((preview_id, preview_name)) =
                                previewed_tool_calls.iter().find(|(preview_id, preview_name)| {
                                    !tool_calls.iter().any(|call| {
                                        matches!(
                                            call,
                                            ContentBlock::ToolUse { id, name, .. }
                                                if id == *preview_id && name == *preview_name
                                        )
                                    })
                                })
                        {
                            efficiency.observe_calls(&self.tools, &tool_calls);
                            return Err(AgentError::ApiError(format!(
                                "provider stream protocol violation: tool preview '{preview_id}' for '{preview_name}' had no matching completed ToolUse before Done"
                            )));
                        }
                        done_count += 1;
                        stop_reason = sr;
                        turn_usage = usage;
                    }
                    LlmEvent::Error(e) => {
                        efficiency.observe_calls(&self.tools, &tool_calls);
                        return Err(AgentError::ApiError(e));
                    }
                }
            }

            efficiency.observe_calls(&self.tools, &tool_calls);
            if done_count != 1 {
                return Err(AgentError::ApiError(format!(
                    "provider stream protocol violation: expected exactly one Done event, received {done_count}"
                )));
            }
            let terminal_shape_error = match stop_reason {
                StopReason::ToolUse if tool_calls.is_empty() => Some(
                    "provider stream protocol violation: ToolUse Done contained no complete tool calls",
                ),
                StopReason::EndTurn | StopReason::MaxTokens | StopReason::Refusal
                    if !tool_calls.is_empty() => Some(
                    "provider stream protocol violation: EndTurn/MaxTokens/Refusal Done contained tool calls",
                ),
                StopReason::MaxTurns => Some(
                    "provider stream protocol violation: provider emitted engine-only MaxTurns Done",
                ),
                _ => None,
            };
            if let Some(error) = terminal_shape_error {
                return Err(AgentError::ApiError(error.to_string()));
            }
            if saw_truncated_tool_use && stop_reason != StopReason::MaxTokens {
                return Err(AgentError::ApiError(
                    "provider stream protocol violation: truncated tool evidence requires MaxTokens Done"
                        .to_string(),
                ));
            }
            if provider_round_id.is_some()
                && (saw_truncated_tool_use
                    || stop_reason == StopReason::Refusal
                    || (stop_reason == StopReason::MaxTokens
                        && !previewed_tool_calls.is_empty()))
            {
                return Err(AgentError::ApiError(
                    "provider stream protocol violation: terminal response exposed an unsafe round id"
                        .to_string(),
                ));
            }

            // Assignment is performed once for every completed provider round,
            // including rounds with no calls, so stale candidates cannot cross
            // an intervening model response.
            let tool_call_contexts = tool_retry_tracker
                .assign(&tool_calls)
                .map_err(|id| {
                    AgentError::ApiError(format!(
                        "provider stream protocol violation: tool_use_id '{id}' was reused within one root user turn"
                    ))
                })?;
            let invalid_argument_call_ids: HashSet<String> = tool_calls
                .iter()
                .filter_map(|call| {
                    let ContentBlock::ToolUse {
                        id, name, input, ..
                    } = call
                    else {
                        return None;
                    };
                    (!tool_authority.is_deferred(name)
                        && self.tools.validate_input(name, input).is_err())
                    .then(|| id.clone())
                })
                .collect();

            // Done is the provider-turn commit point. Only now may complete,
            // authorized, non-deferred, schema-valid calls enter the frontend
            // Running lifecycle. Provider Error/EOF, terminal-shape failures,
            // and preview reconciliation failures therefore publish nothing.
            for call in &tool_calls {
                let ContentBlock::ToolUse {
                    id, name, input, ..
                } = call
                else {
                    continue;
                };
                if tool_authority.is_deferred(name)
                    || self.tools.validate_input(name, input).is_err()
                {
                    continue;
                }
                let input_str = serde_json::to_string(input).unwrap_or_default();
                let artifact_identity = self
                    .tools
                    .get(name)
                    .map(nomi_tools::Tool::artifact_identity)
                    .unwrap_or(name);
                let Some(context) = tool_call_contexts.get(id) else {
                    continue;
                };
                self.output.emit_tool_call_with_context(
                    id,
                    name,
                    artifact_identity,
                    &input_str,
                    context,
                );
            }

            self.total_usage.input_tokens += turn_usage.input_tokens;
            self.total_usage.output_tokens += turn_usage.output_tokens;
            self.total_usage.reasoning_tokens += turn_usage.reasoning_tokens;
            self.total_usage.cache_creation_tokens += turn_usage.cache_creation_tokens;
            self.total_usage.cache_read_tokens += turn_usage.cache_read_tokens;

            // Track per-turn input tokens for compaction watermark.
            // Use max(provider_reported, local_estimate) as a safety net:
            // some providers (e.g. DeepSeek with prefix caching) underreport
            // prompt_tokens, causing compaction to never trigger.
            let local_estimate = estimate::estimate_tokens_from_messages(&self.messages);
            let effective_watermark = turn_usage.input_tokens.max(local_estimate);

            if local_estimate > turn_usage.input_tokens
                && local_estimate.saturating_sub(turn_usage.input_tokens) > 10_000
            {
                self.output.emit_info(&format!(
                    "Token watermark override: provider={}, local_estimate={}, using={}",
                    turn_usage.input_tokens, local_estimate, effective_watermark
                ));
            }

            self.compact_state.last_input_tokens = effective_watermark;

            // A strict host-routed turn advertises a dedicated capability, not
            // an open-ended tool loop. One schema call can already request a
            // batch (for example image_gen.count); reject parallel or later
            // repeat calls before dispatch so a weak model cannot multiply
            // billable work and then leave the artifact ledger unrecoverable.
            if tool_allowlist.is_some() && !tool_calls.is_empty() {
                if routed_tool_calls_seen.saturating_add(tool_calls.len()) > 1 {
                    return Err(AgentError::ApiError(
                        "strictly routed turns permit exactly one tool call; use the tool's batch parameters instead of retrying or issuing parallel calls"
                            .to_owned(),
                    ));
                }
                routed_tool_calls_seen += tool_calls.len();
            }

            // Cache break detection
            let cache_stats = CacheStats {
                input_tokens: turn_usage.input_tokens,
                cache_read_tokens: turn_usage.cache_read_tokens,
                cache_creation_tokens: turn_usage.cache_creation_tokens,
            };
            if let Some(diagnostic) = self.cache_detector.check_response(cache_stats) {
                // A cache break is a diagnostic, not an error: surface it as INFO
                // only when the user opted into cache diagnostics. Never emit_error
                // here — a benign TTL expiry must not look like a failed turn to
                // the AutoWork runner.
                if let Some(msg) = cache_diagnostic_message(&diagnostic, self.compact_config.cache_diagnostics) {
                    self.output.emit_info(&msg);
                }
            }

            let mut assistant_content: Vec<ContentBlock> = Vec::new();
            if !thinking_text.is_empty() || thinking_signature.is_some() {
                assistant_content.push(ContentBlock::Thinking {
                    thinking: thinking_text,
                    signature: thinking_signature,
                });
            }
            if !assistant_text.is_empty() {
                assistant_content.push(ContentBlock::Text {
                    text: assistant_text.clone(),
                });
            }
            assistant_content.extend(tool_calls.clone());

            // A large pre-tool text block that this same round then wrote to a
            // file is the model composing tool arguments in the open, not an
            // answer. Only the copy that enters durable history is collapsed:
            // it has already streamed to the user verbatim, and the tool call
            // beside it still carries the full body.
            let draft_superseded = supersede_written_draft(&mut assistant_content);
            if draft_superseded {
                tracing::debug!(
                    target: "nomi_agent",
                    turn = turn + 1,
                    "collapsed a pre-tool draft superseded by a file write in the same round"
                );
            }

            let mut assistant_message = Message::now(Role::Assistant, assistant_content);
            if !draft_superseded {
                assistant_message.provider_round_id = provider_round_id;
            }
            self.messages.push(assistant_message);

            // Adopt this pass's cutoffs unconditionally, before any restart
            // decision. Recording them only on the restart path would drop the
            // final pass's cutoff (so the attempt-cap case under-reported what it
            // was reaching for), and recording them only when non-empty would let
            // a previous pass's cutoff persist into a later round's prompt as a
            // stale claim. Replacing every pass keeps "WHAT WAS CUT OFF" a
            // statement about the pass that just ended.
            round.ledger.set_cutoff(std::mem::take(&mut truncated_calls));

            if tool_calls.is_empty() {
                // Resumable round: the provider hit its output ceiling
                // mid-composition. Continuing a truncated draft is not
                // recoverable — it can end mid-token or mid-JSON, and asking a
                // model to continue such a string reliably produces a fresh
                // restatement instead of a completion. Restart the round against
                // the ORIGINAL requirement, carrying forward a ledger of what
                // machine-observably already happened.
                //
                // This runs FIRST inside the block, before `stagnation_guard`
                // is reset and before `safe_messages` is refreshed, because a
                // truncated pass is not a completed assistant response and the
                // three continuation hooks below all assume one. The placement
                // window is exact: after the steering drain further down, the
                // tail message would be a steering user message and `pop()`
                // would delete the wrong one; after the `safe_messages`
                // refresh, the rollback floor would still contain the truncated
                // draft.
                //
                // Reached only with `tool_calls` empty: a MaxTokens terminal
                // carrying complete tool calls is already a hard protocol error
                // above, before the assistant message was pushed.
                // The evidence must be about THE PASS THAT WAS TRUNCATED, not
                // about the turn. `cutoff` is exactly that: this pass streamed a
                // tool call and the ceiling cut it off mid-arguments, so its
                // arguments are unparseable and continuing the draft is provably
                // impossible while re-attempting is provably useful.
                //
                // Turn-lifetime evidence was deliberately rejected here. Gating
                // on "this turn already produced an effect" or "the model has an
                // unfinished plan" would restart a prose-only truncation that had
                // NOTHING in flight — burning two more full ceilings to
                // regenerate the same prose against the same wall, which is the
                // exact waste the deleted host loop caused. Worse, it would hand
                // that model a system section ordering it to open with a tool
                // call, inviting a completed Exec or Irreversible action to run
                // twice. A prose-only truncation is honestly reported as
                // retryable `MaxTokens` instead, which is a decision for the user
                // to spend budget on, not the engine.
                let restart = stop_reason == StopReason::MaxTokens
                    && round.attempt < round::MAX_ROUND_ATTEMPTS
                    && tools_advertised
                    && !round.ledger.cutoff.is_empty();
                if restart {
                    // The assistant message pushed just above is the tail, and
                    // nothing between that push and here mutates `self.messages`
                    // — so this removes exactly this pass's draft, independent of
                    // history length, of autocompaction, and of prior rounds.
                    // Only ever an assistant message, so every already-drained
                    // steering interjection stays in the transcript.
                    let dropped = self.messages.pop().expect(
                        "the assistant message pushed immediately above is still the tail",
                    );
                    debug_assert_eq!(dropped.role, Role::Assistant);
                    let dropped_draft_bytes = assistant_text.len();
                    round.begin_attempt();
                    // Keep exactly one live copy of each image on the wire: the
                    // re-pushed requirement below. Same call and same rationale
                    // as the abort path.
                    self.redact_user_images_since(0);
                    // Re-push the requirement only when it is not already the
                    // tail, so a first-pass truncation does not send the same
                    // request twice in a row.
                    //
                    // Compared AFTER redaction, which is what makes this correct
                    // for all three shapes. Text-only and still at the tail: the
                    // values match and nothing is appended. Multimodal: the
                    // history copy now holds placeholders while the requirement
                    // holds real images, so they differ and the live payload is
                    // restored at the tail. Tail is a tool result or a steering
                    // interjection: they differ, and the requirement is
                    // re-stated where the model will actually act on it.
                    //
                    // Compared through `serde_json::Value` because `ContentBlock`
                    // does not implement `PartialEq`. Only ever reached on a
                    // restart, so the cost is irrelevant.
                    let requirement_is_tail = self.messages.last().is_some_and(|tail| {
                        tail.role == Role::User
                            && serde_json::to_value(&tail.content).ok()
                                == serde_json::to_value(&round.requirement).ok()
                    });
                    if !requirement_is_tail {
                        self.messages
                            .push(Message::now(Role::User, round.requirement.clone()));
                    }
                    // A steer that arrived during the truncated pass is
                    // delivered on the very next pass. Draining is
                    // unconditionally safe here, unlike the gated drain below: a
                    // restart does not increment `turn`, so a provider pass is
                    // guaranteed to follow.
                    for text in self.drain_steering() {
                        let block = ContentBlock::Text { text };
                        completion_requirement.push(block.clone());
                        completion_context.requirement.push(block.clone());
                        self.messages.push(Message::now(Role::User, vec![block]));
                    }
                    completion_context
                        .ensure_target_baselines(
                            &self.workspace_root,
                            self.completion_evidence_mode,
                        )
                        .await;
                    // Fail-closed settlement of any tool card the truncated pass
                    // published but never completed, plus an explicit retraction
                    // boundary for every provisional text sink. `Start` remains
                    // non-destructive because host steering deliberately shares
                    // the current response bubble.
                    self.output.emit_current_attempt_output_discarded(
                        &self.current_msg_id,
                        u32::try_from(round.attempt)
                            .expect("round attempts are bounded below u32::MAX"),
                    );
                    // Round N's rollback floor: the requirement, not the draft.
                    *safe_messages = self.messages.clone();
                    self.save_session();
                    tracing::warn!(
                        target: "nomi_agent",
                        attempt = round.attempt,
                        max_attempts = round::MAX_ROUND_ATTEMPTS,
                        dropped_draft_bytes,
                        plan_steps = round.ledger.steps.len(),
                        effects_total = round.ledger.effects_total,
                        effects_ok = round.ledger.effects_ok_total,
                        cutoff = round.ledger.cutoff.len(),
                        "output ceiling reached; restarting the round against the original requirement"
                    );
                    // Deliberately does NOT increment `turn`: a round is a retry
                    // of this turn, not another tool-loop iteration, and
                    // model-only runtimes are pinned at max_turns = 1, where any
                    // `turn` increment would end the turn instead of retrying it.
                    continue;
                }

                self.stagnation_guard.reset();
                // The provider completed this assistant response. It is a safe
                // rollback point; any steering/goal continuation appended below
                // belongs to the *next* provider pass and must be dropped if that
                // pass fails.
                *safe_messages = self.messages.clone();
                // Steering interjection (point B): a user message injected
                // mid-turn extends a would-end turn instead of returning, so
                // the model incorporates it on the next step. Mirrors the
                // goal-continuation below; valid ordering (assistant→user).
                // Do not move steering into durable engine history when this
                // request has no provider pass left to process it. The host
                // owns terminal generation cleanup and will atomically absorb
                // the still-queued interjection. Appending it here would make
                // an unexecuted old-generation steer appear in the next
                // explicit user's provider request.
                let steered = if turn + 1 < limit {
                    self.drain_steering()
                } else {
                    Vec::new()
                };
                if !steered.is_empty() {
                    for text in steered {
                        let block = ContentBlock::Text { text };
                        completion_requirement.push(block.clone());
                        completion_context.requirement.push(block.clone());
                        self.messages.push(Message::now(Role::User, vec![block]));
                    }
                    completion_context
                        .ensure_target_baselines(
                            &self.workspace_root,
                            self.completion_evidence_mode,
                        )
                        .await;
                    self.save_session();
                    turn += 1;
                    continue;
                }

                // Goal-driven continuation hook (only fires for opt-in goal
                // sessions). Compute the continuation first so the immutable
                // borrow of `self.goal` ends before we mutate `self.messages`.
                let continuation = self.goal.as_ref().and_then(|g| g.maybe_continuation());
                if let Some(cont) = continuation {
                    self.messages.push(cont);
                    self.save_session();
                    turn += 1;
                    continue; // don't return — run another turn toward the goal
                }
                // Verification gate: an unsupported delivery claim gets one
                // corrective pass when budget permits. The typed issue is still
                // returned after that pass (or immediately for max_turns=1), so
                // repeating the claim can never become a successful terminal.
                let completion_adjudication = if stop_reason == StopReason::EndTurn {
                    completion_context.successful_mutation_observed |=
                        round.ledger.effects_ok_total > 0;
                    let exact_receipts = completion_context.terminal_exact_receipts.clone();
                    let supported_targets = completion_context
                        .supported_targets(
                            &assistant_text,
                            &self.workspace_root,
                            self.completion_evidence_mode,
                            &exact_receipts,
                        )
                        .await;
                    // Promote only terminally verified targets into the typed
                    // child/delegation channel. This closes the fork-Skill
                    // chain even when the child used Bash (its local delta +
                    // successful mutation were proven here), without changing
                    round
                        .ledger
                        .record_durable_effect_targets(supported_targets.iter().cloned());
                    completion_adjudication(
                        &assistant_text,
                        &completion_requirement,
                        &supported_targets,
                        &self.workspace_root,
                        self.completion_evidence_mode,
                    )
                } else {
                    None
                };
                if !completion_evidence_nudged
                    && turn + 1 < limit
                    && let Some(issue) = completion_adjudication.as_ref()
                {
                    tracing::warn!(
                        target: "nomi_agent",
                        turn = turn + 1,
                        issue = issue.kind(),
                        "required file mutation lacks durable verification — requesting one corrective pass"
                    );
                    completion_evidence_nudged = true;
                    completion_context.corrective_nudge_used = true;
                    let dropped = self.messages.pop().expect(
                        "the unsupported assistant completion is still the transcript tail",
                    );
                    debug_assert_eq!(dropped.role, Role::Assistant);
                    self.output.emit_current_attempt_output_discarded(
                        &self.current_msg_id,
                        u32::try_from(round.attempt)
                            .expect("round attempts are bounded below u32::MAX"),
                    );
                    // The discarded claim must not survive as a rollback floor,
                    // KB/distillation input, or the prefix of the corrected
                    // answer. Prior tool results remain intact.
                    *safe_messages = self.messages.clone();
                    self.messages.push(Message::now(
                        Role::User,
                        vec![ContentBlock::Text {
                            text: issue.nudge().to_owned(),
                        }],
                    ));
                    self.save_session();
                    turn += 1;
                    continue;
                }
                // The older spec-recheck heuristic remains a one-pass quality
                // nudge only. Its prose classifier is intentionally not a hard
                // terminal verdict; hard adjudication above is derived solely
                // from the trusted user requirement and exact durable targets.
                if completion_adjudication.is_none()
                    && !spec_recheck_nudged
                    && turn + 1 < limit
                    && unbacked_completion_claim(&assistant_text, &self.messages).is_some()
                {
                    spec_recheck_nudged = true;
                    let dropped = self
                        .messages
                        .pop()
                        .expect("the unsupported assistant completion is still the transcript tail");
                    debug_assert_eq!(dropped.role, Role::Assistant);
                    self.output.emit_current_attempt_output_discarded(
                        &self.current_msg_id,
                        u32::try_from(round.attempt)
                            .expect("round attempts are bounded below u32::MAX"),
                    );
                    *safe_messages = self.messages.clone();
                    self.messages.push(Message::now(
                        Role::User,
                        vec![ContentBlock::Text {
                            text: SPEC_RECHECK_NUDGE.to_owned(),
                        }],
                    ));
                    self.save_session();
                    turn += 1;
                    continue;
                }

                if completion_adjudication.is_some() {
                    // The typed result retains this text for diagnostics and
                    // usage accounting, but the live stream must match the
                    // accepted-turn transcript root restored by the outer
                    // wrapper. A host race-tail may have emitted an earlier
                    // provider pass, so retract the immutable turn checkpoint
                    // rather than only this final attempt.
                    self.output
                        .emit_accepted_turn_output_discarded(&self.current_msg_id);
                    // Do not persist the rejected assistant claim here. The
                    // outer accepted-turn wrapper owns one checked write of
                    // the captured root. If that write fails, the previous
                    // durable state can contain at most the already-admitted
                    // user request, never this unsupported completion claim.
                } else {
                    self.save_session();
                }
                return Ok(AgentResult {
                    text: assistant_text,
                    stop_reason,
                    usage: self.total_usage.clone(),
                    turns: turn + 1,
                    rounds: round.attempt,
                    effects_ok: round.ledger.effects_ok_total,
                    durable_effect_targets: round.ledger.durable_effect_targets.clone(),
                    cutoff_state_changing: round.ledger.cutoff_state_changing_total,
                    state_changing_tools_advertised,
                    completion_adjudication,
                });
            }

            let mut outcome = if let Some(writer) = self.protocol_writer.as_ref() {
                execute_tool_calls_with_protocol(
                    &self.tools,
                    &tool_calls,
                    &tool_authority,
                    writer,
                    &self.current_msg_id,
                    self.hooks.as_mut(),
                    self.compaction_level,
                    self.toon_enabled,
                )
                .await
                .expect("FullAuto protocol execution is infallible")
            } else {
                execute_tool_calls_scoped(
                    &self.tools,
                    &tool_calls,
                    &tool_authority,
                    &self.current_msg_id,
                    self.hooks.as_mut(),
                    self.compaction_level,
                    self.toon_enabled,
                )
                .await
                .expect("FullAuto tool execution is infallible")
            };
            let confirmed_invalid_argument_call_ids =
                confirmed_predispatch_schema_invalid_call_ids(
                    &invalid_argument_call_ids,
                    &outcome.results,
                );
            // Deliver binary outputs before success accounting or the next
            // model turn. A provider/tool RPC succeeding is not enough: when
            // the user-facing sink cannot persist the media, convert the tool
            // result into an error and remove the undeliverable in-memory image.
            // The handled error is returned to the model, while subsequent
            // passes in this accepted turn no longer advertise artifact tools.
            let mut artifact_delivery_succeeded = false;
            let mut delivered_exact_targets: HashMap<String, Vec<String>> = HashMap::new();
            for result in &mut outcome.results {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    images,
                } = result
                {
                    let tool_name = tool_calls
                        .iter()
                        .find_map(|call| {
                            if let ContentBlock::ToolUse { id, name, .. } = call
                                && id == tool_use_id
                            {
                                return Some(name.as_str());
                            }
                            None
                        })
                        .unwrap_or("unknown");
                    let artifact_identity = self
                        .tools
                        .get(tool_name)
                        .map(nomi_tools::Tool::artifact_identity)
                        .unwrap_or(tool_name);
                    let status = if *is_error { "error" } else { "completed" };
                    if tool_use_id.trim().is_empty() {
                        tracing::error!(
                            target: "nomi_agent",
                            tool = %tool_name,
                            status,
                            "tool result has empty tool_use_id"
                        );
                    } else {
                        tracing::debug!(
                            target: "nomi_agent",
                            tool_use_id = %tool_use_id,
                            tool = %tool_name,
                            status,
                            image_count = images.len(),
                            "tool result emitted"
                        );
                    }

                    match self
                        .output
                        .emit_tool_result_with_images_and_context(
                        tool_use_id,
                        tool_name,
                        artifact_identity,
                        *is_error,
                        content,
                        images,
                        tool_call_contexts.get(tool_use_id).expect(
                            "every committed ToolUse receives execution context",
                        ),
                    ) {
                        crate::output::ToolMediaDelivery::Unmanaged => {
                            // Diagnostic screenshots or other binary payloads
                            // returned by a failed tool are never valid model
                            // context for a completed artifact.  Sinks mark
                            // these as unmanaged because they intentionally do
                            // not persist them; discard the bytes here as well
                            // so an error cannot be replayed as if it were a
                            // generated image on the next provider turn.
                            if *is_error {
                                images.clear();
                            }
                        }
                        crate::output::ToolMediaDelivery::Delivered {
                            context,
                            durable_workspace_targets,
                        } => {
                            if !context.trim().is_empty() {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(&context);
                            }
                            // The sink has already persisted and verified these
                            // bytes and appended compact receipt/locator context.
                            // Never serialize the base64 payload into session
                            // history or resend it to the chat provider: that can
                            // multiply memory/token cost and make a successful
                            // generation fail only because the next text pass
                            // cannot accept a 40 MiB visual attachment.
                            images.clear();
                            if crate::output::artifact_contract(artifact_identity).is_some() {
                                artifact_delivery_succeeded = true;
                            }
                            let verified_targets = durable_workspace_targets
                                .iter()
                                .filter_map(|target| {
                                    workspace_evidence_file_target(
                                        target,
                                        &self.workspace_root,
                                        self.completion_evidence_mode,
                                    )
                                })
                                .collect::<Vec<_>>();
                            delivered_exact_targets
                                .insert(tool_use_id.clone(), verified_targets.clone());
                        }
                        crate::output::ToolMediaDelivery::Failed { error } => {
                            *is_error = true;
                            images.clear();
                            if !content.is_empty() {
                                content.push('\n');
                            }
                            content.push_str("Artifact delivery failed: ");
                            content.push_str(&error);
                        }
                    }
                    if *is_error && crate::output::artifact_contract(artifact_identity).is_some() {
                        artifact_retry_blocked = true;
                    }
                }
            }

            efficiency.observe_results(&outcome.results);
            // Round ledger (machine truth only). Runs AFTER the result loop
            // above, not inside it: that loop holds an immutable borrow of
            // `self.tools` through `artifact_identity` for its whole body, and
            // its own `find_map` binds only `id`/`name`, never `input`. Pairing
            // `&outcome.results` with `&tool_calls` here recovers the input and
            // also reads the `is_error` flag AFTER any undeliverable-media
            // downgrade, which is the more correct value.
            //
            // No third producer exists by design: nothing is scraped from
            // transcript prose (which is how a phantom tool call reached the
            // observed production trace), no summarization pass is run, and the
            // filesystem is never probed.
            debug_assert_eq!(outcome.results.len(), outcome.delegated_effects.len());
            for (result_index, result) in outcome.results.iter().enumerate() {
                let ContentBlock::ToolResult {
                    tool_use_id,
                    is_error,
                    ..
                } = result
                else {
                    continue;
                };
                let Some((name, input)) = tool_calls.iter().find_map(|call| match call {
                    ContentBlock::ToolUse {
                        id, name, input, ..
                    } if id == tool_use_id => Some((name, input)),
                    _ => None,
                }) else {
                    continue;
                };
                let Some(tool) = self.tools.get(name) else {
                    continue;
                };
                let invocation_may_mutate = tool.may_have_workspace_side_effects(input);
                if !*is_error && invocation_may_mutate {
                    completion_context.successful_mutation_observed = true;
                }
                let mut exact_targets = if *is_error {
                    Vec::new()
                } else {
                    outcome
                        .delegated_effects
                        .get(result_index)
                        .cloned()
                        .unwrap_or_default()
                };
                if !*is_error {
                    exact_targets.extend(
                        delivered_exact_targets
                            .get(tool_use_id)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                }
                if !*is_error && FILE_WRITE_TOOLS.contains(&name.as_str()) {
                    exact_targets.extend(durable_targets_for_tool(
                        name,
                        input,
                        &self.workspace_root,
                        self.completion_evidence_mode,
                    ));
                }
                exact_targets.sort();
                exact_targets.dedup();

                let deleted_targets = if *is_error {
                    Vec::new()
                } else {
                    deleted_targets_for_tool(
                        name,
                        input,
                        &self.workspace_root,
                        self.completion_evidence_mode,
                    )
                };
                apply_terminal_effect_evidence(
                    completion_context,
                    &mut round.ledger,
                    invocation_may_mutate,
                    *is_error,
                    exact_targets,
                    deleted_targets,
                );
                // Producer A: the model's own plan snapshot. Gated on success
                // because `update_plan` errors on invalid arguments and on an
                // empty plan, and neither may clobber a good ledger. Replaced
                // wholesale, never merged: the tool is stateless, so a merge
                // would resurrect steps the model deliberately dropped.
                if name == "update_plan" {
                    if !*is_error
                        && let Ok(args) = serde_json::from_value::<
                            nomi_tools::update_plan::UpdatePlanArgs,
                        >(input.clone())
                    {
                        round.ledger.replace_plan(
                            args.plan
                                .into_iter()
                                .map(|item| round::LedgerStep {
                                    step: item.step,
                                    status: item.status,
                                })
                                .collect(),
                        );
                    }
                    continue;
                }
                // Producer B: state-changing effects. A successful `Read` is not
                // progress; a `Write` or `Bash` is.
                //
                // Counted when EITHER the base category or this invocation's
                // category is state-changing, deliberately erring toward
                // "something happened". A multi-action tool (browser, computer)
                // reports Exec as its base category — so it is counted in
                // `state_changing_tools_advertised`, which has no input to judge
                // — while `category_for` can report Info for one read-only
                // invocation. Judging effects on `category_for` alone would let
                // such a tool arm the no-progress verdict without ever being
                // able to satisfy it. The rendered label still describes exactly
                // what ran, so the ledger stays honest either way.
                if name != "nomi_delegate"
                    && invocation_may_mutate
                {
                    round.ledger.push_effect(
                        name.clone(),
                        round::effect_label(&tool.describe(input)),
                        !*is_error,
                    );
                }
            }
            tool_retry_tracker.observe_invalid_arguments(
                &tool_calls,
                &tool_call_contexts,
                &confirmed_invalid_argument_call_ids,
            );
            let failed_call_ids: std::collections::HashSet<String> = outcome
                .results
                .iter()
                .filter_map(|result| match result {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        is_error: true,
                        ..
                    } => Some(tool_use_id.clone()),
                    _ => None,
                })
                .collect();
            // A successful invocation is exempt only when the tool explicitly
            // classifies this exact input as polling. Read-only is not polling:
            // repeating a deterministic observation or ToolSearch with the
            // same complete outcome is still a no-progress loop and must stop
            // after the corrective nudge. Failed polling remains tracked.
            let outcome_signature = crate::loop_guard::tool_outcome_signature_filtered(
                &tool_calls,
                &outcome.results,
                |id, name, input| match self.tools.get(name) {
                    Some(tool)
                        if tool.is_polling_invocation(input) && !failed_call_ids.contains(id) =>
                    {
                        false
                    }
                    Some(_) => true,
                    None => true,
                },
            );
            // Exact-signature tracking alone can be evaded by alternating two
            // different failing calls. Count all-failed tool turns separately;
            // assistant filler must not turn a failed tool turn into progress.
            // Any successful result resets this counter, and a user steer below
            // performs a full guard reset.
            let all_tool_results_failed =
                crate::loop_guard::all_tool_results_failed(&outcome.results);
            let mut stagnation_action = self
                .stagnation_guard
                .observe(outcome_signature, all_tool_results_failed);

            // Apply any context modifiers from skill executions before the next turn
            self.apply_context_modifiers(&outcome.modifiers);

            // A newly arrived user steer is material progress. Resolve it before
            // applying the action computed from the just-finished tool result,
            // otherwise a stale Abort decision would discard the steer.
            // A steer is consumable only if this exact request still has
            // authority for another provider pass. Leave it in the host queue
            // at the MaxTurns boundary so terminalization can discard it under
            // the lifecycle gate instead of leaking it through session
            // history into a successor explicit turn.
            let steered = if turn + 1 < limit {
                self.drain_steering()
            } else {
                Vec::new()
            };
            let had_steering = !steered.is_empty();
            if !steered.is_empty() {
                for text in &steered {
                    let block = ContentBlock::Text { text: text.clone() };
                    completion_requirement.push(block.clone());
                    completion_context.requirement.push(block);
                }
                completion_context
                    .ensure_target_baselines(
                        &self.workspace_root,
                        self.completion_evidence_mode,
                    )
                    .await;
                self.stagnation_guard.reset();
                tool_retry_tracker.clear();
                stagnation_action = crate::loop_guard::StagnationAction::Continue;
            }

            let mut tool_result_blocks = outcome.results;
            match stagnation_action {
                crate::loop_guard::StagnationAction::Continue => {}
                crate::loop_guard::StagnationAction::Nudge => {
                    tracing::warn!(
                        target: "nomi_agent",
                        "loop-stagnation guard fired after {STAGNATION_THRESHOLD} no-progress tool turns — injecting corrective nudge"
                    );
                    tool_result_blocks.push(ContentBlock::Text {
                        text: crate::loop_guard::STAGNATION_NUDGE.to_string(),
                    });
                }
                crate::loop_guard::StagnationAction::Abort => {
                    tracing::error!(
                        target: "nomi_agent",
                        "loop-stagnation guard aborted a no-progress tool cycle after the corrective nudge"
                    );
                    tool_result_blocks.push(ContentBlock::Text {
                        text: crate::loop_guard::STAGNATION_ABORT.to_string(),
                    });
                }
            }
            // Cost backstops. Both ride the same trailing-Text slot as the
            // stagnation nudge and fire at most once per turn: they are course
            // corrections, not limits, so repeating them every turn would only
            // add context to a turn already spending too much of it.
            if !tool_error_budget_nudged && efficiency.error_results >= TOOL_ERROR_BUDGET {
                tracing::warn!(
                    target: "nomi_agent",
                    tool_error_results = efficiency.error_results,
                    "tool error budget of {TOOL_ERROR_BUDGET} exhausted — injecting corrective nudge"
                );
                tool_error_budget_nudged = true;
                tool_result_blocks.push(ContentBlock::Text {
                    text: TOOL_ERROR_BUDGET_NUDGE.to_string(),
                });
            }
            if !batch_read_nudged
                && efficiency.lone_single_file_reads >= SINGLE_READ_NUDGE_THRESHOLD
            {
                tracing::info!(
                    target: "nomi_agent",
                    lone_single_file_reads = efficiency.lone_single_file_reads,
                    "single-file reads dominate this turn — injecting batching nudge"
                );
                batch_read_nudged = true;
                tool_result_blocks.push(ContentBlock::Text {
                    text: BATCH_READ_NUDGE.to_string(),
                });
            }
            // Steering interjection (point A): append any queued steer messages
            // as trailing Text blocks on THIS turn's tool-result message, so the
            // model sees them next turn without a second consecutive user
            // message. Mirrors the stagnation nudge above.
            for text in steered {
                tool_result_blocks.push(ContentBlock::Text { text });
            }
            self.messages
                .push(Message::now(Role::User, tool_result_blocks));
            self.prune_old_tool_images();

            // Save session after each turn
            *safe_messages = self.messages.clone();
            self.save_session();
            if tool_allowlist.is_some() && artifact_retry_blocked {
                // A strict artifact tool has already reached a terminal error.
                // Do not ask the provider for prose with an empty tool surface:
                // that pass can only fabricate success or repeat an unadvertised
                // call. End the engine phase and let the host's still-active
                // receipt requirement publish the single authoritative failure.
                return Ok(AgentResult {
                    text: String::new(),
                    stop_reason: StopReason::EndTurn,
                    usage: self.total_usage.clone(),
                    turns: turn + 1,
                    rounds: round.attempt,
                    effects_ok: round.ledger.effects_ok_total,
                    durable_effect_targets: round.ledger.durable_effect_targets.clone(),
                    cutoff_state_changing: round.ledger.cutoff_state_changing_total,
                    state_changing_tools_advertised,
                    completion_adjudication: None,
                });
            }
            if tool_allowlist.is_some() && artifact_delivery_succeeded && !had_steering {
                // A strict artifact route has no useful second model pass: its
                // authoritative user-facing result is the host-verified card,
                // and the routed schema is already closed after one call. End
                // the engine phase now so the manager can persist/reverify the
                // pending bytes immediately. This avoids a redundant provider
                // request turning a paid successful generation into a failed
                // turn before durable delivery.
                return Ok(AgentResult {
                    text: String::new(),
                    stop_reason: StopReason::EndTurn,
                    usage: self.total_usage.clone(),
                    turns: turn + 1,
                    rounds: round.attempt,
                    effects_ok: round.ledger.effects_ok_total,
                    durable_effect_targets: round.ledger.durable_effect_targets.clone(),
                    cutoff_state_changing: round.ledger.cutoff_state_changing_total,
                    state_changing_tools_advertised,
                    completion_adjudication: None,
                });
            }
            if stagnation_action == crate::loop_guard::StagnationAction::Abort {
                return Err(AgentError::Stagnation(
                    crate::loop_guard::STAGNATION_ABORT.to_string(),
                ));
            }
            turn += 1;
        }
    }

    /// Keep at most `max_recent_images` individual images, additionally bounded
    /// by the strictest supported provider request limit. The text part of each
    /// result is preserved.
    fn prune_old_tool_images(&mut self) {
        let mut remaining_count = self.max_recent_images.min(MAX_PROVIDER_REQUEST_IMAGES);
        let mut remaining_data_bytes = MAX_PROVIDER_REQUEST_IMAGE_DATA_BYTES;
        let mut changed = false;
        for msg in self.messages.iter_mut().rev() {
            for block in msg.content.iter_mut().rev() {
                if let ContentBlock::ToolResult {
                    content, images, ..
                } = block
                    && !images.is_empty()
                {
                    let original_len = images.len();
                    images.retain(|image| {
                        if remaining_count == 0 || image.data.len() > remaining_data_bytes {
                            return false;
                        }
                        remaining_count -= 1;
                        remaining_data_bytes -= image.data.len();
                        true
                    });
                    let retained = images.len();
                    let removed = original_len - retained;
                    if removed > 0 {
                        changed = true;
                        content.push_str(&format!(
                            "\n(Only the first {retained} image attachment(s) in this tool result remain; {removed} later attachment(s) were omitted by the recent-image/provider payload budget.)"
                        ));
                    }
                }
            }
        }
        if changed {
            clear_provider_round_ids(&mut self.messages);
        }
    }

    /// Provider failures terminate the current model pass. Historical visual
    /// observations are transport-heavy and can reproduce the same gateway
    /// failure on every retry/model switch, while their textual tool result is
    /// enough to tell the next model to capture a fresh view.
    fn strip_tool_images_after_provider_error(&mut self) {
        const NOTE: &str = "(Image attachment omitted after provider error recovery; capture a fresh observation if needed.)";
        let mut changed = false;
        for message in &mut self.messages {
            for block in &mut message.content {
                let ContentBlock::ToolResult { content, images, .. } = block else {
                    continue;
                };
                if images.is_empty() {
                    continue;
                }
                changed = true;
                images.clear();
                if !content.contains(NOTE) {
                    content.push('\n');
                    content.push_str(NOTE);
                }
            }
        }
        if changed {
            clear_provider_round_ids(&mut self.messages);
        }
    }

    /// Replace top-level user image blocks added by the current turn execution
    /// with one small marker per message. Nested tool-result images are owned by
    /// `prune_old_tool_images` and deliberately remain untouched.
    fn redact_user_images_since(&mut self, first_message: usize) -> bool {
        let mut changed = false;
        for message in self.messages.iter_mut().skip(first_message) {
            if message.role != Role::User
                || !message
                    .content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::Image { .. }))
            {
                continue;
            }

            let mut redacted = Vec::with_capacity(message.content.len());
            let mut marker_inserted = false;
            for block in std::mem::take(&mut message.content) {
                if matches!(block, ContentBlock::Image { .. }) {
                    changed = true;
                    if !marker_inserted {
                        redacted.push(ContentBlock::Text {
                            text: USER_IMAGE_HISTORY_PLACEHOLDER.to_owned(),
                        });
                        marker_inserted = true;
                    }
                } else {
                    redacted.push(block);
                }
            }
            message.content = redacted;
        }
        if changed {
            clear_provider_round_ids(&mut self.messages);
        }
        changed
    }

    /// Run the multi-level compaction pipeline before each API call.
    ///
    /// Execution order: microcompact → autocompact → emergency check.
    /// After a successful autocompact the emergency check is skipped
    /// because the context has been significantly reduced.
    async fn run_compaction(&mut self) -> Result<(), AgentError> {
        // 1. Microcompact (lightweight, no LLM call)
        if micro::should_microcompact(&self.messages, &self.compact_config) {
            let result = micro::microcompact(&mut self.messages, &self.compact_config);
            if result.cleared_count > 0 {
                clear_provider_round_ids(&mut self.messages);
                self.output.emit_info(&format!(
                    "Microcompact: cleared {} tool results (~{} tokens freed)",
                    result.cleared_count, result.estimated_tokens_freed
                ));
            }
        }

        // 2. Autocompact (LLM summarization)
        let mut compacted = false;
        let should_compact =
            auto::should_autocompact(self.compact_state.last_input_tokens, &self.compact_config);
        if should_compact {
            tracing::info!(target: "nomi_agent", last_input_tokens = self.compact_state.last_input_tokens, "context compaction triggered");
            if let Some(pct) = self.compact_config.autocompact_threshold_pct {
                self.output.emit_info(&format!(
                    "Autocompact threshold: {} tokens ({}% of {})",
                    auto::autocompact_threshold(&self.compact_config),
                    pct,
                    self.compact_config.context_window
                ));
            }
        }
        if should_compact && !self.compact_state.is_circuit_broken(&self.compact_config) {
            let provider = Arc::clone(&self.provider);
            match auto::autocompact(
                provider.as_ref(),
                &self.messages,
                &self.model,
                &self.compact_config,
                &mut self.compact_state,
            )
            .await
            {
                Ok(result) => {
                    self.output.emit_info(&format!(
                        "Autocompact: summarized {} messages ({} tokens → compact)",
                        result.messages_summarized, result.pre_compact_tokens
                    ));
                    self.messages = result.messages;
                    self.editable_turn = None;
                    compacted = true;
                }
                Err(auto::CompactError::CircuitBroken { .. }) => {
                    // Already tripped; logged at circuit-breaker level
                }
                Err(e) => {
                    self.output
                        .emit_warning(&format!("Autocompact failed: {}", e));
                }
            }
        } else if should_compact {
            self.output.emit_info(&format!(
                "Autocompact: skipped (circuit breaker tripped after {} consecutive failures, \
                 last_input_tokens={})",
                self.compact_state.consecutive_failures, self.compact_state.last_input_tokens
            ));
        } else if !self.compact_config.enabled {
            let threshold = auto::autocompact_threshold(&self.compact_config);
            if self.compact_state.last_input_tokens as usize >= threshold {
                self.output.emit_info(&format!(
                    "Autocompact: disabled (compact.enabled=false, \
                     last_input_tokens={}, threshold={})",
                    self.compact_state.last_input_tokens, threshold
                ));
            }
        }

        // 3. Emergency check (skip if autocompact just succeeded)
        if !compacted
            && emergency::is_at_emergency_limit(
                self.compact_state.last_input_tokens,
                &self.compact_config,
            )
        {
            return Err(AgentError::ContextTooLong {
                input_tokens: self.compact_state.last_input_tokens,
                limit: self
                    .compact_config
                    .context_window
                    .saturating_sub(self.compact_config.emergency_buffer),
            });
        }

        Ok(())
    }

    /// Run stop hooks when the agent session ends
    pub async fn run_stop_hooks(&self) {
        if let Some(hook_engine) = &self.hooks {
            let messages = hook_engine.run_stop().await;
            for msg in messages {
                tracing::info!(target: "nomi_agent", hook_message = %msg, "stop hook output");
            }
        }
    }

    /// Apply context modifiers collected from skill tool executions.
    fn apply_context_modifiers(&mut self, modifiers: &[Option<ContextModifier>]) {
        for modifier in modifiers.iter().flatten() {
            if let Some(ref model) = modifier.model {
                self.model = model.clone();
            }
            if let Some(effort) = modifier.effort {
                self.current_reasoning_effort = Some(effort_to_string(effort));
            }
            // Handle plan mode transitions
            if let Some(ref transition) = modifier.plan_mode_transition {
                match transition {
                    PlanModeTransition::Enter => {
                        self.plan_state.is_active = true;
                        if let Some(ref flag) = self.plan_active_flag {
                            flag.store(true, Ordering::Release);
                        }
                    }
                    PlanModeTransition::Exit { .. } => {
                        self.plan_state.is_active = false;
                        if let Some(ref flag) = self.plan_active_flag {
                            flag.store(false, Ordering::Release);
                        }
                    }
                }
            }
        }
    }

    fn sync_current_session_state(&mut self) {
        if let Some(session) = &mut self.current_session {
            session.messages = self.messages.clone();
            session.total_usage = self.total_usage.clone();
            session.activated_deferred_tools = self.tools.session_deferred_tool_identities();
            session.editable_turn = self.editable_turn.clone();
            session.host_context = self.host_context.clone();
            session.updated_at = chrono::Utc::now();
        }
    }

    fn try_save_session_document(&mut self) -> anyhow::Result<()> {
        self.sync_current_session_state();
        if let (Some(mgr), Some(session)) = (&self.session_manager, &self.current_session) {
            mgr.save(session)?;
        }
        Ok(())
    }

    fn try_save_session(&mut self) -> anyhow::Result<()> {
        self.try_save_session_document()?;
        if let (Some(mgr), Some(session)) = (&self.session_manager, &self.current_session) {
            mgr.update_index_for(session)?;
        }
        Ok(())
    }

    /// Capture the hidden in-memory authority this turn is allowed to mutate.
    ///
    /// See [`runtime_authority`] for the field-by-field audit, including
    /// intentionally excluded provider usage and external side effects.
    fn snapshot_runtime_authority(&self) -> AcceptedTurnRuntimeAuthority {
        AcceptedTurnRuntimeAuthority {
            model: self.model.clone(),
            thinking: self.thinking.clone(),
            current_reasoning_effort: self.current_reasoning_effort.clone(),
            compaction_level: self.compaction_level,
            compact_state: self.compact_state.clone(),
            hooks: self
                .hooks
                .as_ref()
                .map(|hooks| hooks.hooks_config().clone()),
            plan_state: self.plan_state.clone(),
            plan_active: self
                .plan_active_flag
                .as_ref()
                .map(|flag| flag.load(Ordering::Acquire)),
            goal: self.goal.as_ref().map(|goal| goal.snapshot_state()),
            cache_detector: self.cache_detector.clone(),
            stagnation_guard: self.stagnation_guard.clone(),
        }
    }

    /// Restore the accepted turn's captured runtime authority.
    ///
    /// The snapshot is captured in the same guarded step as the transcript root,
    /// so any context holding a root also holds the matching authority; a caller
    /// that has already matched a root cannot observe a missing snapshot here.
    /// Every write is an infallible in-memory assignment, which keeps the
    /// callers' checked session write the single failure gate: if that write
    /// fails the turn is reported as a state-consistency failure and the runtime
    /// is retired instead of reused.
    fn restore_runtime_authority(&mut self, completion_context: &CompletionEvidenceContext) {
        let Some(authority) = completion_context.turn_authority.clone() else {
            return;
        };
        self.model = authority.model;
        self.thinking = authority.thinking;
        self.current_reasoning_effort = authority.current_reasoning_effort;
        self.compaction_level = authority.compaction_level;
        self.compact_state = authority.compact_state;
        // Replace only the hook config. The supervised shell and its process
        // supervisor are live turn authority and must survive the rollback.
        if let (Some(hooks), Some(config)) = (self.hooks.as_mut(), authority.hooks) {
            hooks.replace_hooks_config(config);
        }
        self.plan_state = authority.plan_state;
        if let (Some(flag), Some(active)) = (&self.plan_active_flag, authority.plan_active) {
            flag.store(active, Ordering::Release);
        }
        // Write through the shared handle: `UpdateGoalTool` holds a clone of it.
        if let (Some(goal), Some(state)) = (self.goal.as_ref(), authority.goal) {
            goal.restore_state(state);
        }
        self.cache_detector = authority.cache_detector;
        self.stagnation_guard = authority.stagnation_guard;
    }

    fn persist_accepted_turn_root(
        &mut self,
        completion_context: &CompletionEvidenceContext,
    ) -> Result<(), AgentError> {
        if self.session_manager.is_none() || self.current_session.is_none() {
            return Ok(());
        }
        let root = completion_context.turn_root.as_ref().ok_or_else(|| {
            AgentError::ApiError("accepted turn is missing its completion root".to_owned())
        })?;
        match self
            .current_session
            .as_ref()
            .and_then(|session| session.accepted_turn_root.as_ref())
        {
            Some(persisted) if persisted.source_message_id == root.source_message_id => {
                return Ok(());
            }
            Some(_) => {
                return Err(AgentError::ApiError(
                    "a different accepted turn is still pending in this session".to_owned(),
                ));
            }
            None => {}
        }
        self.current_session
            .as_mut()
            .expect("checked above")
            .accepted_turn_root = Some(root.clone());
        if let Some(session) = &mut self.current_session {
            session.pending_host_terminal_root = None;
            session.last_interrupted_turn_source = None;
        }
        if let Err(error) = self.try_save_session_document() {
            // The session document is atomically replaced, so a failed write
            // left the old committed document authoritative. Revert the
            // in-memory marker as well and fail before any provider await.
            if let Some(session) = &mut self.current_session {
                session.accepted_turn_root = None;
            }
            return Err(AgentError::ApiError(format!(
                "failed to persist the accepted-turn recovery root: {error}"
            )));
        }
        Ok(())
    }

    fn save_session(&mut self) {
        if let Err(error) = self.try_save_session() {
            self.output
                .emit_warning(&format!("Failed to save session: {error}"));
        }
    }

    /// Stamp the owning-conversation token onto the current session and persist
    /// it. Idempotent (no-op when already equal, or when `token` is `None`).
    /// Called right after a session is created or resumed so the
    /// per-conversation-instance identity (see [`crate::session::Session::owner_token`])
    /// is written to disk — resume paths reject a stale session left by a prior
    /// conversation that reused this id.
    pub fn stamp_owner_token(&mut self, token: Option<String>) {
        let Some(token) = token else { return };
        let needs = match &self.current_session {
            Some(s) => s.owner_token.as_deref() != Some(token.as_str()),
            None => false,
        };
        if !needs {
            return;
        }
        if let Some(s) = &mut self.current_session {
            s.owner_token = Some(token);
        }
        self.save_session();
    }

    /// Clear the conversation context: drop all in-memory messages, reset
    /// compaction state and accumulated token usage, and persist the now-empty
    /// session so a process restart does not reload the old history.
    ///
    /// This is the engine-level primitive behind the backend "clear context"
    /// operation (mirrors the interactive `/clear` slash command, which mutates
    /// the same `messages` + `compact_state`). The session id is preserved so
    /// the conversation keeps its identity; only its contents are emptied.
    pub fn clear_context(&mut self) -> anyhow::Result<()> {
        let prior_messages = self.messages.clone();
        let prior_editable_turn = self.editable_turn.clone();
        let prior_host_context = self.host_context.clone();
        let prior_compact_state = self.compact_state.clone();
        let prior_total_usage = self.total_usage.clone();
        let prior_session = self.current_session.clone();
        self.messages.clear();
        self.editable_turn = None;
        self.host_context.clear();
        self.compact_state = CompactState::new();
        self.total_usage = TokenUsage::default();
        if let Some(session) = &mut self.current_session {
            // An explicit quiescent reset is authoritative over every
            // accepted-turn recovery phase. Keeping either root would retain
            // the cleared transcript (and could resurrect it on restart).
            session.accepted_turn_root = None;
            session.pending_host_terminal_root = None;
            session.last_interrupted_turn_source = None;
        }
        if let Err(error) = self.try_save_session() {
            self.messages = prior_messages;
            self.editable_turn = prior_editable_turn;
            self.host_context = prior_host_context;
            self.compact_state = prior_compact_state;
            self.total_usage = prior_total_usage;
            self.current_session = prior_session;
            let rollback = self.try_save_session();
            anyhow::bail!(match rollback {
                Ok(()) => format!(
                    "failed to persist the cleared Agent context; the prior session was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "failed to persist the cleared Agent context ({error}) and failed to restore the prior session ({rollback_error})"
                ),
            });
        }
        Ok(())
    }

    /// Validate that the exact durable user message still owns the latest
    /// rewind checkpoint. This is deliberately read-only so callers can fail
    /// before claiming a destructive edit receipt.
    pub fn can_rewind_last_turn(&self, expected_source_message_id: &str) -> bool {
        if let Some(checkpoint) = self.editable_turn.as_ref() {
            return !expected_source_message_id.is_empty()
                && checkpoint.source_message_id == expected_source_message_id
                && checkpoint.start_len <= self.messages.len();
        }

        // A prior fail-closed reset may leave no resumable transcript at all.
        // The Conversation service still validates the exact latest durable
        // user message, so an empty engine has nothing stale to truncate and a
        // no-op rewind is safe. Never infer a boundary for non-empty legacy
        // history from Role::User messages.
        !expected_source_message_id.is_empty() && self.messages.is_empty()
    }

    /// Rewind the engine transcript to the exact persisted root-turn boundary.
    ///
    /// The source id is checked again at mutation time so a stale preflight can
    /// never truncate a different turn.
    pub fn rewind_last_turn(
        &mut self,
        expected_source_message_id: &str,
    ) -> anyhow::Result<bool> {
        let Some(checkpoint) = self.editable_turn.as_ref().cloned() else {
            return Ok(!expected_source_message_id.is_empty() && self.messages.is_empty());
        };
        if checkpoint.source_message_id != expected_source_message_id
            || checkpoint.start_len > self.messages.len()
        {
            return Ok(false);
        }
        let prior_messages = self.messages.clone();
        let prior_editable_turn = self.editable_turn.clone();
        let prior_host_context = self.host_context.clone();
        let prior_session = self.current_session.clone();
        self.messages.truncate(checkpoint.start_len);
        self.host_context = checkpoint.prior_host_context;
        self.editable_turn = None;
        if let Err(error) = self.try_save_session() {
            self.messages = prior_messages;
            self.editable_turn = prior_editable_turn;
            self.host_context = prior_host_context;
            self.current_session = prior_session;
            let rollback = self.try_save_session();
            anyhow::bail!(match rollback {
                Ok(()) => format!(
                    "failed to persist the Agent turn rewind; the prior session was restored: {error}"
                ),
                Err(rollback_error) => format!(
                    "failed to persist the Agent turn rewind ({error}) and failed to restore the prior session ({rollback_error})"
                ),
            });
        }
        Ok(true)
    }

    /// Close a partially recorded turn after the host cancels execution.
    ///
    /// Providers in the Anthropic family require every assistant `tool_use` to
    /// be followed immediately by user `tool_result` blocks. If the host drops
    /// `run()` while tools are executing, the assistant `tool_use` message may
    /// already be in memory without its matching results. Add synthetic error
    /// results so the next request can safely reuse this history. The dropped
    /// `execute_turn_with_content()` future cannot execute its normal image-redaction
    /// wrapper, so this path also strips current-turn user image payloads.
    pub fn abort_current_turn(&mut self, reason: &str) {
        let pending_results: Vec<_> = self
            .messages
            .last()
            .filter(|message| message.role == Role::Assistant)
            .into_iter()
            .flat_map(|message| &message.content)
            .filter_map(|block| {
                let ContentBlock::ToolUse { id, name, .. } = block else {
                    return None;
                };
                Some((id.clone(), name.clone()))
            })
            .collect();

        let mut changed = false;
        if !pending_results.is_empty() {
            let result_blocks = pending_results
                .into_iter()
                .map(|(tool_use_id, name)| {
                    tracing::info!(
                        target: "nomi_agent",
                        tool_use_id = %tool_use_id,
                        tool = %name,
                        "closing pending tool_use after abort"
                    );
                    self.output
                        .emit_tool_result(&tool_use_id, &name, true, reason);
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content: reason.to_string(),
                        is_error: true,
                        images: Vec::new(),
                    }
                })
                .collect();
            self.messages.push(Message::now(Role::User, result_blocks));
            changed = true;
        }

        // Top-level user images are ephemeral transport payloads. Redact all of
        // them here rather than relying on the rewind anchor: compaction may
        // legitimately clear that anchor while a run is still in flight.
        changed |= self.redact_user_images_since(0);
        if changed {
            self.save_session();
        }
    }
}

impl Drop for AgentEngine {
    fn drop(&mut self) {
        let Some(supervisor) = self.process_supervisor.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = supervisor.shutdown().await;
            });
        } else {
            let _ = std::thread::Builder::new()
                .name("nomi-engine-process-cleanup".to_owned())
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    let _ = runtime.block_on(supervisor.shutdown());
                });
        }
    }
}

/// Injected once when a turn tries to end by claiming verified completion of
/// spec-driven work whose spec was never re-read after implementation began.
///
/// This targets the observed failure exactly. The model read README/QA_TASK in
/// messages 10-12, then wrote code for 57 more messages without ever looking at
/// the contract again; it invented its own CLI flags, wrote tests around the
/// inventions, and truthfully reported that those tests passed. An
/// exit-code check cannot catch that — `bun test` really did exit 0 — so the
/// gate asks for the one thing that was actually missing: a clause-by-clause
/// re-read of the spec before declaring the work deliverable.
pub(crate) const SPEC_RECHECK_NUDGE: &str = "Verification gate: you are reporting this work as \
complete, but you have not re-read the spec since you started editing files. Passing your own \
tests is not evidence that the contract is met — tests written from memory tend to encode what you \
built, not what was asked. Re-read the spec file(s) you were given now, list each required \
behavior, and for each one state the concrete evidence that it works (the exact command and its \
real output) or that it does not. If any clause is unmet or unverified, say so instead of \
reporting the work as deliverable.";

pub(crate) const STATE_CHANGE_EVIDENCE_NUDGE: &str = "Completion evidence gate: the accepted \
request requires a specific workspace file to be created or changed, but this turn has no durable \
evidence for that target. Use the file tools to perform and verify the requested change now. If it \
is genuinely blocked, report the blocker accurately; the host will keep the turn failed instead of \
recording an unfinished state-changing request as successful.";

/// Phrases that assert verified completion. Matched case-insensitively against
/// the final answer; Chinese is included because the observed false-green
/// summary was written in Chinese.
const COMPLETION_CLAIM_MARKERS: [&str; 14] = [
    "tests pass",
    "all pass",
    "0 fail",
    "ready to ship",
    "ready to deliver",
    "deliverable",
    "全部通过",
    "测试通过",
    "全部保留且通过",
    "可交付",
    "类型检查通过",
    "检查通过",
    "已完成",
    "交付总结",
];

/// Files that state requirements rather than implement them. A spec read is the
/// only reliable signal that the turn had an external contract to satisfy.
const SPEC_FILE_MARKERS: [&str; 6] = [
    "readme",
    "spec",
    "task",
    "requirement",
    "contract",
    "acceptance",
];

fn is_spec_path(path: &str) -> bool {
    let lowered = path.to_lowercase();
    let name = lowered
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(lowered.as_str());
    name.ends_with(".md") && SPEC_FILE_MARKERS.iter().any(|m| name.contains(m))
}

/// Every path a tool call touched, paired with whether it read a spec and
/// whether it mutated a file.
fn spec_and_write_positions(messages: &[Message]) -> (Vec<usize>, Option<usize>) {
    let mut spec_reads = Vec::new();
    let mut first_write = None;
    for (index, block) in messages
        .iter()
        .enumerate()
        .flat_map(|(i, m)| m.content.iter().map(move |b| (i, b)))
    {
        let ContentBlock::ToolUse { name, input, .. } = block else {
            continue;
        };
        let paths = input
            .get("file_path")
            .and_then(Value::as_str)
            .map(|p| vec![p.to_owned()])
            .or_else(|| {
                input.get("file_paths").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
            })
            .unwrap_or_default();

        if name == "Read" && paths.iter().any(|p| is_spec_path(p)) {
            spec_reads.push(index);
        }
        if FILE_WRITE_TOOLS.contains(&name.as_str()) {
            first_write.get_or_insert(index);
        }
    }
    (spec_reads, first_write)
}

/// Returns the corrective nudge when `answer` reports verified completion of
/// spec-driven work whose spec was not consulted after editing began.
///
/// Deliberately narrow, so it costs an extra turn only in the exact shape that
/// produced a false-green delivery: it requires an explicit completion claim, a
/// spec that was actually read, and file mutations that followed that read
/// without any later re-read. Ordinary answers, questions, honest "this is
/// unverified" reports, and read-only turns pass untouched.
pub(crate) fn unbacked_completion_claim(
    answer: &str,
    messages: &[Message],
) -> Option<&'static str> {
    if !positive_spec_delivery_assertion(answer) {
        return None;
    }
    let (spec_reads, first_write) = spec_and_write_positions(messages);
    let first_write = first_write?;
    if spec_reads.is_empty() || spec_reads.iter().any(|read| *read > first_write) {
        return None;
    }
    Some(SPEC_RECHECK_NUDGE)
}

const STATE_CHANGE_REQUIREMENT_MARKERS: &[&str] = &[
    "create",
    "write",
    "rewrite",
    "generate",
    "build",
    "implement",
    "fix",
    "edit",
    "modify",
    "update",
    "produce",
    "save",
    "add",
    "创建",
    "新建",
    "编写",
    "写入",
    "生成",
    "构建",
    "实现",
    "修复",
    "编辑",
    "修改",
    "更新",
    "产出",
    "保存",
    "添加",
];

const NON_MUTATING_REQUIREMENT_MARKERS: &[&str] = &[
    "how to",
    "explain",
    "describe",
    "summarize",
    "show",
    "read",
    "inspect",
    "analyze",
    "example",
    "review",
    "report whether",
    "confirm whether",
    "check whether",
    "whether",
    "did you",
    "have you",
    "what would",
    "would this",
    "will this",
    "should this",
    "review only",
    "read-only",
    "do not modify",
    "do not create",
    "don't modify",
    "don't create",
    "don't want you to",
    "do not want you to",
    "without modifying",
    "without creating",
    "如何",
    "怎么",
    "解释",
    "说明",
    "总结",
    "展示",
    "示例",
    "读取",
    "检查",
    "分析",
    "审查",
    "报告是否",
    "确认是否",
    "判断是否",
    "核实是否",
    "检查是否",
    "是否",
    "是否已",
    "是否会",
    "会不会",
    "有没有",
    "仅审查",
    "只读",
    "不要修改",
    "不要创建",
    "不要实际写入",
    "无需修改",
    "无需创建",
];

const STATE_CHANGE_CLAIM_MARKERS: &[&str] = &[
    "created",
    "written",
    "wrote",
    "generated",
    "built",
    "implemented",
    "fixed",
    "edited",
    "modified",
    "updated",
    "produced",
    "saved",
    "added",
    "已创建",
    "已新建",
    "已编写",
    "已写入",
    "已生成",
    "已构建",
    "已实现",
    "已修复",
    "已修改",
    "已更新",
    "已产出",
];

const NON_COMPLETION_CLAIM_MARKERS: &[&str] = &[
    "not created",
    "wasn't created",
    "isn't created",
    "did not create",
    "didn't create",
    "failed to create",
    "not fixed",
    "not modified",
    "not written",
    "not generated",
    "not implemented",
    "not complete",
    "not deliverable",
    "not all tests pass",
    "still missing",
    "is missing",
    "does not exist",
    "doesn't exist",
    "would create",
    "could create",
    "will create",
    "should create",
    "would modify",
    "could modify",
    "will modify",
    "going to",
    "plan to",
    "next",
    "no file",
    "couldn't",
    "could not",
    "can't",
    "cannot",
    "unable",
    "blocked",
    "permission denied",
    "deleted",
    "removed",
    "rolled back",
    "the log says",
    "log says",
    "previous answer",
    "previous response",
    "previously claimed",
    "previous run",
    "prior run",
    "last turn",
    "last session",
    "before this turn",
    "earlier",
    "was false",
    "is false",
    "false claim",
    "model's final answer",
    "model final answer",
    "the answer says",
    "the response says",
    "claimed",
    "would",
    "could",
    "will",
    "should",
    "未创建",
    "没有创建",
    "并未创建",
    "尚未创建",
    "创建失败",
    "未修复",
    "没有修复",
    "并未修复",
    "尚未修复",
    "未写入",
    "未生成",
    "未实现",
    "未完成",
    "尚未完成",
    "不可交付",
    "并非全部通过",
    "仍不存在",
    "不存在",
    "文件缺失",
    "将创建",
    "可以创建",
    "会创建",
    "应创建",
    "将修改",
    "可以修改",
    "无法创建",
    "不能创建",
    "被阻塞",
    "权限不足",
    "已删除",
    "删除了",
    "已回滚",
    "日志声称",
    "日志显示",
    "上一轮声称",
    "之前声称",
    "上一轮",
    "上一会话",
    "之前",
    "此前",
    "先前",
    "上次",
    "声称",
    "错误",
    "不实",
    "计划",
    "准备",
    "是否",
];

fn clauses(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(|character| {
        matches!(
            character,
            '\n' | ';' | '；' | '。' | '!' | '！' | '?' | '？'
        )
    })
    .map(str::trim)
    .filter(|clause| !clause.is_empty())
}

fn contains_marker(text: &str, marker: &str) -> bool {
    if !marker.is_ascii() {
        return text.contains(marker);
    }
    text.match_indices(marker).any(|(start, _)| {
        let end = start + marker.len();
        let before_is_word = text[..start]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        let after_is_word = text[end..]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        !before_is_word && !after_is_word
    })
}

fn contains_any_marker(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| contains_marker(text, marker))
}

fn positive_spec_delivery_assertion(answer: &str) -> bool {
    clauses(answer).any(|clause| {
        let clause = clause.to_lowercase();
        contains_any_marker(&clause, &COMPLETION_CLAIM_MARKERS)
            && !contains_any_marker(&clause, NON_COMPLETION_CLAIM_MARKERS)
            && !clause.contains(['?', '？', '"', '“', '”'])
    })
}

fn normalized_file_target(raw: &str, mode: CompletionEvidenceMode) -> Option<String> {
    if raw.contains("://") {
        return None;
    }
    let candidate = raw.trim_matches(|character: char| {
            matches!(
                character,
                '`' | '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
                    | ':' | '!' | '?' | '。' | '，' | '；' | '：' | '！' | '？'
            )
        });
    let candidate = candidate.trim_end_matches('.');
    if mode == CompletionEvidenceMode::RemoteExactReceipts && candidate.contains('\\') {
        // The SSH backend does not expose the remote OS/path rules. Restrict
        // hard adjudication to unambiguous POSIX-style relative identifiers
        // rather than applying the desktop host's Windows semantics.
        return None;
    }
    let normalized = if mode == CompletionEvidenceMode::LocalFingerprint {
        candidate.replace('\\', "/")
    } else {
        candidate.to_owned()
    };
    let mut components = Vec::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            component => components.push(component),
        }
    }
    let normalized = components.join("/");
    let leaf = normalized.rsplit('/').next().unwrap_or(&normalized);
    const HIGH_CONFIDENCE_BASENAMES: &[&str] = &[
        ".env",
        ".gitignore",
        ".gitattributes",
        ".editorconfig",
        ".npmrc",
        ".nvmrc",
        "Dockerfile",
        "Makefile",
        "Justfile",
        "Procfile",
        "LICENSE",
    ];
    let conventional_basename = HIGH_CONFIDENCE_BASENAMES.iter().any(|known| {
        if cfg!(windows) {
            leaf.eq_ignore_ascii_case(known)
        } else {
            leaf == *known
        }
    });
    if conventional_basename {
        return Some(normalized);
    }
    let (stem, extension) = leaf.rsplit_once('.')?;
    const HIGH_CONFIDENCE_EXTENSIONS: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "json", "html", "htm", "css", "scss", "sass",
        "less", "vue", "svelte", "md", "markdown", "txt", "toml", "yaml", "yml", "xml",
        "csv", "sql", "py", "go", "java", "kt", "kts", "c", "h", "cc", "cpp", "hpp",
        "cs", "rb", "php", "swift", "sh", "ps1", "bat", "cmd", "lock", "env", "png",
        "jpg", "jpeg", "webp", "gif", "svg", "pdf", "zip", "docx", "xlsx", "pptx", "mp3",
        "wav", "ogg", "mp4", "webm", "mov",
    ];
    if stem.is_empty()
        || extension.is_empty()
        || extension.len() > 12
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        || !HIGH_CONFIDENCE_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    {
        return None;
    }
    Some(normalized)
}

fn workspace_relative_file_target(
    raw: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Option<String> {
    let trimmed = raw.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';'
                | ':' | '!' | '?' | '。' | '，' | '；' | '：' | '！' | '？'
        )
    });
    if mode == CompletionEvidenceMode::RemoteExactReceipts {
        if let Some(home_relative) = trimmed.strip_prefix("~/") {
            let normalized = normalized_file_target(home_relative, mode)?;
            return Some(format!("remote-home:{normalized}"));
        }
        if let Some(posix) = trimmed.strip_prefix('/') {
            if posix.is_empty() || trimmed.contains(['\\', ':']) {
                return None;
            }
            let normalized = normalized_file_target(posix, mode)?;
            return Some(format!("remote-posix:/{normalized}"));
        }
        let path = std::path::Path::new(trimmed);
        if trimmed.starts_with('\\')
            || trimmed.contains([':', '\\'])
            || path.has_root()
            || path.is_absolute()
        {
            return None;
        }
        return normalized_file_target(trimmed, mode).map(|target| format!("remote-rel:{target}"));
    }
    if trimmed.starts_with("~/") {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    if !path.is_absolute() {
        return normalized_file_target(trimmed, mode);
    }

    #[cfg(not(windows))]
    fn lexical_key(path: &std::path::Path) -> Option<String> {
        let mut parts = Vec::new();
        for component in dunce::simplified(path)
            .to_string_lossy()
            .replace('\\', "/")
            .split('/')
        {
            match component {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                component => parts.push(component.to_owned()),
            }
        }
        let path = parts.join("/");
        Some(path)
    }

    let root = if workspace_root.is_absolute() {
        workspace_root.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(workspace_root)
    };
    #[cfg(windows)]
    {
        let root = dunce::canonicalize(root).ok()?;
        let absolute = resolve_workspace_target(&root, trimmed)?;
        let relative = absolute.strip_prefix(&root).ok()?;
        return normalized_file_target(&relative.to_string_lossy(), mode);
    }
    #[cfg(not(windows))]
    {
        let root = lexical_key(&root)?;
        let absolute = lexical_key(path)?;
        let prefix = format!("{}/", root.trim_end_matches('/'));
        let relative = absolute.strip_prefix(&prefix)?;
        normalized_file_target(relative, mode)
    }
}

fn resolve_workspace_target(
    workspace_root: &std::path::Path,
    target: &str,
) -> Option<std::path::PathBuf> {
    let root = dunce::canonicalize(workspace_root).ok()?;
    if !std::fs::metadata(&root).ok()?.is_dir() {
        return None;
    }
    let candidate = root.join(target);
    let mut ancestor = candidate.as_path();
    let mut suffix = Vec::new();
    let canonical_ancestor = loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break dunce::canonicalize(ancestor).ok()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(ancestor.file_name()?.to_os_string());
                ancestor = ancestor.parent()?;
            }
            Err(_) => return None,
        }
    };
    if !canonical_ancestor.starts_with(&root) {
        return None;
    }
    let mut resolved = canonical_ancestor;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    resolved.starts_with(&root).then_some(resolved)
}

fn metadata_modified_nanos(metadata: &std::fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

fn metadata_refers_to_open_file(file: &std::fs::File, path: &std::path::Path) -> bool {
    let Ok(cloned) = file.try_clone() else {
        return false;
    };
    let Ok(open_handle) = same_file::Handle::from_file(cloned) else {
        return false;
    };
    let Ok(path_handle) = same_file::Handle::from_path(path) else {
        return false;
    };
    open_handle == path_handle
}

fn completion_target_fingerprint(
    workspace_root: &std::path::Path,
    target: &str,
    remaining_hash_bytes: u64,
) -> (CompletionFingerprintProbe, u64) {
    let Some(root) = dunce::canonicalize(workspace_root).ok() else {
        return (CompletionFingerprintProbe::Unavailable, 0);
    };
    let Some(path) = resolve_workspace_target(workspace_root, target) else {
        return (CompletionFingerprintProbe::Unsafe, 0);
    };
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (
                CompletionFingerprintProbe::State(CompletionTargetFingerprint::Absent),
                0,
            );
        }
        Err(_) => return (CompletionFingerprintProbe::Unavailable, 0),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return (CompletionFingerprintProbe::Unsafe, 0);
    }
    let Some(canonical) = dunce::canonicalize(&path).ok() else {
        return (CompletionFingerprintProbe::Unavailable, 0);
    };
    if !canonical.starts_with(root) {
        return (CompletionFingerprintProbe::Unsafe, 0);
    }
    let Some(mut file) = std::fs::File::open(&canonical).ok() else {
        return (CompletionFingerprintProbe::Unavailable, 0);
    };
    let Some(before) = file.metadata().ok() else {
        return (CompletionFingerprintProbe::Unavailable, 0);
    };
    if !before.is_file() {
        return (CompletionFingerprintProbe::Unsafe, 0);
    }
    let modified_nanos = metadata_modified_nanos(&before);
    let hash_allowed = before.len() <= MAX_COMPLETION_EVIDENCE_FILE_BYTES
        && before.len() <= remaining_hash_bytes;
    if !hash_allowed {
        // Metadata/mtime cannot prove content identity: Write(B) followed by a
        // Bash restore(A) leaves a changed mtime while the final bytes equal the
        // baseline. Stay fail-closed when the bounded hash cannot be taken.
        return (CompletionFingerprintProbe::ResourceLimited, 0);
    }
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let Some(read) = std::io::Read::read(&mut file, &mut buffer).ok() else {
            return (CompletionFingerprintProbe::Unavailable, 0);
        };
        if read == 0 {
            break;
        }
        let Some(next_total) = total.checked_add(read as u64) else {
            return (CompletionFingerprintProbe::Unavailable, 0);
        };
        total = next_total;
        if total > MAX_COMPLETION_EVIDENCE_FILE_BYTES {
            return (CompletionFingerprintProbe::Unavailable, 0);
        }
        hasher.update(&buffer[..read]);
    }
    let Some(handle_after) = file.metadata().ok() else {
        return (CompletionFingerprintProbe::Unavailable, 0);
    };
    let path_after = std::fs::metadata(&path).ok();
    let canonical_after = dunce::canonicalize(&path).ok();
    let stable = handle_after.is_file()
        && handle_after.len() == before.len()
        && metadata_modified_nanos(&handle_after) == modified_nanos
        && total == before.len()
        && canonical_after.as_ref() == Some(&canonical)
        && metadata_refers_to_open_file(&file, &path)
        && path_after.as_ref().is_some_and(|after| {
            after.is_file()
                && after.len() == before.len()
                && metadata_modified_nanos(after) == modified_nanos
        });
    if !stable {
        return (CompletionFingerprintProbe::Unavailable, 0);
    }
    (
        CompletionFingerprintProbe::State(CompletionTargetFingerprint::Present {
            size_bytes: total,
            modified_nanos,
            sha256: Some(hasher.finalize().into()),
        }),
        total,
    )
}

fn completion_target_fingerprints(
    workspace_root: &std::path::Path,
    targets: Vec<String>,
    hash_budget: u64,
) -> Vec<(String, CompletionFingerprintProbe, u64)> {
    let mut hashed = 0u64;
    targets
        .into_iter()
        .map(|target| {
            let remaining = hash_budget.saturating_sub(hashed);
            let (probe, bytes) =
                completion_target_fingerprint(workspace_root, &target, remaining);
            hashed = hashed.saturating_add(bytes);
            (target, probe, bytes)
        })
        .collect()
}

fn workspace_evidence_file_target(
    raw: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Option<String> {
    let target = workspace_relative_file_target(raw, workspace_root, mode)?;
    if mode == CompletionEvidenceMode::LocalFingerprint {
        resolve_workspace_target(workspace_root, &target)?;
        let lexical = workspace_root.join(&target);
        if std::fs::symlink_metadata(lexical)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return None;
        }
    }
    Some(target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetSpan {
    start: usize,
    end: usize,
    normalized: String,
}

/// Extract targets that are explicitly delimited by inline code or a Markdown
/// link label, and return every range the ordinary ASCII scanner must ignore.
///
/// Treating the delimited value as one token is load-bearing for paths with
/// spaces: `` `my app.html` `` must never degrade into the unrelated suffix
/// `app.html`. Markdown destinations are never task targets (and may be URLs).
fn delimited_targets_and_exclusions(
    text: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> (Vec<TargetSpan>, Vec<(usize, usize)>) {
    let mut spans = Vec::new();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while let Some(start_rel) = text[cursor..].find("```") {
        let start = cursor + start_rel;
        let content_start = start + 3;
        let Some(end_rel) = text[content_start..].find("```") else {
            ranges.push((start, text.len()));
            break;
        };
        let end = content_start + end_rel + 3;
        ranges.push((start, end));
        cursor = end;
    }

    cursor = 0;
    while let Some(start_rel) = text[cursor..].find('`') {
        let start = cursor + start_rel;
        if ranges
            .iter()
            .any(|(excluded_start, excluded_end)| start >= *excluded_start && start < *excluded_end)
        {
            cursor = start + 1;
            continue;
        }
        let content_start = start + 1;
        let Some(end_rel) = text[content_start..].find('`') else {
            ranges.push((start, text.len()));
            break;
        };
        let content_end = content_start + end_rel;
        let end = content_end + 1;
        ranges.push((start, end));
        if let Some(normalized) =
            workspace_relative_file_target(
                &text[content_start..content_end],
                workspace_root,
                mode,
            )
        {
            spans.push(TargetSpan {
                start: content_start,
                end: content_end,
                normalized,
            });
        }
        cursor = end;
    }

    cursor = 0;
    while let Some(open_rel) = text[cursor..].find('[') {
        let open = cursor + open_rel;
        let label_start = open + 1;
        let Some(label_end_rel) = text[label_start..].find("](") else {
            break;
        };
        let label_end = label_start + label_end_rel;
        let destination_start = label_end + 2;
        let Some(destination_end_rel) = text[destination_start..].find(')') else {
            break;
        };
        let end = destination_start + destination_end_rel + 1;
        ranges.push((open, end));
        if !ranges.iter().any(|(excluded_start, excluded_end)| {
            *excluded_start < open && open < *excluded_end
        }) && let Some(normalized) =
            workspace_relative_file_target(&text[label_start..label_end], workspace_root, mode)
        {
            spans.push(TargetSpan {
                start: label_start,
                // The destination is excluded from identity, but belongs to
                // the syntactic target token for suffix/claim boundaries.
                end,
                normalized,
            });
        }
        cursor = end;
    }
    (spans, ranges)
}

fn lex_targets(
    text: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<TargetSpan> {
    let (mut spans, excluded) =
        delimited_targets_and_exclusions(text, workspace_root, mode);
    let mut run_start = None;
    let mut finish_run = |start: usize, end: usize| {
        if excluded
            .iter()
            .any(|(excluded_start, excluded_end)| start >= *excluded_start && start < *excluded_end)
        {
            return;
        }
        let token_end = start
            + text[start..end]
                .trim_end_matches(['?', '!'])
                .len();
        if token_end == start {
            return;
        }
        let raw = &text[start..token_end];
        let lowered = raw.to_ascii_lowercase();
        if lowered.starts_with("www.")
            || raw.contains("://")
            || raw.contains(['?', '#', '@'])
        {
            return;
        }
        if let Some(normalized) = workspace_relative_file_target(raw, workspace_root, mode) {
            spans.push(TargetSpan {
                start,
                end: token_end,
                normalized,
            });
        }
    };
    for (index, character) in text.char_indices() {
        let allowed = character.is_ascii_alphanumeric()
            || matches!(
                character,
                '_' | '-' | '.' | '/' | '\\' | ':' | '?' | '#' | '@' | '~'
            );
        match (run_start, allowed) {
            (None, true) => run_start = Some(index),
            (Some(start), false) => {
                finish_run(start, index);
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        finish_run(start, text.len());
    }
    spans.sort_by_key(|span| span.start);
    spans.dedup_by(|left, right| left.start == right.start && left.end == right.end);
    spans
}

pub(crate) fn durable_targets_for_tool(
    name: &str,
    input: &Value,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<String> {
    match name {
        "Write" | "Edit" => input
            .get("file_path")
            .and_then(Value::as_str)
            .and_then(|path| workspace_evidence_file_target(path, workspace_root, mode))
            .into_iter()
            .collect(),
        "ApplyPatch" => input
            .get("files")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|file| !file.get("delete").and_then(Value::as_bool).unwrap_or(false))
            .filter_map(|file| file.get("file_path").and_then(Value::as_str))
            .filter_map(|path| workspace_evidence_file_target(path, workspace_root, mode))
            .collect(),
        _ => Vec::new(),
    }
}

fn deleted_targets_for_tool(
    name: &str,
    input: &Value,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<String> {
    if name != "ApplyPatch" {
        return Vec::new();
    }
    input
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|file| file.get("delete").and_then(Value::as_bool) == Some(true))
        .filter_map(|file| file.get("file_path").and_then(Value::as_str))
        .filter_map(|path| workspace_evidence_file_target(path, workspace_root, mode))
        .collect()
}

fn marker_spans(text: &str, markers: &[&str]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for marker in markers {
        for (start, _) in text.match_indices(marker) {
            let end = start + marker.len();
            let bounded = !marker.is_ascii()
                || (!text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|character| {
                        character.is_ascii_alphanumeric() || character == '_'
                    })
                    && !text[end..].chars().next().is_some_and(|character| {
                        character.is_ascii_alphanumeric() || character == '_'
                    }));
            if bounded {
                spans.push((start, end));
            }
        }
    }
    spans.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| (right.1 - right.0).cmp(&(left.1 - left.0)))
    });
    let mut disjoint = Vec::new();
    for span in spans {
        if disjoint
            .last()
            .is_some_and(|(_, previous_end)| span.0 < *previous_end)
        {
            continue;
        }
        disjoint.push(span);
    }
    disjoint
}

/// Lowercase the ASCII command vocabulary without changing UTF-8 byte
/// offsets. Classifier spans are measured against the original text, so full
/// Unicode case folding (for example `İ` -> `i` + combining dot) cannot be
/// used before slicing with those offsets.
fn ascii_lowercase_preserving_offsets(text: &str) -> String {
    let mut lowered = text.to_owned();
    lowered.make_ascii_lowercase();
    lowered
}

fn clause_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let ascii_sentence_period = character == '.'
            && text[index + character.len_utf8()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace);
        if ascii_sentence_period
            || matches!(
                character,
                '\n' | ';' | '；' | '。' | '!' | '！' | '?' | '？'
            )
        {
            let end = index + character.len_utf8();
            if !text[start..end].trim().is_empty() {
                ranges.push((start, end));
            }
            start = end;
        }
    }
    if start < text.len() && !text[start..].trim().is_empty() {
        ranges.push((start, text.len()));
    }
    ranges
}

fn directive_prefix_is_allowed(prefix: &str) -> bool {
    let prefix = prefix
        .trim_matches(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .to_lowercase();
    if prefix.is_empty() {
        return true;
    }
    const SEQUENCE_ANCHORS: &[&str] = &["and", "and then", "then", "先", "然后", "再"];
    if SEQUENCE_ANCHORS
        .iter()
        .any(|anchor| prefix.trim_end().ends_with(anchor))
    {
        return true;
    }
    const ENGLISH_POLITE_PREFIXES: &[&str] = &[
        "please",
        "kindly",
        "can you",
        "could you",
        "would you",
        "will you",
        "i need you to",
        "i want you to",
        "we need you to",
        "you need to",
        "help me",
        "help me to",
    ];
    if ENGLISH_POLITE_PREFIXES.contains(&prefix.as_str()) {
        return true;
    }
    const CHINESE_POLITE_PREFIXES: &[&str] = &[
        "请", "请你", "请帮我", "可以帮我", "能否", "能否帮我", "能不能", "麻烦", "麻烦你",
        "帮我", "需要你", "我需要你", "我要你", "直接",
    ];
    if CHINESE_POLITE_PREFIXES.contains(&prefix.as_str()) {
        return true;
    }

    // A comma is not itself command authority: prose such as "the log says,
    // create x failed" must remain history. Only an explicit local directive
    // anchor after the last comma starts a new imperative segment.
    let local = prefix
        .rsplit([',', '，'])
        .next()
        .unwrap_or(&prefix)
        .trim();
    SEQUENCE_ANCHORS.contains(&local)
        || ENGLISH_POLITE_PREFIXES.contains(&local)
        || CHINESE_POLITE_PREFIXES.contains(&local)
}

fn claim_prefix_is_allowed(prefix: &str) -> bool {
    let prefix = prefix
        .trim_matches(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .to_lowercase();
    matches!(
        prefix.as_str(),
        "" | "i"
            | "i've"
            | "i have"
            | "we"
            | "we've"
            | "we have"
            | "successfully"
            | "now"
            | "done"
            | "我"
            | "我们"
            | "已经"
            | "现已"
            | "成功"
    )
}

fn cancellation_prefix_is_allowed(_prefix: &str) -> bool {
    // Cancellation is fail-open for the hard verdict: missing a trusted user
    // correction such as "Actually, don't create..." can falsely punish a
    // model for obeying it. Positive mutation authority remains strict; a
    // broad cancellation can only disable this optional adjudication gate.
    true
}

const SOURCE_CUES: &[&str] = &[
    "from",
    "using",
    "based on",
    "according to",
    "根据",
    "使用",
    "从",
];

fn direct_target_gap(gap: &str) -> bool {
    let mut gap = gap
        .trim_matches(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .to_lowercase();
    for allowed in ["新文件", "文件", "一个", "新的", "了"] {
        gap = gap.replace(allowed, "");
    }
    let gap = gap.trim();
    gap.is_empty()
        || gap
            .split_whitespace()
            .all(|word| matches!(word, "a" | "an" | "the" | "file" | "new"))
}

const CANCELLATION_ACTION_MARKERS: &[&str] = &[
    "do not create",
    "do not write",
    "do not generate",
    "do not implement",
    "do not fix",
    "do not edit",
    "do not modify",
    "don't create",
    "don't write",
    "don't modify",
    "stop creating",
    "cancel creating",
    "不要创建",
    "不要再创建",
    "不要新建",
    "不要编写",
    "不要写入",
    "不要生成",
    "不要实现",
    "不要修复",
    "不要编辑",
    "不要修改",
    "不要再修改",
    "别创建",
    "别再创建",
    "别新建",
    "别编写",
    "别写入",
    "别生成",
    "别实现",
    "别修复",
    "别编辑",
    "别修改",
    "别再修改",
    "取消创建",
    "不用创建",
    "不用修改",
    "无需创建",
    "无需修改",
    "禁止创建",
    "禁止修改",
];

fn bind_first_target_after_actions(
    clause: &str,
    action_markers: &[&str],
    targets: &[TargetSpan],
    prefix_allowed: fn(&str) -> bool,
    veto_markers: &[&str],
) -> Vec<(usize, usize, String)> {
    let lowered = ascii_lowercase_preserving_offsets(clause);
    let actions = marker_spans(&lowered, action_markers);
    let mut bound = Vec::new();
    for (index, (action_start, action_end)) in actions.iter().copied().enumerate() {
        if !prefix_allowed(&lowered[..action_start]) {
            continue;
        }
        let window_end = actions
            .get(index + 1)
            .map(|(start, _)| *start)
            .unwrap_or(clause.len());
        let Some(target) = targets
            .iter()
            .filter(|target| target.start >= action_end && target.start < window_end)
            .min_by_key(|target| target.start)
        else {
            continue;
        };
        let gap = &lowered[action_end..target.start];
        if !direct_target_gap(gap)
            || contains_any_marker(gap, SOURCE_CUES)
            || contains_any_marker(gap, veto_markers)
        {
            continue;
        }
        bound.push((action_start, target.end, target.normalized.clone()));
    }
    bound
}

fn required_state_change_targets(
    requirement: &[ContentBlock],
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<String> {
    let requirement_text = requirement
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let all_targets = lex_targets(&requirement_text, workspace_root, mode);
    let mut required = Vec::new();
    for (clause_start, clause_end) in clause_ranges(&requirement_text) {
        let clause = &requirement_text[clause_start..clause_end];
        if clause.trim_start().starts_with('>') {
            continue;
        }
        let targets = all_targets
            .iter()
            .filter(|target| target.start >= clause_start && target.start < clause_end)
            .map(|target| TargetSpan {
                start: target.start - clause_start,
                end: target.end - clause_start,
                normalized: target.normalized.clone(),
            })
            .collect::<Vec<_>>();
        let lowered_clause = ascii_lowercase_preserving_offsets(clause);
        let cancellation_spans = marker_spans(&lowered_clause, CANCELLATION_ACTION_MARKERS);
        let mut directives = bind_first_target_after_actions(
            clause,
            STATE_CHANGE_REQUIREMENT_MARKERS,
            &targets,
            directive_prefix_is_allowed,
            NON_MUTATING_REQUIREMENT_MARKERS,
        )
        .into_iter()
        .filter(|(position, _, _)| {
            !cancellation_spans
                .iter()
                .any(|(start, end)| *position >= *start && *position < *end)
        })
        .map(|(position, _, target)| (position, true, target))
        .chain(
            bind_first_target_after_actions(
                clause,
                CANCELLATION_ACTION_MARKERS,
                &targets,
                cancellation_prefix_is_allowed,
                &[],
            )
            .into_iter()
            .map(|(position, _, target)| (position, false, target)),
        )
        .collect::<Vec<_>>();
        directives.sort_by_key(|(position, _, _)| *position);
        for (_, positive, target) in directives {
            if positive {
                if !required.contains(&target) {
                    required.push(target);
                }
            } else {
                required.retain(|known| known != &target);
            }
        }
    }
    required
}

fn trim_claim_suffix(suffix: &str) -> &str {
    suffix.trim_matches(|character: char| {
        character.is_whitespace()
            || character.is_ascii_punctuation()
            || matches!(
                character,
                '。' | '，' | '；' | '：' | '！' | '？' | '—' | '–'
            )
    })
}

fn claim_target_suffix_is_allowed(
    suffix: &str,
    target: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> bool {
    let lowered = ascii_lowercase_preserving_offsets(suffix);
    const CAVEAT_BOUNDARIES: &[&str] = &[", but", " but ", " however", " though", "—", "–", "，但", "但是", "不过"];
    let boundary = CAVEAT_BOUNDARIES
        .iter()
        .filter_map(|marker| lowered.find(marker).map(|index| (index, marker.len())))
        .min_by_key(|(index, _)| *index);
    let (primary, caveat) = boundary.map_or((lowered.as_str(), None), |(index, length)| {
        (&lowered[..index], Some(&lowered[index + length..]))
    });
    let primary = trim_claim_suffix(primary);
    if !matches!(
        primary,
        ""
            | "now"
            | "successfully"
            | "as requested"
            | "successfully as requested"
            | "了"
            | "现已"
            | "已按要求完成"
    ) {
        return false;
    }
    let Some(caveat) = caveat.map(trim_claim_suffix).filter(|value| !value.is_empty()) else {
        return true;
    };

    // A caveat naming another high-confidence file is scoped to that file and
    // must not launder away the already-complete target assertion.
    let caveat_targets = lex_targets(caveat, workspace_root, mode);
    if !caveat_targets.is_empty()
        && caveat_targets
            .iter()
            .all(|other| other.normalized != target)
    {
        return true;
    }

    const DURABILITY_VETOES: &[&str] = &[
        "did not save",
        "didn't save",
        "not saved",
        "not persisted",
        "not written to disk",
        "not to disk",
        "still missing",
        "is missing",
        "does not exist",
        "doesn't exist",
        "rolled back",
        "deleted it",
        "removed it",
        "未保存",
        "未落盘",
        "未写入磁盘",
        "仍不存在",
        "文件缺失",
        "已回滚",
        "只是草稿",
    ];
    if contains_any_marker(caveat, DURABILITY_VETOES) {
        return false;
    }

    contains_any_marker(
        caveat,
        &[
            "run the tests",
            "run tests",
            "tests failed",
            "would you like anything else",
            "anything else",
            "未生成预览图",
            "预览图失败",
            "还需要其他",
        ],
    )
}

fn claimed_state_change_targets(
    answer: &str,
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<String> {
    let all_targets = lex_targets(answer, workspace_root, mode);
    let mut claimed = Vec::new();
    for (clause_start, clause_end) in clause_ranges(answer) {
        let clause = &answer[clause_start..clause_end];
        let lowered = ascii_lowercase_preserving_offsets(clause);
        let trimmed = clause.trim();
        let quoted_whole = (trimmed.starts_with('`') && trimmed.ends_with('`'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\''));
        if clause.trim_start().starts_with('>') || quoted_whole {
            continue;
        }
        let targets = all_targets
            .iter()
            .filter(|target| target.start >= clause_start && target.start < clause_end)
            .map(|target| TargetSpan {
                start: target.start - clause_start,
                end: target.end - clause_start,
                normalized: target.normalized.clone(),
            })
            .collect::<Vec<_>>();
        for (_, target_end, target) in bind_first_target_after_actions(
            clause,
            STATE_CHANGE_CLAIM_MARKERS,
            &targets,
            claim_prefix_is_allowed,
            NON_COMPLETION_CLAIM_MARKERS,
        ) {
            if claim_target_suffix_is_allowed(
                &clause[target_end..],
                &target,
                workspace_root,
                mode,
            ) && !claimed.contains(&target)
            {
                claimed.push(target);
            }
        }
        const TARGET_FIRST_CHINESE_CLAIMS: &[&str] = &[
            "已创建", "已新建", "已编写", "已写入", "已生成", "已构建", "已实现", "已修复",
            "已修改", "已更新", "已产出",
        ];
        for target in &targets {
            let suffix = &lowered[target.end..];
            let target_first_prefix = TARGET_FIRST_CHINESE_CLAIMS
                .iter()
                .copied()
                .find(|claim| trim_claim_suffix(suffix).starts_with(claim))
                .or_else(|| {
                    [
                        "is now in the workspace",
                        "is ready",
                        "is now ready",
                        "now contains the implementation",
                        "has been created",
                        "has been written",
                        "has been fixed",
                        "has been updated",
                    ]
                    .into_iter()
                    .find(|claim| trim_claim_suffix(suffix).starts_with(claim))
                });
            if let Some(prefix) = target_first_prefix {
                let trimmed_suffix = trim_claim_suffix(suffix);
                let remainder = &trimmed_suffix[prefix.len()..];
                if claim_target_suffix_is_allowed(
                    remainder,
                    &target.normalized,
                    workspace_root,
                    mode,
                ) && !claimed.contains(&target.normalized)
                {
                    claimed.push(target.normalized.clone());
                }
            }
        }
    }
    claimed
}

fn effective_claimed_state_change_targets(
    answer: &str,
    required_targets: &[String],
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Vec<String> {
    let claimed = claimed_state_change_targets(answer, workspace_root, mode);
    if !claimed.is_empty() || required_targets.is_empty() {
        return claimed;
    }
    let trimmed = answer.trim();
    let lowered = ascii_lowercase_preserving_offsets(trimmed);
    let task_complete_prefixes = [
        "task complete — see ",
        "task complete - see ",
        "task complete: see ",
    ];
    if let Some(prefix) = task_complete_prefixes
        .iter()
        .find(|prefix| lowered.starts_with(**prefix))
    {
        let targets = lex_targets(trimmed, workspace_root, mode);
        if targets.len() == 1 {
            let target = &targets[0];
            let before = lowered[..target.start]
                .trim_matches(|character: char| character.is_whitespace() || character == '"');
            let after = lowered[target.end..].trim_matches(|character: char| {
                character.is_whitespace()
                    || character.is_ascii_punctuation()
                    || matches!(character, '。' | '！')
            });
            if before == prefix.trim()
                && after.is_empty()
                && required_targets.contains(&target.normalized)
            {
                return vec![target.normalized.clone()];
            }
        }
    }
    let generic = answer
        .trim()
        .trim_end_matches(['.', '!', '。', '！'])
        .trim();
    let generic_completion = generic.eq_ignore_ascii_case("done")
        || generic.eq_ignore_ascii_case("completed")
        || matches!(generic, "已完成" | "完成了");
    generic_completion
        .then(|| required_targets.to_vec())
        .unwrap_or_default()
}

pub(crate) fn completion_adjudication(
    answer: &str,
    requirement: &[ContentBlock],
    supported_targets: &[String],
    workspace_root: &std::path::Path,
    mode: CompletionEvidenceMode,
) -> Option<CompletionAdjudication> {
    let required_targets = required_state_change_targets(requirement, workspace_root, mode);
    let claimed_targets = effective_claimed_state_change_targets(
        answer,
        &required_targets,
        workspace_root,
        mode,
    );
    required_targets
        .into_iter()
        .find(|target| {
            claimed_targets.contains(target) && !supported_targets.contains(target)
        })
        .map(|target| CompletionAdjudication::UnbackedStateChangeClaim { target })
}

/// Cumulative failed tool results in one user turn before the engine stops
/// absorbing them silently and tells the model to change course.
///
/// The stagnation guard only catches *repeated identical* failures, so a turn
/// that fails ten different ways while making nominal forward progress never
/// trips it — the observed 55-turn session absorbed exactly that: 10 tool errors
/// out of 54 calls, 1.6M input tokens, and a wrong deliverable. This budget is a
/// diversity-blind backstop: high enough that ordinary trial-and-error is
/// untouched, low enough that a turn cannot quietly burn its whole budget on
/// failures.
const TOOL_ERROR_BUDGET: usize = 8;

/// Injected once when the error budget is exhausted. Asks for a decision rather
/// than forbidding retries, because the failures may still be recoverable.
pub(crate) const TOOL_ERROR_BUDGET_NUDGE: &str = "Tool error budget: this turn has now produced \
many failed tool results. Stop improvising individual retries. State what is actually blocking \
you, then either fix the root cause with a materially different approach or report the blocker and \
what you did verify — do not keep trying variations of the same failing calls.";

/// Injected once when a turn keeps reading files one at a time.
///
/// `Read` accepts `file_paths` for several files in one call, and the guidance
/// already says so, but the observed session issued 13 separate single-file
/// reads across 13 model turns — each one re-sending the whole conversation.
const SINGLE_READ_NUDGE_THRESHOLD: usize = 8;

pub(crate) const BATCH_READ_NUDGE: &str = "Efficiency: you are reading files one per model turn, \
and every turn re-sends the whole conversation. When you already know which files you need, pass \
them together in one Read call via file_paths, and issue independent read-only calls in the same \
turn instead of one at a time.";

/// Minimum amount of already-written text before collapsing is worthwhile.
/// Narration ("Let me write the test file now.") never reaches this, so it is
/// always preserved verbatim.
const MIN_SUPERSEDED_DRAFT_CHARS: usize = 400;

/// Shortest line considered for removal. Trivial lines (`}`, `return;`) recur in
/// unrelated files, so matching them would prove nothing about supersession.
const MIN_SUPERSEDED_LINE_CHARS: usize = 8;

/// Cap on the written body compared against a draft. Beyond this the substring
/// scans stop being free, and a body this large is already unambiguous from its
/// prefix.
const MAX_COMPARED_WRITE_CHARS: usize = 262_144;

/// Tools whose input body becomes a file on disk, making identical preceding
/// text a superseded draft rather than an answer. A Bash command that echoes the
/// same bytes is deliberately excluded: nothing was persisted, so the text may
/// still be the substance of the reply.
const FILE_WRITE_TOOLS: [&str; 3] = ["Write", "Edit", "ApplyPatch"];

/// Collapse whitespace so a line still matches a body the model reindented while
/// emitting tool arguments.
fn whitespace_insensitive(text: &str) -> String {
    text.split_whitespace().collect::<String>()
}

/// Every text body a file-write tool call would persist, with its target path.
///
/// `ApplyPatch` nests its bodies under `files[]`, so each shape is read
/// explicitly rather than assuming one flat schema.
fn written_bodies(block: &ContentBlock) -> Option<(String, String)> {
    let ContentBlock::ToolUse { name, input, .. } = block else {
        return None;
    };
    if !FILE_WRITE_TOOLS.contains(&name.as_str()) {
        return None;
    }

    fn collect_bodies(target: &Value, into: &mut String) {
        for key in ["content", "new_string"] {
            if let Some(body) = target.get(key).and_then(Value::as_str) {
                into.push_str(body);
            }
        }
        if let Some(edits) = target.get("edits").and_then(Value::as_array) {
            for edit in edits {
                if let Some(body) = edit.get("new_string").and_then(Value::as_str) {
                    into.push_str(body);
                }
            }
        }
    }

    let mut bodies = String::new();
    collect_bodies(input, &mut bodies);
    let mut path = input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if let Some(files) = input.get("files").and_then(Value::as_array) {
        for file in files {
            collect_bodies(file, &mut bodies);
            if path.is_empty()
                && let Some(file_path) = file.get("file_path").and_then(Value::as_str)
            {
                path = file_path.to_owned();
            }
        }
    }
    if bodies.is_empty() {
        return None;
    }
    if path.is_empty() {
        path.push_str("the file");
    }

    let mut normalized = whitespace_insensitive(&bodies);
    normalized.truncate(
        (0..=MAX_COMPARED_WRITE_CHARS.min(normalized.len()))
            .rev()
            .find(|i| normalized.is_char_boundary(*i))
            .unwrap_or(0),
    );
    Some((path, normalized))
}

/// Drop the lines of a pre-tool text block that a file-write tool in the SAME
/// assistant round already persisted, leaving a short marker in their place.
///
/// Such lines are not an answer — they are the model composing tool arguments in
/// the open. Left intact they are replayed to the provider on every later turn,
/// so one 5 KB draft can cost tens of thousands of cumulative input tokens
/// across a long coding session while adding nothing the tool call does not
/// already carry.
///
/// Only lines that literally appear in the written body are removed, never the
/// whole block: the engine coalesces a round's text deltas into a single block,
/// so an explanation and a draft routinely share one block and discarding it
/// wholesale would silently destroy the explanation. Returns whether anything
/// was replaced.
fn supersede_written_draft(content: &mut [ContentBlock]) -> bool {
    let written: Vec<(String, String)> = content.iter().filter_map(written_bodies).collect();
    if written.is_empty() {
        return false;
    }

    let mut replaced = false;
    for block in content.iter_mut() {
        let ContentBlock::Text { text } = block else {
            continue;
        };
        if text.chars().count() < MIN_SUPERSEDED_DRAFT_CHARS {
            continue;
        }

        let mut kept: Vec<&str> = Vec::new();
        let mut dropped_chars = 0usize;
        let mut dropped_path: Option<&str> = None;
        let mut marker_at = None;
        for line in text.lines() {
            let normalized = whitespace_insensitive(line);
            let persisted = if normalized.chars().count() >= MIN_SUPERSEDED_LINE_CHARS {
                written
                    .iter()
                    .find(|(_, body)| body.contains(normalized.as_str()))
                    .map(|(path, _)| path.as_str())
            } else {
                // Short lines (`}`, `});`, blank) carry no evidence either way.
                // Keeping them would strand a wall of orphaned punctuation after
                // the marker; they are dropped along with the block they closed
                // and only survive if no neighbouring line was persisted.
                None
            };
            match persisted {
                Some(path) => {
                    dropped_chars += line.chars().count();
                    dropped_path = Some(path);
                    marker_at.get_or_insert(kept.len());
                }
                None => kept.push(line),
            }
        }

        let (Some(path), Some(marker_at)) = (dropped_path, marker_at) else {
            continue;
        };
        if dropped_chars < MIN_SUPERSEDED_DRAFT_CHARS {
            continue;
        }

        // Structural leftovers of a removed block are noise, not content: a line
        // too short to prove anything is only worth keeping if it still sits
        // next to surviving prose.
        let mut rebuilt: Vec<&str> = Vec::new();
        for (index, line) in kept.iter().enumerate() {
            let trivial = whitespace_insensitive(line).chars().count() < MIN_SUPERSEDED_LINE_CHARS;
            let neighbours_substance = |i: usize| {
                kept.get(i).is_some_and(|other: &&str| {
                    whitespace_insensitive(other).chars().count() >= MIN_SUPERSEDED_LINE_CHARS
                })
            };
            if trivial
                && !neighbours_substance(index.wrapping_sub(1))
                && !neighbours_substance(index + 1)
            {
                continue;
            }
            rebuilt.push(line);
        }

        let marker =
            format!("[Draft omitted: this turn wrote it to {path}; see that tool call for the body.]");
        let marker_at = marker_at.min(rebuilt.len());
        rebuilt.insert(marker_at, &marker);
        *text = rebuilt.join("\n");
        replaced = true;
    }
    replaced
}

#[cfg(test)]
mod set_config_tests;

#[cfg(test)]
mod phase6_tests;

#[cfg(test)]
mod compact_tests;

#[cfg(test)]
mod plan_mode_tests;

#[cfg(test)]
mod handle_command_tests;

#[derive(Debug)]
pub enum CompletionAdjudication {
    /// The accepted requirement explicitly required a workspace file mutation,
    /// but the logical turn ended without durable direct, delegated, or
    /// artifact evidence for that exact target.
    UnbackedStateChangeClaim { target: String },
    /// The model verdict was detected, but restoring the accepted-turn root
    /// could not be durably persisted. Hosts must surface the non-retryable
    /// state-consistency error and retire this runtime.
    HistoryRollbackFailed { target: String },
    /// A direct caller reached a clean model terminal, but the durable session
    /// transaction marker could not be cleared. The apparent completion must
    /// not be published or the resumable transcript reused.
    SessionCommitFailed { detail: String },
}

impl CompletionAdjudication {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnbackedStateChangeClaim { .. } => "unbacked_state_change_claim",
            Self::HistoryRollbackFailed { .. } => "completion_history_rollback_failed",
            Self::SessionCommitFailed { .. } => "completion_history_commit_failed",
        }
    }

    pub fn nudge(&self) -> &'static str {
        match self {
            Self::UnbackedStateChangeClaim { .. } => STATE_CHANGE_EVIDENCE_NUDGE,
            Self::HistoryRollbackFailed { .. } => STATE_CHANGE_EVIDENCE_NUDGE,
            Self::SessionCommitFailed { .. } => STATE_CHANGE_EVIDENCE_NUDGE,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::UnbackedStateChangeClaim { target } => format!(
                "The accepted request required '{target}' to be created or changed, but the turn ended without durable workspace or artifact evidence for that target."
            ),
            Self::HistoryRollbackFailed { target } => format!(
                "The unsupported completion claim for '{target}' was rejected, but restoring its conversation history could not be persisted. The runtime must not be reused."
            ),
            Self::SessionCommitFailed { detail } => format!(
                "{detail}. The resumable Agent session could not be committed safely and must not be reused."
            ),
        }
    }

    pub fn history_rollback_succeeded(&self) -> bool {
        matches!(self, Self::UnbackedStateChangeClaim { .. })
    }

    fn mark_history_rollback_failed(&mut self) {
        if let Self::UnbackedStateChangeClaim { target } = self {
            *self = Self::HistoryRollbackFailed {
                target: std::mem::take(target),
            };
        }
    }
}

#[derive(Debug)]
pub struct AgentResult {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub turns: usize,
    /// Attempts at the accepted requirement, including the first. 1 = the
    /// provider never hit its output ceiling with recoverable work in flight.
    pub rounds: usize,
    /// Successful state-changing tool effects across every round of this turn.
    /// Carried from a monotonic counter, never from a bounded render window.
    pub effects_ok: usize,
    /// Successful, durable workspace effects proven by atomic file tools or a
    /// correlated nested Agent. Unlike `effects_ok`, a generic successful
    /// shell/browser/tool result is not enough.
    pub durable_effect_targets: Vec<String>,
    /// State-changing tool calls the output ceiling cut off across every round.
    /// Non-zero is machine evidence that the turn was reaching for an effect.
    pub cutoff_state_changing: usize,
    /// Whether this turn's requests advertised any state-changing tool. False
    /// for plan mode and model-only runtimes, whose turns must never be judged
    /// for producing no state-changing effect.
    pub state_changing_tools_advertised: bool,
    /// Typed hard verdict carried with text and usage so hosts can emit metrics
    /// before failing the delivery instead of discarding accounting in `Err`.
    pub completion_adjudication: Option<CompletionAdjudication>,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("User aborted the session")]
    UserAborted,
    #[error("Context window nearly full ({input_tokens} tokens used, limit {limit})")]
    ContextTooLong { input_tokens: u64, limit: usize },
    #[error("Tool loop stopped: {0}")]
    Stagnation(String),
}

#[cfg(test)]
mod cache_diagnostic_tests;

#[cfg(test)]
mod transcript_tests;

#[cfg(test)]
mod tool_efficiency_tests;

#[cfg(test)]
mod superseded_draft_tests;

#[cfg(test)]
mod completion_evidence_tests;

#[cfg(test)]
mod runtime_authority_tests;
