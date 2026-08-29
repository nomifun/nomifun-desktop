use std::collections::BTreeSet;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use nomi_config::hooks::HookEngine;
use nomi_protocol::events::{OutputType, ProtocolEvent, ToolStatus};
use nomi_protocol::writer::ProtocolEmitter;
use nomi_types::message::ContentBlock;
use nomi_types::skill_types::ContextModifier;
use nomi_types::tool::{ToolDef, ToolResult};

use nomi_tools::{ToolExecutionContext, registry::ToolRegistry};

pub(crate) const SKIPPED_AFTER_PRIOR_ERROR: &str = "\
Skipped because a previous tool call in this assistant turn failed. Inspect the failed result first, then decide whether to retry with a larger timeout, use exec_command/write_stdin for long-running commands, or choose a different next step. Do not assume this step ran.";

/// The combined output of a tool execution batch: protocol content blocks
/// paired with per-call context modifiers (None for non-skill tools).
pub struct ToolCallOutcome {
    pub results: Vec<ContentBlock>,
    pub modifiers: Vec<Option<ContextModifier>>,
    /// Machine-observed state-changing effects completed by nested Agents,
    /// paired by index with `results`.
    pub delegated_effects: Vec<Vec<String>>,
}

/// Keeps the diagnostic `started`/`completed` lifecycle paired even when an
/// in-flight tool future is dropped by turn cancellation. Protocol/UI terminal
/// events remain owned by the surrounding turn machinery; this guard only
/// closes the structured log span used to diagnose stuck tool calls.
struct ToolExecutionLog<'a> {
    tool: &'a str,
    call_id: &'a str,
    started: std::time::Instant,
    stage: &'static str,
    finished: bool,
}

impl<'a> ToolExecutionLog<'a> {
    fn start(tool: &'a str, call_id: &'a str) -> Self {
        tracing::info!(
            target: "nomi_agent",
            tool,
            call_id,
            "tool execution started"
        );
        Self {
            tool,
            call_id,
            started: std::time::Instant::now(),
            stage: "pre_hook",
            finished: false,
        }
    }

    fn enter(&mut self, stage: &'static str) {
        self.stage = stage;
    }

    fn finish(&mut self, success: bool, outcome: &'static str) {
        let duration_ms = self.started.elapsed().as_millis() as u64;
        tracing::info!(
            target: "nomi_agent",
            tool = self.tool,
            call_id = self.call_id,
            stage = self.stage,
            duration_ms,
            success,
            outcome,
            "tool execution completed"
        );
        self.finished = true;
    }
}

impl Drop for ToolExecutionLog<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        let duration_ms = self.started.elapsed().as_millis() as u64;
        tracing::info!(
            target: "nomi_agent",
            tool = self.tool,
            call_id = self.call_id,
            stage = self.stage,
            duration_ms,
            success = false,
            outcome = "aborted_or_cancelled",
            "tool execution completed"
        );
    }
}

/// Immutable execution authority captured from the exact tool definitions in
/// one provider request. It cannot be reconstructed from the live registry:
/// plan mode, deferred activation, and later dynamic registration can all make
/// registry membership broader than what that request was allowed to call.
#[derive(Debug, Clone)]
pub struct ProviderToolAuthority {
    advertised: BTreeSet<String>,
    deferred: BTreeSet<String>,
}

impl ProviderToolAuthority {
    pub fn from_request_tools(tools: &[ToolDef]) -> Self {
        Self {
            advertised: tools.iter().map(|tool| tool.name.clone()).collect(),
            deferred: tools
                .iter()
                .filter(|tool| tool.deferred)
                .map(|tool| tool.name.clone())
                .collect(),
        }
    }

    pub(crate) fn advertises(&self, name: &str) -> bool {
        self.advertised.contains(name)
    }

    pub(crate) fn is_deferred(&self, name: &str) -> bool {
        self.deferred.contains(name)
    }
}

impl std::ops::Deref for ToolCallOutcome {
    type Target = Vec<ContentBlock>;
    fn deref(&self) -> &Self::Target {
        &self.results
    }
}

impl std::ops::DerefMut for ToolCallOutcome {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.results
    }
}

/// Execute provider-advertised tool calls directly under the selected request
/// snapshot, namespacing provider tool-call ids by the durable turn/message id.
/// This prevents providers that reuse `call_0` in later turns from aliasing a
/// prior operation receipt. `authority` must be the snapshot captured from the
/// request that produced `tool_calls`; there is deliberately no live-registry
/// convenience overload. Pass an empty `execution_scope` only when call-id
/// aliasing across turns is acceptable (for example, single-shot test harnesses).
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_calls_scoped(
    registry: &ToolRegistry,
    tool_calls: &[ContentBlock],
    authority: &ProviderToolAuthority,
    execution_scope: &str,
    mut hooks: Option<&mut HookEngine>,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
) -> Result<ToolCallOutcome, std::convert::Infallible> {
    let mut results = Vec::new();
    let mut modifiers = Vec::new();
    let mut delegated_effects = Vec::new();
    let mut halt_after_error = false;
    // Engine-produced calls are already canonical. Preparing again here keeps
    // direct/internal execution paths on the same boundary and guarantees that
    // validation, hooks, category checks, and dispatch all receive one value.
    let prepared_tool_calls = tool_calls
        .iter()
        .map(|call| prepare_call_for_execution(registry, call, authority))
        .collect::<Vec<_>>();

    for batch in partition(registry, &prepared_tool_calls, authority) {
        if halt_after_error {
            for call in &batch.calls {
                results.push(skipped_after_prior_error(call));
                modifiers.push(None);
                delegated_effects.push(Vec::new());
            }
            continue;
        }

        if batch.is_concurrent {
            // Preflight the entire concurrent batch before any dispatch.
            // This preserves the provider-turn snapshot even when ToolSearch
            // and its target are emitted together, and guarantees deferred or
            // schema-invalid tools cannot run.
            let mut completed: Vec<
                Option<(ContentBlock, Option<ContextModifier>, Vec<String>)>,
            > =
                std::iter::repeat_with(|| None)
                    .take(batch.calls.len())
                    .collect();
            for (idx, call) in batch.calls.iter().enumerate() {
                if let Some(gated) = invocation_gate_result(
                    registry,
                    call,
                    authority,
                ) {
                    completed[idx] = Some((gated, None, Vec::new()));
                }
            }

            // Concurrent tools are never SkillTool
            // (`is_concurrency_safe=false` for Skill), so no skill hook merge
            // is needed here.
            let mut runnable = Vec::new();
            for (idx, call) in batch.calls.iter().enumerate() {
                if completed[idx].is_some() {
                    continue;
                }
                runnable.push((idx, *call));
            }
            // Reborrow as shared for concurrent execution.
            let hooks_shared: Option<&HookEngine> = hooks.as_deref();
            let futures: Vec<_> = runnable
                .iter()
                .map(|(_, call)| {
                    execute_single_with_authority(
                        registry,
                        call,
                        authority,
                        execution_scope,
                        hooks_shared,
                        compaction_level,
                        toon_enabled,
                    )
                })
                .collect();
            let batch_results = futures::future::join_all(futures).await;
            for ((idx, _), outcome) in runnable.into_iter().zip(batch_results) {
                completed[idx] = Some(outcome);
            }
            for outcome in completed {
                let (block, modifier, nested_effects) =
                    outcome.expect("every concurrent call has an outcome");
                if block_is_error(&block) {
                    halt_after_error = true;
                }
                results.push(block);
                modifiers.push(modifier);
                delegated_effects.push(nested_effects);
            }
        } else {
            for call in &batch.calls {
                if halt_after_error {
                    results.push(skipped_after_prior_error(call));
                    modifiers.push(None);
                    delegated_effects.push(Vec::new());
                    continue;
                }
                if let Some(gated) = invocation_gate_result(
                    registry,
                    call,
                    authority,
                ) {
                    halt_after_error = true;
                    results.push(gated);
                    modifiers.push(None);
                    delegated_effects.push(Vec::new());
                    continue;
                }
                // Reborrow as shared for execute_single, then reclaim mut for merge.
                let block;
                let modifier;
                let nested_effects;
                {
                    let hooks_shared: Option<&HookEngine> = hooks.as_deref();
                    (block, modifier, nested_effects) = execute_single_with_authority(
                        registry,
                        call,
                        authority,
                        execution_scope,
                        hooks_shared,
                        compaction_level,
                        toon_enabled,
                    )
                    .await;
                }
                // Merge skill hooks after a successful sequential execution.
                if !block_is_error(&block) {
                    maybe_merge_skill_hooks(registry, call, hooks.as_deref_mut());
                } else {
                    halt_after_error = true;
                }
                results.push(block);
                modifiers.push(modifier);
                delegated_effects.push(nested_effects);
            }
        }
    }

    Ok(ToolCallOutcome {
        results,
        modifiers,
        delegated_effects,
    })
}

fn prepare_call_for_execution(
    registry: &ToolRegistry,
    call: &ContentBlock,
    authority: &ProviderToolAuthority,
) -> ContentBlock {
    let ContentBlock::ToolUse {
        id,
        name,
        input,
        extra,
    } = call
    else {
        return call.clone();
    };
    if !authority.advertises(name) || authority.is_deferred(name) || !input.is_object() {
        return call.clone();
    }
    let Ok(input) = registry.prepare_input(name, input.clone()) else {
        return call.clone();
    };
    ContentBlock::ToolUse {
        id: id.clone(),
        name: name.clone(),
        input,
        extra: extra.clone(),
    }
}

/// Build the fail-closed local result for a tool whose full schema was not
/// active when this provider turn began. Callers invoke this before any
/// hook, running event, or tool dispatch.
fn deferred_gate_result(
    call: &ContentBlock,
    authority: &ProviderToolAuthority,
) -> Option<ContentBlock> {
    let ContentBlock::ToolUse { id, name, .. } = call else {
        return None;
    };
    if !authority.is_deferred(name) {
        return None;
    }

    Some(ContentBlock::ToolResult {
        tool_use_id: id.clone(),
        content: format!(
            "Tool '{name}' is deferred and its schema was not activated at the start of \
             this model turn. Call ToolSearch for '{name}', then call the tool in a \
             subsequent model turn. The tool was not executed."
        ),
        is_error: true,
        images: Vec::new(),
    })
}

/// Validate an invocation before any tool-specific policy method, hook, running
/// event, or dispatch. Deferred authority is checked first so a
/// provider that only saw a schema stub is told to activate it instead of being
/// asked to guess required parameters it was never shown.
fn invocation_gate_result(
    registry: &ToolRegistry,
    call: &ContentBlock,
    authority: &ProviderToolAuthority,
) -> Option<ContentBlock> {
    let ContentBlock::ToolUse { id, name, .. } = call else {
        return None;
    };
    if !authority.advertises(name) {
        let content = if registry.get(name).is_none() {
            format!("Unknown tool: {name}. The tool was not executed.")
        } else {
            format!(
                "Tool '{name}' was not advertised in the provider request that produced this call. The tool was not executed."
            )
        };
        return Some(ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            content,
            is_error: true,
            images: Vec::new(),
        });
    }
    if let Some(gated) = deferred_gate_result(call, authority) {
        return Some(gated);
    }
    let ContentBlock::ToolUse { input, .. } = call else {
        unreachable!("tool-use shape checked above")
    };
    if !input.is_object() {
        return Some(ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            content: format!(
                "Invalid arguments for tool '{name}': expected a JSON object. Correct the arguments and retry; the tool was not executed."
            ),
            is_error: true,
            images: Vec::new(),
        });
    }
    registry
        .validate_input(name, input)
        .err()
        .map(|content| ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            content,
            is_error: true,
            images: Vec::new(),
        })
}

/// Extract a human-readable message from a caught panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
async fn execute_single(
    registry: &ToolRegistry,
    call: &ContentBlock,
    hooks: Option<&HookEngine>,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
) -> (ContentBlock, Option<ContextModifier>, Vec<String>) {
    let authority = ProviderToolAuthority::from_request_tools(&registry.to_tool_defs());
    execute_single_with_authority(
        registry,
        call,
        &authority,
        "",
        hooks,
        compaction_level,
        toon_enabled,
    )
    .await
}

async fn execute_single_with_authority(
    registry: &ToolRegistry,
    call: &ContentBlock,
    authority: &ProviderToolAuthority,
    execution_scope: &str,
    hooks: Option<&HookEngine>,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
) -> (ContentBlock, Option<ContextModifier>, Vec<String>) {
    let ContentBlock::ToolUse { name, input, .. } = call
    else {
        unreachable!("execute_single called with non-ToolUse block")
    };

    if let Some(gated) = invocation_gate_result(registry, call, authority) {
        return (gated, None, Vec::new());
    }

    let timeout = registry
        .get(name)
        .map(|tool| tool.execution_timeout(input))
        .unwrap_or(nomi_tools::DEFAULT_TOOL_EXECUTION_TIMEOUT);
    let timeout = if timeout.is_zero() {
        Duration::from_millis(1)
    } else {
        timeout
    };
    match tokio::time::timeout(
        timeout,
        execute_single_without_deadline(
            registry,
            call,
            execution_scope,
            hooks,
            compaction_level,
            toon_enabled,
        ),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => {
            let ContentBlock::ToolUse { id, name, .. } = call else {
                unreachable!("execute_single called with non-ToolUse block")
            };
            if let Some(tool) = registry.get(name) {
                let execution_context =
                    ToolExecutionContext::from_scoped_tool_call(execution_scope, id);
                // The deadline drops the execution future before its ordinary
                // finally-like sidecar drain. Consume any evidence staged by a
                // boundary tool so a later reused provider call id cannot
                // inherit it.
                let _ = tool.take_delegated_effects(&execution_context);
            }
            tracing::error!(
                target: "nomi_agent",
                tool = %name,
                call_id = %id,
                timeout_ms = timeout.as_millis() as u64,
                "tool execution exceeded its bounded deadline"
            );
            (
                ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: format!(
                        "Tool '{name}' timed out after {} ms. The invocation was cancelled \
                         before a result was available; do not assume its side effects \
                         completed. Inspect the current state before retrying.",
                        timeout.as_millis()
                    ),
                    is_error: true,
                    images: Vec::new(),
                },
                None,
                Vec::new(),
            )
        }
    }
}

async fn execute_single_without_deadline(
    registry: &ToolRegistry,
    call: &ContentBlock,
    execution_scope: &str,
    hooks: Option<&HookEngine>,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
) -> (ContentBlock, Option<ContextModifier>, Vec<String>) {
    let ContentBlock::ToolUse {
        id, name, input, ..
    } = call
    else {
        unreachable!("execute_single called with non-ToolUse block")
    };

    let mut execution_log = ToolExecutionLog::start(name, id);

    // Run pre-tool-use hooks
    if let Some(hook_engine) = hooks
        && let Err(e) = hook_engine.run_pre_tool_use(name, input).await
    {
        execution_log.finish(false, "blocked_by_hook");
        return (
            ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: format!("Blocked by hook: {}", e),
                is_error: true,
                images: Vec::new(),
            },
            None,
            Vec::new(),
        );
    }

    execution_log.enter("tool");
    let (result, modifier, delegated_effects) = match registry.get(name) {
        Some(tool) => {
            let max_size = tool.max_result_size();
            // `input` passed the strict object/schema preflight before
            // dispatch all see the exact provider-supplied value.
            // Catch a panic inside the tool so it becomes an error ToolResult
            // fed back to the model, instead of unwinding out of the agent loop
            // and terminating the subprocess — nomi-cli awaits `engine.execute_turn()`
            // directly on the `#[tokio::main]` task with no catch_unwind above
            // it. This catches Rust *unwinding* panics only; a native fault
            // (SIGSEGV/SIGABRT inside FFI) is process-wide and cannot be caught
            // here — those must be prevented at the FFI boundary or isolated in
            // a separate process.
            let execution_context =
                ToolExecutionContext::from_scoped_tool_call(execution_scope, id);
            let r = match AssertUnwindSafe(
                tool.execute_with_context(input.clone(), &execution_context),
            )
            .catch_unwind()
            .await
            {
                Ok(r) => r,
                Err(payload) => {
                    let msg = panic_message(payload.as_ref());
                    tracing::error!(
                        target: "nomi_agent",
                        tool = %name,
                        call_id = %id,
                        panic = %msg,
                        "tool panicked; recovered as an error result"
                    );
                    ToolResult::error(format!(
                        "Tool '{name}' panicked and was recovered (the agent stays alive): {msg}"
                    ))
                }
            };
            let modifier = if r.is_error {
                None
            } else {
                tool.context_modifier_for(input)
            };
            // Always consume the operation-id sidecar. A boundary tool that
            // staged evidence before returning an error must not leave a stale
            // entry that can be observed by a later reused provider call id.
            let staged_delegated_effects = tool.take_delegated_effects(&execution_context);
            let delegated_effects = if r.is_error {
                Vec::new()
            } else {
                staged_delegated_effects
            };
            let content = truncate_result(&r.content, max_size);
            let content = nomi_compact::compact_output(&content, compaction_level);
            let content = if toon_enabled {
                nomi_compact::compact_output_toon(&content)
            } else {
                content
            };
            (
                ToolResult {
                    content,
                    is_error: r.is_error,
                    images: r.images,
                },
                modifier,
                delegated_effects,
            )
        }
        None => (
            ToolResult::error(format!("Unknown tool: {}", name)),
            None,
            Vec::new(),
        ),
    };

    // Run post-tool-use hooks
    if let Some(hook_engine) = hooks {
        execution_log.enter("post_hook");
        let messages = hook_engine
            .run_post_tool_use(name, input, &result.content)
            .await;
        for msg in messages {
            tracing::info!(target: "nomi_agent", hook_message = %msg, "post-tool-use hook output");
        }
    }

    execution_log.enter("finalize");
    execution_log.finish(
        !result.is_error,
        if result.is_error { "error" } else { "success" },
    );

    // Defense-in-depth: scrub secret patterns (API keys, tokens, PEM blocks)
    // from tool output before it enters the model context / provider request /
    // persisted transcript. Tight patterns → negligible false positives. (Phase 1)
    let content = nomi_redact::redact_secrets_owned(result.content);

    (
        ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            content,
            is_error: result.is_error,
            images: result.images,
        },
        modifier,
        delegated_effects,
    )
}

fn skipped_after_prior_error(call: &ContentBlock) -> ContentBlock {
    let ContentBlock::ToolUse { id, .. } = call else {
        unreachable!("skipped_after_prior_error called with non-ToolUse block")
    };
    ContentBlock::ToolResult {
        tool_use_id: id.clone(),
        content: SKIPPED_AFTER_PRIOR_ERROR.to_string(),
        is_error: true,
        images: Vec::new(),
    }
}

fn emit_skipped_after_prior_error(
    writer: &Arc<dyn ProtocolEmitter>,
    msg_id: &str,
    call: &ContentBlock,
) -> ContentBlock {
    let block = skipped_after_prior_error(call);
    emit_tool_result_event(writer, msg_id, call, &block);
    block
}

fn emit_tool_result_event(
    writer: &Arc<dyn ProtocolEmitter>,
    msg_id: &str,
    call: &ContentBlock,
    block: &ContentBlock,
) {
    if let (
        ContentBlock::ToolUse { id, name, .. },
        ContentBlock::ToolResult {
            content, is_error, ..
        },
    ) = (call, &block)
    {
        let _ = writer.emit(&ProtocolEvent::ToolResult {
            msg_id: msg_id.to_string(),
            call_id: id.clone(),
            tool_name: name.clone(),
            status: if *is_error {
                ToolStatus::Error
            } else {
                ToolStatus::Success
            },
            output: content.clone(),
            output_type: OutputType::Text,
            metadata: None,
        });
    }
}

/// Execute provider-advertised tool calls directly while emitting JSON stream
/// running/result lifecycle events.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_calls_with_protocol(
    registry: &ToolRegistry,
    tool_calls: &[ContentBlock],
    authority: &ProviderToolAuthority,
    writer: &Arc<dyn ProtocolEmitter>,
    msg_id: &str,
    mut hooks: Option<&mut HookEngine>,
    compaction_level: nomi_compact::CompactionLevel,
    toon_enabled: bool,
) -> Result<ToolCallOutcome, std::convert::Infallible> {
    let mut results = Vec::new();
    let mut modifiers = Vec::new();
    let mut delegated_effects = Vec::new();
    let mut halt_after_error = false;
    // Keep direct protocol callers on the same canonical boundary as the
    // engine and terminal path. Every later hook and dispatch below observes
    // this one schema-validated value.
    let prepared_tool_calls = tool_calls
        .iter()
        .map(|call| prepare_call_for_execution(registry, call, authority))
        .collect::<Vec<_>>();
    let tool_calls = prepared_tool_calls.as_slice();

    // Consecutive concurrency-safe calls run together. Gated calls remain
    // serial so they can emit only their paired error result, never a running
    // event.
    let batchable: Vec<bool> = tool_calls
        .iter()
        .map(|call| {
            let ContentBlock::ToolUse { name, input, .. } = call else {
                return false;
            };
            // Blocked calls must be routed through the serial preflight gate,
            // never into a concurrent group that emits ToolRunning first.
            if invocation_gate_result(registry, call, authority).is_some() {
                return false;
            }
            registry
                .get(name)
                .is_some_and(|tool| tool.is_concurrency_safe(input))
        })
        .collect();

    for group in group_batches(&batchable) {
        if halt_after_error {
            for idx in group.clone() {
                let block = emit_skipped_after_prior_error(writer, msg_id, &tool_calls[idx]);
                results.push(block);
                modifiers.push(None);
                delegated_effects.push(Vec::new());
            }
            continue;
        }

        // running for all, execute in parallel (join_all preserves submission
        // order so tool_use/tool_result pairing stays intact), emit results in
        // order. This brings the production/protocol path to parity with the REPL
        // path, which already parallelized. (Phase 2 tool-call)
        if group.end - group.start > 1 {
            for idx in group.clone() {
                if let ContentBlock::ToolUse { id, name, .. } = &tool_calls[idx] {
                    let _ = writer.emit(&ProtocolEvent::ToolRunning {
                        msg_id: msg_id.to_string(),
                        call_id: id.clone(),
                        tool_name: name.clone(),
                    });
                }
            }
            let hooks_shared: Option<&HookEngine> = hooks.as_deref();
            let futures: Vec<_> = group
                .clone()
                .map(|idx| {
                    execute_single_with_authority(
                        registry,
                        &tool_calls[idx],
                        authority,
                        msg_id,
                        hooks_shared,
                        compaction_level,
                        toon_enabled,
                    )
                })
                .collect();
            let batch_results = futures::future::join_all(futures).await;
            for (offset, (block, modifier, nested_effects)) in
                batch_results.into_iter().enumerate()
            {
                let idx = group.start + offset;
                if let (
                    ContentBlock::ToolUse { id, name, .. },
                    ContentBlock::ToolResult {
                        content, is_error, ..
                    },
                ) = (&tool_calls[idx], &block)
                {
                    let status = if *is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Success
                    };
                    let _ = writer.emit(&ProtocolEvent::ToolResult {
                        msg_id: msg_id.to_string(),
                        call_id: id.clone(),
                        tool_name: name.clone(),
                        status,
                        output: content.clone(),
                        output_type: OutputType::Text,
                        metadata: None,
                    });
                }
                if block_is_error(&block) {
                    halt_after_error = true;
                }
                results.push(block);
                modifiers.push(modifier);
                delegated_effects.push(nested_effects);
            }
            continue;
        }

        // Serial path for stateful or non-concurrent tools.
        let call = &tool_calls[group.start];
        let ContentBlock::ToolUse { id, name, .. } = call
        else {
            continue;
        };

        // Fail synchronously before ToolRunning. Emit only the paired error
        // ToolResult.
        if let Some(gated) = invocation_gate_result(registry, call, authority) {
            emit_tool_result_event(writer, msg_id, call, &gated);
            halt_after_error = true;
            results.push(gated);
            modifiers.push(None);
            delegated_effects.push(Vec::new());
            continue;
        }

        let _ = writer.emit(&ProtocolEvent::ToolRunning {
            msg_id: msg_id.to_string(),
            call_id: id.clone(),
            tool_name: name.clone(),
        });

        // Execute the tool (reborrow as shared for execute_single, then reclaim mut for merge).
        let result;
        let modifier;
        let nested_effects;
        {
            let hooks_shared: Option<&HookEngine> = hooks.as_deref();
            (result, modifier, nested_effects) = execute_single_with_authority(
                registry,
                call,
                authority,
                msg_id,
                hooks_shared,
                compaction_level,
                toon_enabled,
            )
            .await;
        }

        emit_tool_result_event(writer, msg_id, call, &result);

        // Merge skill hooks after a successful execution.
        if !block_is_error(&result) {
            maybe_merge_skill_hooks(registry, call, hooks.as_deref_mut());
        } else {
            halt_after_error = true;
        }

        results.push(result);
        modifiers.push(modifier);
        delegated_effects.push(nested_effects);
    }

    Ok(ToolCallOutcome {
        results,
        modifiers,
        delegated_effects,
    })
}

/// If `call` is a Skill tool call that returned successfully, parse and merge
/// its declared hooks into the active HookEngine.
/// If `call` is a Skill tool call that returned successfully, merge skill hooks into the engine.
fn merge_skill_hooks_into(engine: &mut HookEngine, registry: &ToolRegistry, call: &ContentBlock) {
    let ContentBlock::ToolUse { name, input, .. } = call else {
        return;
    };
    if name != "Skill" {
        return;
    }
    let Some(tool) = registry.get(name) else {
        return;
    };
    if let Some(skill_hooks) = tool.skill_hooks_for(input) {
        engine.merge_hooks(skill_hooks);
    }
}

fn maybe_merge_skill_hooks(
    registry: &ToolRegistry,
    call: &ContentBlock,
    hooks: Option<&mut HookEngine>,
) {
    if let Some(engine) = hooks {
        merge_skill_hooks_into(engine, registry, call);
    }
}

/// Returns true when a ContentBlock::ToolResult has is_error=true.
fn block_is_error(block: &ContentBlock) -> bool {
    matches!(block, ContentBlock::ToolResult { is_error: true, .. })
}

fn truncate_result(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let half = max_chars / 2;
    // Find char boundaries to avoid panicking on multi-byte characters
    let head_end = content
        .char_indices()
        .nth(half)
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    let tail_start = content
        .char_indices()
        .rev()
        .nth(half - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let head = &content[..head_end];
    let tail = &content[tail_start..];
    format!(
        "{}\n\n... [truncated {} chars] ...\n\n{}",
        head,
        content.len() - max_chars,
        tail
    )
}

struct Batch<'a> {
    is_concurrent: bool,
    calls: Vec<&'a ContentBlock>,
}

fn partition<'a>(
    registry: &ToolRegistry,
    calls: &'a [ContentBlock],
    authority: &ProviderToolAuthority,
) -> Vec<Batch<'a>> {
    let mut batches: Vec<Batch<'a>> = Vec::new();

    for call in calls {
        let ContentBlock::ToolUse { name, input, .. } = call else {
            continue;
        };
        let is_safe = invocation_gate_result(registry, call, authority).is_none()
            && registry
                .get(name)
                .map(|t| t.is_concurrency_safe(input))
                .unwrap_or(false);

        match batches.last_mut() {
            Some(last) if last.is_concurrent && is_safe => {
                last.calls.push(call);
            }
            _ => {
                batches.push(Batch {
                    is_concurrent: is_safe,
                    calls: vec![call],
                });
            }
        }
    }

    batches
}

/// Group call indices for the protocol path: consecutive `batchable` calls
/// can execute in parallel; every other call is its own singleton range so its
/// keeps tool_use/tool_result pairing intact for the model. (Phase 2 tool-call)
fn group_batches(batchable: &[bool]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < batchable.len() {
        if batchable[i] {
            let start = i;
            while i < batchable.len() && batchable[i] {
                i += 1;
            }
            groups.push(start..i);
        } else {
            groups.push(i..i + 1);
            i += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture_logs(run: impl FnOnce()) -> String {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer_output = Arc::clone(&output);
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_writer(move || LogWriter(Arc::clone(&writer_output)))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            // A callsite may have been initialized before this test installed
            // its capture subscriber. Refresh interest before exercising logs.
            tracing::callsite::rebuild_interest_cache();
            run();
        });

        let bytes = output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        String::from_utf8(bytes).expect("test tracing output is UTF-8")
    }

    #[test]
    fn dropped_tool_future_logs_one_cancelled_completion() {
        use std::future::Future;
        use std::task::{Context, Poll};

        let logs = capture_logs(|| {
            let mut future = Box::pin(async {
                let mut execution_log = ToolExecutionLog::start("Glob", "call_cancelled");
                execution_log.enter("tool");
                std::future::pending::<()>().await;
            });
            let waker = futures::task::noop_waker();
            let mut context = Context::from_waker(&waker);
            assert_eq!(future.as_mut().poll(&mut context), Poll::Pending);
            drop(future);
        });

        assert_eq!(logs.matches("tool execution started").count(), 1, "{logs}");
        assert_eq!(logs.matches("tool execution completed").count(), 1, "{logs}");
        assert!(logs.contains("call_cancelled"), "{logs}");
        assert!(logs.contains("aborted_or_cancelled"), "{logs}");
        assert!(logs.contains("stage=\"tool\""), "{logs}");
    }

    #[test]
    fn explicitly_finished_tool_log_is_not_duplicated_on_drop() {
        let logs = capture_logs(|| {
            let mut execution_log = ToolExecutionLog::start("Glob", "call_success");
            execution_log.enter("finalize");
            execution_log.finish(true, "success");
        });

        assert_eq!(logs.matches("tool execution started").count(), 1, "{logs}");
        assert_eq!(logs.matches("tool execution completed").count(), 1, "{logs}");
        assert!(logs.contains("call_success"), "{logs}");
        assert!(logs.contains("outcome=\"success\""), "{logs}");
        assert!(!logs.contains("aborted_or_cancelled"), "{logs}");
    }

    #[test]
    fn group_batches_groups_consecutive_batchable_and_isolates_rest() {
        // [t,t,f,t] -> [0..2 concurrent][2..3 serial][3..4 serial]
        let groups = group_batches(&[true, true, false, true]);
        assert_eq!(groups, vec![0..2, 2..3, 3..4]);
    }

    #[test]
    fn group_batches_all_batchable_is_one_group() {
        assert_eq!(group_batches(&[true, true, true]), vec![0..3]);
    }

    #[test]
    fn group_batches_none_batchable_is_all_singletons() {
        assert_eq!(group_batches(&[false, false]), vec![0..1, 1..2]);
    }

    #[test]
    fn group_batches_empty_is_empty() {
        assert_eq!(group_batches(&[]), Vec::<std::ops::Range<usize>>::new());
    }
    use serde_json::json;

    // -- truncate_result ------------------------------------------------------

    #[test]
    fn truncate_result_short_unchanged() {
        let s = "short content";
        assert_eq!(truncate_result(s, 1000), s);
    }

    #[test]
    fn truncate_result_cjk_does_not_panic() {
        let cjk: String = "这是一段较长的中文内容用于测试截断功能".repeat(50);
        let result = truncate_result(&cjk, 100);
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_result_mixed_cjk_ascii_does_not_panic() {
        let mixed = "Hello你好World世界Test测试".repeat(100);
        let result = truncate_result(&mixed, 200);
        assert!(result.contains("truncated"));
    }

    // -- execute_single integration tests (deferred tool activation) ----------

    use nomi_tools::Tool;
    use nomi_tools::registry::ToolRegistry;

    struct ContextRecordingTool {
        operation_ids: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Tool for ContextRecordingTool {
        fn name(&self) -> &str {
            "ContextRecording"
        }

        fn description(&self) -> &str {
            "records engine-owned execution context"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            true
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
        ) -> nomi_types::tool::ToolResult {
            nomi_types::tool::ToolResult::error("execution context was bypassed")
        }

        async fn execute_with_context(
            &self,
            _input: serde_json::Value,
            context: &ToolExecutionContext,
        ) -> nomi_types::tool::ToolResult {
            self.operation_ids
                .lock()
                .unwrap()
                .push(context.operation_id().to_owned());
            nomi_types::tool::ToolResult::text("ok")
        }

        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Info
        }
    }

    #[tokio::test]
    async fn scoped_execution_context_reaches_tool_and_separates_reused_call_ids() {
        let operation_ids = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ContextRecordingTool {
            operation_ids: operation_ids.clone(),
        }));
        let authority = ProviderToolAuthority::from_request_tools(
            &registry.to_tool_defs(),
        );
        let call = ContentBlock::ToolUse {
            id: "call_0".into(),
            name: "ContextRecording".into(),
            input: json!({}),
            extra: None,
        };

        for scope in ["conversation-a:turn-1", "conversation-a:turn-2"] {
            let (result, _, _) = execute_single_with_authority(
                &registry,
                &call,
                &authority,
                scope,
                None,
                nomi_compact::CompactionLevel::Off,
                false,
            )
            .await;
            assert!(
                matches!(result, ContentBlock::ToolResult { is_error: false, .. })
            );
        }

        let recorded = operation_ids.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0],
            ToolExecutionContext::from_scoped_tool_call(
                "conversation-a:turn-1",
                "call_0"
            )
            .operation_id()
        );
        assert_ne!(recorded[0], recorded[1]);
    }

    struct MockDeferredTool {
        schema: serde_json::Value,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        concurrent_safe: bool,
    }

    #[async_trait::async_trait]
    impl Tool for MockDeferredTool {
        fn name(&self) -> &str {
            "MockDeferred"
        }
        fn description(&self) -> &str {
            "A mock deferred tool for testing"
        }
        fn input_schema(&self) -> serde_json::Value {
            self.schema.clone()
        }
        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            self.concurrent_safe
        }
        fn is_deferred(&self) -> bool {
            true
        }
        async fn execute(&self, input: serde_json::Value) -> nomi_types::tool::ToolResult {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if input.get("tasks").is_none() {
                return nomi_types::tool::ToolResult {
                    content: "Missing or invalid 'tasks' array".to_string(),
                    is_error: true,
                    images: Vec::new(),
                };
            }
            nomi_types::tool::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                images: Vec::new(),
            }
        }
        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Exec
        }
    }

    struct MockNonDeferredTool;

    #[async_trait::async_trait]
    impl Tool for MockNonDeferredTool {
        fn name(&self) -> &str {
            "MockNonDeferred"
        }
        fn description(&self) -> &str {
            "A mock non-deferred tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": { "cmd": { "type": "string" } },
                "required": ["cmd"]
            })
        }
        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            true
        }
        async fn execute(&self, input: serde_json::Value) -> nomi_types::tool::ToolResult {
            if input.get("cmd").and_then(serde_json::Value::as_str) == Some("fail") {
                return nomi_types::tool::ToolResult {
                    content: "Command failed".to_string(),
                    is_error: true,
                    images: Vec::new(),
                };
            }
            nomi_types::tool::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                images: Vec::new(),
            }
        }
        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Exec
        }
    }

    struct SchemaValidatedKnowledgeTool {
        dispatches: Arc<std::sync::atomic::AtomicUsize>,
        policy_calls: Arc<std::sync::atomic::AtomicUsize>,
        seen_inputs: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }

    #[async_trait::async_trait]
    impl Tool for SchemaValidatedKnowledgeTool {
        fn name(&self) -> &str {
            "mcp__knowledge__search__schemafixture"
        }
        fn reserved_provider_name_prefix(&self) -> Option<&'static str> {
            Some("mcp__")
        }
        fn activation_identity(&self) -> &str {
            "mcp:9:knowledge:6:search-schema-fixture"
        }
        fn description(&self) -> &str {
            "Knowledge/MCP schema validation fixture"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "kb_id": { "type": "string", "minLength": 1 },
                    "options": {
                        "type": "object",
                        "properties": {
                            "limit": { "type": "integer", "minimum": 1 },
                            "mode": { "enum": ["semantic", "keyword"] }
                        },
                        "required": ["limit", "mode"],
                        "additionalProperties": false
                    },
                    "scope": {
                        "oneOf": [
                            { "const": "all" },
                            {
                                "type": "object",
                                "properties": { "document_id": { "type": "string" } },
                                "required": ["document_id"],
                                "additionalProperties": false
                            }
                        ]
                    }
                },
                "required": ["kb_id", "options", "scope"],
                "additionalProperties": false
            })
        }
        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            self.policy_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        async fn execute(&self, input: serde_json::Value) -> nomi_types::tool::ToolResult {
            self.dispatches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.seen_inputs.lock().unwrap().push(input);
            nomi_types::tool::ToolResult::text("knowledge result")
        }
        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Info
        }
    }

    fn schema_validated_knowledge_registry() -> (
        ToolRegistry,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        let dispatches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let policy_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen_inputs = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::new();
        assert!(registry.register(Box::new(SchemaValidatedKnowledgeTool {
            dispatches: dispatches.clone(),
            policy_calls: policy_calls.clone(),
            seen_inputs: seen_inputs.clone(),
        })));
        (registry, dispatches, policy_calls, seen_inputs)
    }

    #[derive(Default)]
    struct CapturingEmitter {
        events: std::sync::Mutex<Vec<String>>,
    }

    impl CapturingEmitter {
        fn has_event_type(&self, event_type: &str) -> bool {
            let event_type = format!(r#""type":"{event_type}""#);
            self.events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.contains(&event_type))
        }

        fn has_event_for(&self, event_type: &str, call_id: &str) -> bool {
            let event_type = format!(r#""type":"{event_type}""#);
            let call_id = format!(r#""call_id":"{call_id}""#);
            self.events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.contains(&event_type) && event.contains(&call_id))
        }

        fn event_count(&self, event_type: &str) -> usize {
            let event_type = format!(r#""type":"{event_type}""#);
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.contains(&event_type))
                .count()
        }
    }

    impl nomi_protocol::writer::ProtocolEmitter for CapturingEmitter {
        fn emit(&self, event: &nomi_protocol::events::ProtocolEvent) -> std::io::Result<()> {
            let encoded = serde_json::to_string(event)
                .map_err(|e| std::io::Error::other(format!("serialize protocol event: {e}")))?;
            self.events.lock().unwrap().push(encoded);
            Ok(())
        }
    }



    #[tokio::test]
    async fn nested_type_enum_and_one_of_errors_are_local_and_valid_input_dispatches() {
        let (registry, dispatches, policy_calls, _seen_inputs) =
            schema_validated_knowledge_registry();
        let authority = ProviderToolAuthority::from_request_tools(&registry.to_tool_defs());
        let invalid = vec![ContentBlock::ToolUse {
            id: "nested-invalid".into(),
            name: "mcp__knowledge__search__schemafixture".into(),
            input: json!({
                "kb_id": "kb-1",
                "options": { "limit": "many", "mode": "hybrid" },
                "scope": { "wrong": true }
            }),
            extra: None,
        }];

        let invalid_outcome = execute_tool_calls_scoped(
            &registry,
            &invalid,
            &authority,
            "",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await
        .unwrap();
        assert!(matches!(
            &invalid_outcome.results[0],
            ContentBlock::ToolResult { content, is_error: true, .. }
                if content.contains("/options/limit")
                    && content.contains("/options/mode")
                    && content.contains("/scope")
        ));
        assert_eq!(dispatches.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(policy_calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let valid = vec![ContentBlock::ToolUse {
            id: "schema-valid".into(),
            name: "mcp__knowledge__search__schemafixture".into(),
            input: json!({
                "kb_id": "kb-1",
                "options": { "limit": 5, "mode": "semantic" },
                "scope": { "document_id": "doc-1" }
            }),
            extra: None,
        }];
        let valid_outcome = execute_tool_calls_scoped(
            &registry,
            &valid,
            &authority,
            "",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await
        .unwrap();
        assert!(matches!(
            &valid_outcome.results[0],
            ContentBlock::ToolResult { content, is_error: false, .. }
                if content == "knowledge result"
        ));
        assert_eq!(dispatches.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(policy_calls.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    fn make_registry_with_deferred_safety(
        concurrent_safe: bool,
    ) -> (ToolRegistry, Arc<std::sync::atomic::AtomicUsize>) {
        let mut registry = ToolRegistry::new();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        registry.register(Box::new(MockDeferredTool {
            schema: json!({
                "type": "object",
                "properties": { "tasks": { "type": "array" } },
                "required": ["tasks"]
            }),
            calls: calls.clone(),
            concurrent_safe,
        }));
        registry.register(Box::new(MockNonDeferredTool));
        (registry, calls)
    }

    fn make_registry_with_deferred() -> (ToolRegistry, Arc<std::sync::atomic::AtomicUsize>) {
        make_registry_with_deferred_safety(true)
    }

    fn deferred_call(id: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: "MockDeferred".into(),
            input: json!({"tasks": [{"name": "would_mutate"}]}),
            extra: None,
        }
    }




    struct MockSecretTool;
    #[async_trait::async_trait]
    impl Tool for MockSecretTool {
        fn name(&self) -> &str {
            "MockSecret"
        }
        fn description(&self) -> &str {
            "returns output containing a secret"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            false
        }
        async fn execute(&self, _input: serde_json::Value) -> nomi_types::tool::ToolResult {
            nomi_types::tool::ToolResult {
                content: "the key is sk-ABCDEFGHIJKLMNOPQRSTUVWX and that is all".to_string(),
                is_error: false,
                images: Vec::new(),
            }
        }
        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Info
        }
    }

    struct MockPanickingTool;
    #[async_trait::async_trait]
    impl Tool for MockPanickingTool {
        fn name(&self) -> &str {
            "MockPanic"
        }
        fn description(&self) -> &str {
            "panics during execution (simulates a tool bug / caught FFI unwind)"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object" })
        }
        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            false
        }
        async fn execute(&self, _input: serde_json::Value) -> nomi_types::tool::ToolResult {
            panic!("boom: simulated tool panic");
        }
        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Info
        }
    }

    struct DeadlineTool {
        name: &'static str,
        timeout: Duration,
        delay: Duration,
        concurrent_safe: bool,
        dispatches: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for DeadlineTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "deadline test tool"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({"type": "object"})
        }

        fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
            self.concurrent_safe
        }

        fn execution_timeout(&self, _input: &serde_json::Value) -> Duration {
            self.timeout
        }

        async fn execute(&self, _input: serde_json::Value) -> nomi_types::tool::ToolResult {
            self.dispatches
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            nomi_types::tool::ToolResult::text(self.name)
        }

        fn category(&self) -> nomi_protocol::events::ToolCategory {
            nomi_protocol::events::ToolCategory::Info
        }
    }

    #[tokio::test]
    async fn tool_execution_deadline_returns_fail_closed_error_with_original_call_id() {
        let dispatches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DeadlineTool {
            name: "DeadlineTool",
            timeout: Duration::from_millis(10),
            delay: Duration::from_secs(60),
            concurrent_safe: false,
            dispatches: dispatches.clone(),
        }));
        let call = ContentBlock::ToolUse {
            id: "deadline-call".into(),
            name: "DeadlineTool".into(),
            input: json!({}),
            extra: None,
        };
        let authority = ProviderToolAuthority::from_request_tools(&registry.to_tool_defs());

        let (result, modifier, delegated_effects) = execute_single_with_authority(
            &registry,
            &call,
            &authority,
            "deadline-test",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;

        assert!(modifier.is_none());
        assert!(delegated_effects.is_empty());
        assert!(matches!(
            result,
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
                ..
            } if tool_use_id == "deadline-call"
                && content.contains("timed out")
                && content.contains("do not assume")
        ));
        assert_eq!(
            dispatches.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the timeout result must not claim that dispatch never started"
        );
    }


    #[tokio::test]
    async fn execute_single_redacts_secrets_in_output() {
        // Secrets in tool output must never reach the model/provider verbatim.
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockSecretTool));
        let call = ContentBlock::ToolUse {
            id: "c".into(),
            name: "MockSecret".into(),
            input: json!({}),
            extra: None,
        };
        let (result, _, _) = execute_single(
            &registry,
            &call,
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;
        if let ContentBlock::ToolResult { content, .. } = &result {
            assert!(
                !content.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"),
                "secret must be redacted, got: {content}"
            );
            assert!(content.contains("REDACTED"), "should show a redaction placeholder: {content}");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[tokio::test]
    async fn execute_single_recovers_from_a_panicking_tool() {
        // A panic inside a tool's execute() must be caught and surfaced as an
        // error ToolResult fed back to the model — NOT unwind out of the agent
        // loop. nomi-cli awaits engine.execute_turn() directly on the #[tokio::main]
        // task with no catch_unwind above it, so an unguarded tool panic would
        // terminate the whole agent subprocess.
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockPanickingTool));
        let call = ContentBlock::ToolUse {
            id: "cp".into(),
            name: "MockPanic".into(),
            input: json!({}),
            extra: None,
        };
        let (result, modifier, delegated_effects) = execute_single(
            &registry,
            &call,
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;
        assert!(modifier.is_none());
        assert!(delegated_effects.is_empty());
        if let ContentBlock::ToolResult { content, is_error, .. } = &result {
            assert!(is_error, "a panicking tool must yield an error result");
            assert!(
                content.to_lowercase().contains("panic"),
                "recovered result should mention the panic, got: {content}"
            );
        } else {
            panic!("expected ToolResult");
        }
    }



    #[tokio::test]
    async fn unactivated_deferred_tool_is_blocked_before_dispatch() {
        let (registry, calls) = make_registry_with_deferred();
        let call = ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "MockDeferred".into(),
            input: json!({"tasks": [{"name": "would_mutate"}]}),
            extra: None,
        };
        let (result, _, _) = execute_single(
            &registry,
            &call,
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(content.contains("ToolSearch"));
            assert!(content.contains("subsequent model turn"));
            assert!(content.contains("not executed"));
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        } else {
            panic!("expected ToolResult");
        }
    }

    #[tokio::test]
    async fn tool_search_and_target_in_same_model_turn_cannot_bypass_gate() {
        let (mut registry, calls) = make_registry_with_deferred();
        let state = registry.deferred_state();
        registry.register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(
            state,
        )));
        let tool_calls = vec![
            ContentBlock::ToolUse {
                id: "search".into(),
                name: "ToolSearch".into(),
                input: json!({"query": "MockDeferred"}),
                extra: None,
            },
            ContentBlock::ToolUse {
                id: "target".into(),
                name: "MockDeferred".into(),
                input: json!({"tasks": [{"name": "would_mutate"}]}),
                extra: None,
            },
        ];
        let outcome = execute_tool_calls_scoped(
            &registry,
            &tool_calls,
            &ProviderToolAuthority::from_request_tools(&registry.to_tool_defs()),
            "",
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await
        .unwrap();

        assert_eq!(outcome.results.len(), 2);
        assert!(matches!(
            &outcome.results[0],
            ContentBlock::ToolResult { is_error: false, .. }
        ));
        assert!(matches!(
            &outcome.results[1],
            ContentBlock::ToolResult { content, is_error: true, .. }
                if content.contains("subsequent model turn")
        ));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!registry
            .provider_deferred_tool_names()
            .contains("MockDeferred"));
    }


    #[tokio::test]
    async fn protocol_concurrent_safe_deferred_calls_never_emit_running() {
        let (registry, calls) = make_registry_with_deferred();
        let writer_capture = Arc::new(CapturingEmitter::default());
        let writer: Arc<dyn nomi_protocol::writer::ProtocolEmitter> = writer_capture.clone();
        let tool_calls = vec![
            deferred_call("protocol-concurrent-deferred-1"),
            deferred_call("protocol-concurrent-deferred-2"),
        ];

        // With no deferred preflight these two concurrency-safe calls would be
        // grouped and ToolRunning would be emitted before execute_single.
        let outcome = execute_tool_calls_with_protocol(&registry, &tool_calls, &ProviderToolAuthority::from_request_tools(&registry.to_tool_defs()), &writer, "msg-protocol-concurrent", None, nomi_compact::CompactionLevel::Off, false)
        .await
        .unwrap();

        assert_eq!(outcome.results.len(), 2);
        assert!(!writer_capture.has_event_type("tool_running"));
        assert_eq!(writer_capture.event_count("tool_result"), 2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn protocol_tool_search_and_target_same_turn_uses_frozen_gate_snapshot() {
        let (mut registry, calls) = make_registry_with_deferred();
        let state = registry.deferred_state();
        registry.register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(
            state,
        )));
        let writer_capture = Arc::new(CapturingEmitter::default());
        let writer: Arc<dyn nomi_protocol::writer::ProtocolEmitter> = writer_capture.clone();
        let tool_calls = vec![
            ContentBlock::ToolUse {
                id: "protocol-search".into(),
                name: "ToolSearch".into(),
                input: json!({"query": "MockDeferred"}),
                extra: None,
            },
            deferred_call("protocol-target"),
        ];

        let outcome = execute_tool_calls_with_protocol(&registry, &tool_calls, &ProviderToolAuthority::from_request_tools(&registry.to_tool_defs()), &writer, "msg-protocol-frozen-gate", None, nomi_compact::CompactionLevel::Off, false)
        .await
        .unwrap();

        assert_eq!(outcome.results.len(), 2);
        assert!(matches!(
            &outcome.results[0],
            ContentBlock::ToolResult { is_error: false, .. }
        ));
        assert!(matches!(
            &outcome.results[1],
            ContentBlock::ToolResult { content, is_error: true, .. }
                if content.contains("subsequent model turn")
        ));
        assert!(writer_capture.has_event_for("tool_running", "protocol-search"));
        assert!(!writer_capture.has_event_for("tool_running", "protocol-target"));
        assert!(writer_capture.has_event_for("tool_result", "protocol-target"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!registry
            .provider_deferred_tool_names()
            .contains("MockDeferred"));
    }

    #[tokio::test]
    async fn activated_deferred_tool_dispatches_successfully() {
        let (registry, calls) = make_registry_with_deferred();
        let search = nomi_tools::tool_search::ToolSearchTool::new(registry.deferred_state());
        assert!(
            !search
                .execute(json!({"query": "MockDeferred"}))
                .await
                .is_error
        );
        let call = ContentBlock::ToolUse {
            id: "call_3".into(),
            name: "MockDeferred".into(),
            input: json!({"tasks": [{"name": "t1", "prompt": "do x"}]}),
            extra: None,
        };
        let (result, _, _) = execute_single(
            &registry,
            &call,
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(!is_error);
            assert_eq!(content, "ok");
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        } else {
            panic!("expected ToolResult");
        }
    }

    #[tokio::test]
    async fn execute_single_non_deferred_tool_error_no_hint() {
        let (registry, _calls) = make_registry_with_deferred();
        let call = ContentBlock::ToolUse {
            id: "call_4".into(),
            name: "MockNonDeferred".into(),
            input: json!({"cmd": "fail"}),
            extra: None,
        };
        let (result, _, _) = execute_single(
            &registry,
            &call,
            None,
            nomi_compact::CompactionLevel::Off,
            false,
        )
        .await;
        if let ContentBlock::ToolResult {
            content, is_error, ..
        } = &result
        {
            assert!(is_error);
            assert!(content.contains("Command failed"));
            assert!(!content.contains("ToolSearch"));
        } else {
            panic!("expected ToolResult");
        }
    }
}
