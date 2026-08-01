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
use nomi_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use nomi_types::skill_types::{ContextModifier, PlanModeTransition, effort_to_string};
use serde_json::Value;
use tracing::Instrument;

use crate::cache_diagnostics::{CacheBreakDetector, CacheDiagnostic, CacheStats};
use crate::compact::state::CompactState;
use crate::compact::{auto, emergency, estimate, micro};
use crate::confirm::ToolConfirmer;
use crate::tool_execution::{
    ExecutionControl, ProviderToolAuthority, SKIPPED_AFTER_PRIOR_ERROR,
    execute_tool_calls_scoped, execute_tool_calls_with_approval,
};
use crate::output::{OutputSink, ToolCallExecutionContext, ToolCallRetryContext};
use crate::plan::prompt as plan_prompt;
use crate::plan::state::PlanState;
use crate::session::{EditableTurnCheckpoint, Session, SessionManager};

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
/// turn out of both approval and execution paths.
const MAX_PROVIDER_TURN_TOOL_CALLS: usize = 128;

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
/// Bedrock Converse rejects a request containing more than 20 images.
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
            Ok(result) => (
                "ok",
                match result.stop_reason {
                    StopReason::EndTurn => "end_turn",
                    StopReason::ToolUse => "tool_use",
                    StopReason::MaxTokens => "max_tokens",
                    StopReason::MaxTurns => "max_turns",
                },
                "none",
                result.turns,
            ),
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
    messages: Vec<Message>,
    system_prompt: String,
    model: String,
    max_tokens: u32,
    max_turns: Option<usize>,
    total_usage: TokenUsage,
    thinking: Option<nomi_types::llm::ThinkingConfig>,
    /// Resolved provider compat settings (for capability validation)
    compat: nomi_config::compat::ProviderCompat,
    confirmer: Arc<Mutex<ToolConfirmer>>,
    hooks: Option<HookEngine>,
    session_manager: Option<SessionManager>,
    current_session: Option<Session>,
    output: Arc<dyn OutputSink>,
    current_msg_id: String,
    approval_manager: Option<Arc<nomi_protocol::ToolApprovalManager>>,
    protocol_writer: Option<Arc<dyn nomi_protocol::writer::ProtocolEmitter>>,
    allow_list: Vec<String>,
    /// Persisted reasoning effort, updated by skill context modifiers.
    /// Carried into each turn's LlmRequest.reasoning_effort.
    current_reasoning_effort: Option<String>,
    /// Compaction configuration (thresholds, enabled flag, etc.)
    compact_config: CompactConfig,
    /// Runtime compaction state (circuit breaker, last input tokens)
    compact_state: CompactState,
    /// Runtime plan mode state (active flag, pre-plan allow-list, plan file path)
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
        let confirmer =
            ToolConfirmer::new(config.tools.auto_approve, config.tools.allow_list.clone());

        let session_manager = if config.session.enabled {
            Some(SessionManager::new(
                config.session.directory.clone().into(),
                config.session.max_sessions,
            ))
        } else {
            None
        };

        let allow_list = config.tools.allow_list.clone();
        let compact_config = config.compact.clone();

        Self {
            provider,
            tools,
            messages: Vec::new(),
            system_prompt,
            model: config.model,
            max_tokens: config.max_tokens,
            max_turns: config.max_turns,
            total_usage: TokenUsage::default(),
            thinking: config.thinking,
            compat: config.compat.clone(),
            confirmer: Arc::new(Mutex::new(confirmer)),
            hooks: Some(HookEngine::new(config.hooks.clone(), cwd.clone())),
            session_manager,
            current_session: None,
            output,
            current_msg_id: String::new(),
            approval_manager: None,
            protocol_writer: None,
            allow_list,
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
        let confirmer =
            ToolConfirmer::new(config.tools.auto_approve, config.tools.allow_list.clone());

        let session_manager = if config.session.enabled {
            Some(SessionManager::new(
                config.session.directory.clone().into(),
                config.session.max_sessions,
            ))
        } else {
            None
        };

        let allow_list = config.tools.allow_list.clone();
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

        Self {
            provider,
            tools,
            messages: session.messages.clone(),
            system_prompt,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            max_turns: config.max_turns,
            total_usage: session.total_usage.clone(),
            thinking: config.thinking,
            compat: config.compat.clone(),
            confirmer: Arc::new(Mutex::new(confirmer)),
            hooks: Some(HookEngine::new(config.hooks.clone(), cwd)),
            session_manager,
            current_session: Some(session),
            output,
            current_msg_id: String::new(),
            approval_manager: None,
            protocol_writer: None,
            allow_list,
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
            self.current_session = Some(session);
        }
        Ok(())
    }

    /// Get the current session ID (if sessions are enabled and initialized)
    pub fn current_session_id(&self) -> Option<String> {
        self.current_session.as_ref().map(|s| s.id.clone())
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

    pub fn set_approval_manager(&mut self, mgr: Arc<nomi_protocol::ToolApprovalManager>) {
        self.approval_manager = Some(mgr);
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
        let result = async {
            let result = self
                .execute_turn_inner(
                    user_content,
                    msg_id,
                    source_message_id,
                    &mut efficiency,
                    &mut safe_messages,
                    &mut turn_started,
                )
                .await;
            efficiency.log(&session_id, msg_id, &result);
            result
        }
        .instrument(span)
        .await;

        // Keep the image available for every provider/tool iteration in this
        // turn execution, then remove it before the engine is reused. `execute_turn_inner`
        // has several success/error return paths and may already have persisted
        // the original turn, so perform the cleanup in this outer finally-like
        // wrapper and save the redacted transcript once more. If the host drops
        // this future during non-cooperative cancellation, `abort_current_turn`
        // performs the same cleanup explicitly.
        if result.is_err() && turn_started {
            self.messages = safe_messages;
            if matches!(
                &result,
                Err(AgentError::Provider(_)
                    | AgentError::ApiError(_)
                    | AgentError::ContextTooLong { .. })
            ) {
                self.strip_tool_images_after_provider_error();
            }
            self.save_session();
        } else if self.redact_user_images_since(first_new_message) {
            self.save_session();
        }
        result
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
            });
        }
        self.messages.push(Message::now(Role::User, user_content));
        *turn_started = true;
        // Persist before the first provider await. A stop or process exit must
        // not discard rewind authority for the accepted user message.
        self.save_session();

        let mut turn: usize = 0;
        let mut tool_retry_tracker = ToolRetryTracker::default();
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
            let tools = if self.plan_state.is_active {
                // Plan mode: only Info-category tools (excluding EnterPlanMode)
                self.tools.to_tool_defs_filtered(|t| {
                    t.category() == ToolCategory::Info && t.name() != "EnterPlanMode"
                })
            } else {
                // Normal mode: all tools except ExitPlanMode
                self.tools
                    .to_tool_defs_filtered(|t| t.name() != "ExitPlanMode")
            };
            // This exact request is the authority for what the provider may
            // call. Registry membership is broader (for example, plan mode
            // deliberately hides mutating tools), so dispatch must never use
            // the live registry as an implicit allow-list.
            let tool_authority = ProviderToolAuthority::from_request_tools(&tools);

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

            // Host resource notifications use the trusted system channel, not
            // Role::User. Drain as late as possible before constructing the
            // request so idle notices and mid-turn notices both reach the next
            // provider boundary without creating a new Agent turn.
            let system = append_system_resource_context(
                system,
                self.drain_system_resource_notices(),
            );

            // Record prompt state for cache diagnostics
            self.cache_detector.record_request(&system, &tools);

            let request = LlmRequest {
                model: self.model.clone(),
                system,
                messages: self.messages.clone(),
                tools,
                max_tokens: self.max_tokens,
                thinking: self.thinking.clone(),
                reasoning_effort: self.current_reasoning_effort.clone(),
            };

            efficiency.observe_model_turn_attempt();
            let stream_start = std::time::Instant::now();
            let mut rx = self.provider.stream(&request).await?;
            let mut assistant_text = String::new();
            let mut thinking_text = String::new();
            let mut thinking_signature: Option<String> = None;
            let mut tool_calls: Vec<ContentBlock> = Vec::new();
            let mut previewed_tool_calls: BTreeMap<String, String> = BTreeMap::new();
            let mut stop_reason = StopReason::EndTurn;
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
                        // the canonical call seen by lifecycle output, approval,
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
                    LlmEvent::ThinkingSignature(signature) => {
                        thinking_signature = Some(signature);
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
                StopReason::EndTurn | StopReason::MaxTokens if !tool_calls.is_empty() => Some(
                    "provider stream protocol violation: EndTurn/MaxTokens Done contained tool calls",
                ),
                StopReason::MaxTurns => Some(
                    "provider stream protocol violation: provider emitted engine-only MaxTurns Done",
                ),
                _ => None,
            };
            if let Some(error) = terminal_shape_error {
                return Err(AgentError::ApiError(error.to_string()));
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

            self.messages
                .push(Message::now(Role::Assistant, assistant_content));

            if tool_calls.is_empty() {
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
                        self.messages
                            .push(Message::now(Role::User, vec![ContentBlock::Text { text }]));
                    }
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
                self.save_session();
                return Ok(AgentResult {
                    text: assistant_text,
                    stop_reason,
                    usage: self.total_usage.clone(),
                    turns: turn + 1,
                });
            }

            let mut outcome = if let Some(ref approval_mgr) = self.approval_manager {
                // JSON stream mode: use protocol-based approval
                let writer = self
                    .protocol_writer
                    .as_ref()
                    .expect("protocol writer required for approval");
                let auto_approve = self.confirmer.lock().unwrap().is_auto_approve();
                match execute_tool_calls_with_approval(
                    &self.tools,
                    &tool_calls,
                    &tool_authority,
                    approval_mgr,
                    writer,
                    &self.current_msg_id,
                    auto_approve,
                    &self.allow_list,
                    self.hooks.as_mut(),
                    self.compaction_level,
                    self.toon_enabled,
                )
                .await
                {
                    Ok(o) => o,
                    Err(ExecutionControl::Quit) => {
                        self.save_session();
                        return Err(AgentError::UserAborted);
                    }
                }
            } else {
                // Terminal mode: use interactive confirmation
                match execute_tool_calls_scoped(
                    &self.tools,
                    &tool_calls,
                    &tool_authority,
                    &self.current_msg_id,
                    &self.confirmer,
                    self.hooks.as_mut(),
                    self.compaction_level,
                    self.toon_enabled,
                )
                .await
                {
                    Ok(o) => o,
                    Err(ExecutionControl::Quit) => {
                        self.save_session();
                        return Err(AgentError::UserAborted);
                    }
                }
            };
            let confirmed_invalid_argument_call_ids =
                confirmed_predispatch_schema_invalid_call_ids(
                    &invalid_argument_call_ids,
                    &outcome.results,
                );
            // Deliver binary outputs before success accounting or the next
            // model turn. A provider/tool RPC succeeding is not enough: when
            // the user-facing sink cannot persist the media, convert the tool
            // result into an error and remove the undeliverable in-memory image
            // so the model is forced to retry instead of claiming completion.
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
                        crate::output::ToolMediaDelivery::Delivered { context } => {
                            if !context.trim().is_empty() {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(&context);
                            }
                            // Non-image artifacts are represented to the model by
                            // their verified filesystem locators. Provider image
                            // adapters must never receive audio/PDF/resource bytes
                            // mislabeled as visual input.
                            images.retain(|artifact| {
                                artifact
                                    .media_type
                                    .trim()
                                    .to_ascii_lowercase()
                                    .starts_with("image/")
                            });
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
                }
            }

            efficiency.observe_results(&outcome.results);
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
            if !steered.is_empty() {
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
                        content.push_str(&format!(
                            "\n(Only the first {retained} image attachment(s) in this tool result remain; {removed} later attachment(s) were omitted by the recent-image/provider payload budget.)"
                        ));
                    }
                }
            }
        }
    }

    /// Provider failures terminate the current model pass. Historical visual
    /// observations are transport-heavy and can reproduce the same gateway
    /// failure on every retry/model switch, while their textual tool result is
    /// enough to tell the next model to capture a fresh view.
    fn strip_tool_images_after_provider_error(&mut self) {
        const NOTE: &str = "(Image attachment omitted after provider error recovery; capture a fresh observation if needed.)";
        for message in &mut self.messages {
            for block in &mut message.content {
                let ContentBlock::ToolResult { content, images, .. } = block else {
                    continue;
                };
                if images.is_empty() {
                    continue;
                }
                images.clear();
                if !content.contains(NOTE) {
                    content.push('\n');
                    content.push_str(NOTE);
                }
            }
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
            for tool_name in &modifier.allowed_tools {
                if !self.allow_list.contains(tool_name) {
                    self.allow_list.push(tool_name.clone());
                }
                self.confirmer.lock().unwrap().add_to_allow_list(tool_name);
            }

            // Handle plan mode transitions
            if let Some(ref transition) = modifier.plan_mode_transition {
                match transition {
                    PlanModeTransition::Enter => {
                        self.plan_state.pre_plan_allow_list = self.allow_list.clone();
                        self.plan_state.is_active = true;
                        if let Some(ref flag) = self.plan_active_flag {
                            flag.store(true, Ordering::Release);
                        }
                    }
                    PlanModeTransition::Exit { .. } => {
                        self.plan_state.is_active = false;
                        self.allow_list = self.plan_state.pre_plan_allow_list.clone();
                        if let Some(ref flag) = self.plan_active_flag {
                            flag.store(false, Ordering::Release);
                        }
                    }
                }
            }
        }
    }

    fn save_session(&mut self) {
        if let (Some(mgr), Some(session)) = (&self.session_manager, &mut self.current_session) {
            session.messages = self.messages.clone();
            session.total_usage = self.total_usage.clone();
            session.activated_deferred_tools = self.tools.session_deferred_tool_identities();
            session.editable_turn = self.editable_turn.clone();
            session.updated_at = chrono::Utc::now();
            if let Err(e) = mgr.save(session) {
                self.output
                    .emit_warning(&format!("Failed to save session: {}", e));
            }
            if let Err(e) = mgr.update_index_for(session) {
                self.output
                    .emit_warning(&format!("Failed to update session index: {}", e));
            }
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
    pub fn clear_context(&mut self) {
        self.messages.clear();
        self.editable_turn = None;
        self.compact_state = CompactState::new();
        self.total_usage = TokenUsage::default();
        self.save_session();
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
    pub fn rewind_last_turn(&mut self, expected_source_message_id: &str) -> bool {
        let Some(checkpoint) = self.editable_turn.as_ref().cloned() else {
            return !expected_source_message_id.is_empty() && self.messages.is_empty();
        };
        if checkpoint.source_message_id != expected_source_message_id
            || checkpoint.start_len > self.messages.len()
        {
            return false;
        }
        self.messages.truncate(checkpoint.start_len);
        self.editable_turn = None;
        self.save_session();
        true
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
pub struct AgentResult {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub turns: usize,
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
