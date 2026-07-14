use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use nomi_config::config::Config;
use nomi_process_runtime::{CapabilityPolicy, ProcessSupervisor, SupervisorConfig};
use nomi_providers::LlmProvider;
use nomi_tools::bash::BashTool;
use nomi_tools::edit::EditTool;
use nomi_tools::glob::GlobTool;
use nomi_tools::grep::GrepTool;
use nomi_tools::read::ReadTool;
use nomi_tools::registry::ToolRegistry;
use nomi_tools::write::WriteTool;
use nomi_types::message::TokenUsage;

use crate::context_contributor::ContextContributor;
use crate::engine::AgentEngine;
use crate::local_delegation_progress::{
    EmbeddedExecutionProgress, SiblingProgressContributor,
};
use crate::output::OutputSink;
use crate::output::null_sink::NullSink;

use nomi_types::agent::{
    AgentInvocationInput, AgentInvocationOutput, AgentInvocationRunner, AgentToolPolicy,
};

/// Local implementation of the shared one-Agent invocation primitive.
///
/// Invoked Agents use a [`NullSink`] so their streaming output is silently
/// discarded. Results are collected via `engine.execute_turn()` and returned to
/// the embedded execution caller above this runner.
pub(crate) struct LocalAgentInvocationRunner {
    provider: Arc<dyn LlmProvider>,
    base_config: Config,
    cwd: PathBuf,
    /// Shared across invocation clones so the concurrency cap is global to this
    /// backend (all fan-outs draw from the same permit pool).
    concurrency: Arc<Semaphore>,
    /// Optional shared token ceiling (opt-in; None = uncapped).
    token_budget: Option<Arc<TokenBudget>>,
    process_capability: CapabilityPolicy,
    write_root: Option<PathBuf>,
    parent_tool_scope: ToolScope,
}

impl LocalAgentInvocationRunner {
    pub(crate) fn new(provider: Arc<dyn LlmProvider>, config: Config, cwd: PathBuf) -> Self {
        let parent_tool_scope = ToolScope::from_config(&config.tools.builtin_allowlist);
        let process_capability = CapabilityPolicy::local_owner(cwd.clone());
        Self {
            provider,
            base_config: config,
            cwd,
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_DELEGATIONS)),
            token_budget: None,
            process_capability,
            write_root: None,
            parent_tool_scope,
        }
    }

    pub(crate) fn with_process_capability(
        mut self,
        capability: CapabilityPolicy,
        write_root: Option<PathBuf>,
        parent_allowlist: Vec<String>,
    ) -> Self {
        self.process_capability = capability;
        self.write_root = write_root;
        self.parent_tool_scope = ToolScope::from_config(&parent_allowlist);
        self
    }

    /// Enable a shared cumulative token ceiling for all delegated Agents (§3.4).
    pub(crate) fn with_token_budget(mut self, budget: Option<Arc<TokenBudget>>) -> Self {
        self.token_budget = budget;
        self
    }

    /// Execute a single delegated Agent and await its result.
    pub(crate) async fn invoke_one(&self, input: AgentInvocationInput) -> AgentInvocationOutput {
        self.invoke_with_context(input, None).await
    }

    /// Execute one delegated Agent with optional host-provided per-turn
    /// coordination context. Coordination is an engine detail, never another
    /// model tool or task lifecycle.
    async fn invoke_with_context(
        &self,
        invocation: AgentInvocationInput,
        contributor: Option<Arc<dyn ContextContributor>>,
    ) -> AgentInvocationOutput {
        let effective_scope = self.effective_tool_scope(&invocation);
        self.invoke_with_effective_scope(invocation, contributor, effective_scope)
            .await
    }

    async fn invoke_with_effective_scope(
        &self,
        invocation: AgentInvocationInput,
        contributor: Option<Arc<dyn ContextContributor>>,
        effective_scope: ToolScope,
    ) -> AgentInvocationOutput {
        // Shared token ceiling (opt-in): refuse to start a new delegated Agent once
        // the budget is spent, rather than launching work that won't be paid for.
        if let Some(budget) = &self.token_budget
            && !budget.can_begin()
        {
            return AgentInvocationOutput {
                name: invocation.name,
                text: "Delegated Agent skipped: shared token budget exhausted.".to_string(),
                usage: TokenUsage::default(),
                turns: 0,
                is_error: true,
            };
        }

        let config = self.config_for_invocation(&invocation);

        tracing::info!(target: "nomi_agent", cwd = %self.cwd.display(), "delegated Agent invocation started");

        let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
        let tools = match build_tool_registry(
            effective_scope,
            &self.cwd,
            &self.process_capability,
            self.write_root.as_deref(),
            Arc::clone(&supervisor),
        ) {
            Ok(tools) => tools,
            Err(error) => {
                return AgentInvocationOutput {
                    name: invocation.name,
                    text: format!("Delegated Agent capability denied: {error}"),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                };
            }
        };
        let output: Arc<dyn OutputSink> = Arc::new(NullSink);
        let mut engine = AgentEngine::new_with_provider(
            self.provider.clone(),
            config,
            tools,
            output,
            self.cwd.clone(),
        );
        engine.set_process_supervisor(Arc::clone(&supervisor));
        engine.set_initial_reasoning_effort(invocation.effort.clone());
        if let Some(contributor) = contributor {
            engine.register_context_contributor(contributor);
        }

        let result =
            execute_delegated_agent(engine, invocation.name, &invocation.prompt, supervisor).await;
        // Charge the shared budget for what this delegated Agent actually consumed.
        if let Some(budget) = &self.token_budget {
            budget.record(result.usage.input_tokens + result.usage.output_tokens);
        }
        result
    }

    fn effective_tool_scope(&self, invocation: &AgentInvocationInput) -> ToolScope {
        self.parent_tool_scope
            .intersect(&ToolScope::from_policy(invocation.tool_policy))
            .intersect(&ToolScope::from_exact_tools(&invocation.exact_tools))
    }

    fn config_for_invocation(&self, invocation: &AgentInvocationInput) -> Config {
        let mut config = self.base_config.clone();
        config.max_turns = Some(invocation.max_turns);
        config.max_tokens = invocation.max_tokens;
        if let Some(system_prompt) = invocation.system_prompt.clone() {
            config.system_prompt = Some(system_prompt);
        }
        if let Some(model) = invocation.model.clone() {
            config.model = model;
        }
        config.session.enabled = false;
        // Hooks are executable host policy, not AgentInvocation authority. The
        // child registry is capability-scoped, while inherited hooks currently
        // execute raw shell commands outside that registry and its supervisor.
        // Crossing this boundary would make read_only and synthesis Agents
        // mutation-capable, so delegated executions start with no parent hooks.
        config.hooks = Default::default();
        config
    }

    /// Execute one embedded Agent fan-out under a deterministic workspace plan.
    ///
    /// Read-only tasks always share the caller's workspace. A single
    /// mutation-capable task also keeps direct-write behavior. When two or more
    /// siblings can mutate, only those siblings receive detached worktrees and
    /// return patches; this prevents clobbering without imposing isolation on
    /// ordinary reads or one-Agent edits. The private progress ledger is
    /// orthogonal to workspace placement and follows every sibling.
    pub(crate) async fn execute_fanout(
        &self,
        invocations: Vec<AgentInvocationInput>,
    ) -> Vec<AgentInvocationOutput> {
        let effective_scopes = invocations
            .iter()
            .map(|invocation| self.effective_tool_scope(invocation))
            .collect::<Vec<_>>();
        let workspace_plan = FanoutWorkspacePlan::for_scopes(
            &effective_scopes,
            nomi_tools::worktree::is_git_repo(&self.cwd),
        );
        if workspace_plan.warn_shared_mutation {
            tracing::warn!(
                target: "nomi_agent",
                cwd = %self.cwd.display(),
                "parallel mutation-capable Agents cannot use worktree isolation because the workspace is not a Git repository"
            );
        }
        let worktree_baseline = if workspace_plan.isolate_mutating {
            let cwd = self.cwd.clone();
            Some(
                match tokio::task::spawn_blocking(move || {
                    nomi_tools::worktree::WorktreeBaseline::capture(&cwd)
                })
                .await
                {
                    Ok(Ok(baseline)) => Ok(Arc::new(baseline)),
                    Ok(Err(error)) => Err(Arc::new(error)),
                    Err(error) => Err(Arc::new(format!(
                        "source workspace snapshot task failed: {error}"
                    ))),
                },
            )
        } else {
            None
        };
        let progress = (invocations.len() >= 2)
            .then(|| EmbeddedExecutionProgress::register(&invocations));
        let tasks: Vec<_> = invocations
            .into_iter()
            .zip(effective_scopes)
            .enumerate()
            .map(|(index, (config, effective_scope))| {
                let runner = self.clone_for_invocation();
                let progress = progress.clone();
                let worktree_baseline = worktree_baseline.clone();
                let isolate = workspace_plan.isolates(&effective_scope);
                let warn = workspace_plan.warns(&effective_scope);
                async move {
                    let run = progress.as_ref().map(|progress| progress.begin(index));
                    let contributor = progress.as_ref().map(|progress| {
                        Arc::new(SiblingProgressContributor::new(
                            Arc::clone(progress),
                            index,
                        )) as Arc<dyn ContextContributor>
                    });
                    let mut output = if isolate {
                        match worktree_baseline
                            .as_ref()
                            .expect("isolation plan always captures one source baseline")
                        {
                            Ok(baseline) => {
                                runner
                                    .invoke_isolated_one(
                                        config,
                                        contributor,
                                        effective_scope,
                                        Arc::clone(baseline),
                                    )
                                    .await
                            }
                            Err(error) => AgentInvocationOutput {
                                name: config.name,
                                text: format!(
                                    "Worktree isolation source snapshot failed: {}",
                                    error.as_str()
                                ),
                                usage: TokenUsage::default(),
                                turns: 0,
                                is_error: true,
                            },
                        }
                    } else {
                        runner
                            .invoke_with_effective_scope(config, contributor, effective_scope)
                            .await
                    };
                    if warn {
                        append_shared_mutation_warning(&mut output);
                    }
                    if let Some(run) = run {
                        run.finish(&output);
                    }
                    output
                }
            })
            .collect();

        execute_bounded(self.concurrency.clone(), tasks)
            .await
            .into_iter()
            .map(|joined| {
                joined.unwrap_or_else(|e| AgentInvocationOutput {
                    name: "unknown".to_string(),
                    text: format!("Task join error: {}", e),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                })
            })
            .collect()
    }

    async fn invoke_isolated_one(
        &self,
        config: AgentInvocationInput,
        contributor: Option<Arc<dyn ContextContributor>>,
        effective_scope: ToolScope,
        baseline: Arc<nomi_tools::worktree::WorktreeBaseline>,
    ) -> AgentInvocationOutput {
        let worktree = match tokio::task::spawn_blocking(move || baseline.create()).await {
            Ok(Ok(worktree)) => worktree,
            Ok(Err(error)) => {
                return AgentInvocationOutput {
                    name: config.name,
                    text: format!("Worktree isolation failed: {error}"),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                };
            }
            Err(error) => {
                return AgentInvocationOutput {
                    name: config.name,
                    text: format!("Worktree isolation task failed: {error}"),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                };
            }
        };
        let backend = match self.clone_for_isolated_worktree(&worktree) {
            Ok(backend) => backend,
            Err(error) => {
                let _ = tokio::task::spawn_blocking(move || drop(worktree)).await;
                return AgentInvocationOutput {
                    name: config.name,
                    text: format!("Worktree isolation capability setup failed: {error}"),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                };
            }
        };
        let mut result = backend
            .invoke_with_effective_scope(config, contributor, effective_scope)
            .await;
        match tokio::task::spawn_blocking(move || worktree.capture_diff()).await {
            Ok(Ok(diff)) if !diff.trim().is_empty() => {
                result.text.push_str(
                    "\n\n--- Changes from the isolated workspace; review and apply this patch in the parent workspace ---\n",
                );
                result.text.push_str(&diff);
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                result.text.push_str(&format!(
                    "\n\nERROR: The isolated workspace changes could not be captured: {error}"
                ));
                result.is_error = true;
            }
            Err(error) => {
                result.text.push_str(&format!(
                    "\n\nERROR: The isolated workspace capture task failed: {error}"
                ));
                result.is_error = true;
            }
        }
        result
    }

    fn clone_for_invocation(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            base_config: self.base_config.clone(),
            cwd: self.cwd.clone(),
            concurrency: self.concurrency.clone(),
            token_budget: self.token_budget.clone(),
            process_capability: self.process_capability.clone(),
            write_root: self.write_root.clone(),
            parent_tool_scope: self.parent_tool_scope.clone(),
        }
    }

    /// Delegate the parent's repository-scoped authority to the exact detached
    /// worktree represented by `worktree`.
    ///
    /// The path is accepted only after [`nomi_tools::worktree::Worktree`]
    /// verifies its live Git registration. Capability and write roots are then
    /// translated relative to the source workspace instead of granting the
    /// worktree's parent directory.
    fn clone_for_isolated_worktree(
        &self,
        worktree: &nomi_tools::worktree::Worktree,
    ) -> Result<Self, String> {
        let source = self
            .cwd
            .canonicalize()
            .map_err(|error| format!("invalid source workspace {}: {error}", self.cwd.display()))?;
        if !self.process_capability.cwd_roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|root| source.starts_with(root))
        }) {
            return Err(format!(
                "source workspace {} is outside inherited capability roots",
                source.display()
            ));
        }
        let cwd = worktree.verified_cwd(&source)?;
        let mut child = self.clone_for_invocation();
        child.cwd = cwd.clone();
        child.process_capability.cwd_roots = vec![cwd.clone()];
        child.write_root = Some(match &self.write_root {
            Some(root) => translate_root_to_worktree(&source, &cwd, root, "write")?,
            None => cwd.clone(),
        });
        if let nomi_process_runtime::SandboxPolicy::MacSeatbelt { write_roots } =
            &mut child.process_capability.sandbox
        {
            let mut translated = Vec::with_capacity(write_roots.len());
            for root in write_roots.iter() {
                translated.push(translate_root_to_worktree(
                    &source,
                    &cwd,
                    root,
                    "Seatbelt write",
                )?);
            }
            *write_roots = translated;
        }
        Ok(child)
    }

}

#[async_trait]
impl AgentInvocationRunner for LocalAgentInvocationRunner {
    async fn invoke(&self, input: AgentInvocationInput) -> AgentInvocationOutput {
        self.invoke_one(input).await
    }
}

/// Wall-clock cap for a single delegated Agent invocation. Without this, an Agent stuck
/// in a tool-call loop (up to its max_turns) or on a hung provider stream blocks
/// the parent indefinitely — a production hang risk. (Phase 2 multi-agent)
const DELEGATED_AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Global cap on concurrently-running delegated Agents across all fan-outs from one
/// backend. Without it a single `Delegate` call with N tasks (or several concurrent
/// `Delegate` calls sharing the backend) starts N engines/provider streams at
/// once — a resource and provider-rate-limit storm. (Phase 2 multi-agent §3.4)
const MAX_CONCURRENT_DELEGATIONS: usize = 8;

/// Optional cumulative token ceiling shared across every delegated Agent of a backend
/// (§3.4 "共享 token 预算"). Opt-in: absent → uncapped (default, zero change).
/// A *soft* ceiling — concurrent delegated Agents each pass `can_begin` before any of
/// them `record`s, so one fan-out may slightly overshoot; its job is to bound
/// cumulative spend across successive `Delegate` calls / nested fan-outs, not to
/// be an exact per-call limit.
pub(crate) struct TokenBudget {
    remaining: std::sync::atomic::AtomicU64,
}

impl TokenBudget {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicU64::new(limit),
        }
    }

    /// Whether there is budget left to start another delegated Agent.
    fn can_begin(&self) -> bool {
        self.remaining.load(std::sync::atomic::Ordering::Relaxed) > 0
    }

    /// Deduct `tokens` from the remaining budget (saturating at 0).
    fn record(&self, tokens: u64) {
        let _ = self.remaining.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |cur| Some(cur.saturating_sub(tokens)),
        );
    }

    #[cfg(test)]
    fn remaining(&self) -> u64 {
        self.remaining.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Execute `tasks` concurrently with at most `semaphore`'s permits in flight at
/// once. Each task holds a permit for its whole run; excess tasks queue until a
/// slot frees. Returns each task's output (or its `JoinError` if the task
/// panicked) — successful results in input order, the rare panics appended.
///
/// Uses a `JoinSet`, so if THIS future is dropped (e.g. the parent turn is
/// cancelled while a fan-out is in flight) every still-running delegated Agent task is
/// aborted instead of detaching and burning compute/tokens unattended.
async fn execute_bounded<F, T>(
    semaphore: Arc<Semaphore>,
    tasks: Vec<F>,
) -> Vec<Result<T, tokio::task::JoinError>>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let n = tasks.len();
    let mut set = tokio::task::JoinSet::new();
    for (idx, task) in tasks.into_iter().enumerate() {
        let sem = semaphore.clone();
        set.spawn(async move {
            // Held for the task's whole lifetime; dropped on completion to free
            // the slot for a queued task.
            let _permit = sem.acquire_owned().await.expect("delegation semaphore never closed");
            (idx, task.await)
        });
    }

    let mut slots: Vec<Option<T>> = (0..n).map(|_| None).collect();
    let mut panics: Vec<tokio::task::JoinError> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((idx, val)) => slots[idx] = Some(val),
            // A panicked task loses its index; collect its error and append it.
            Err(e) => panics.push(e),
        }
    }

    let mut results: Vec<Result<T, tokio::task::JoinError>> = Vec::with_capacity(n);
    for value in slots.into_iter().flatten() {
        results.push(Ok(value));
    }
    results.extend(panics.into_iter().map(Err));
    results
}

/// Map a timeout-wrapped turn outcome to an AgentInvocationOutput. Extracted so
/// the timeout/error/success mapping is unit-testable without a live engine.
fn map_agent_invocation_outcome(
    name: String,
    outcome: Result<
        Result<crate::engine::AgentResult, crate::engine::AgentError>,
        tokio::time::error::Elapsed,
    >,
    timeout_secs: u64,
) -> AgentInvocationOutput {
    match outcome {
        Ok(Ok(result)) => AgentInvocationOutput {
            name,
            text: result.text,
            usage: result.usage,
            turns: result.turns,
            is_error: false,
        },
        Ok(Err(e)) => AgentInvocationOutput {
            name,
            text: format!("Delegated Agent error: {}", e),
            usage: TokenUsage::default(),
            turns: 0,
            is_error: true,
        },
        Err(_elapsed) => AgentInvocationOutput {
            name,
            text: format!("Delegated Agent timed out after {timeout_secs}s and was aborted"),
            usage: TokenUsage::default(),
            turns: 0,
            is_error: true,
        },
    }
}

/// Run an engine to completion under a wall-clock timeout, mapping the outcome.
async fn execute_delegated_agent(
    mut engine: AgentEngine,
    name: String,
    prompt: &str,
    supervisor: Arc<ProcessSupervisor>,
) -> AgentInvocationOutput {
    let mut shutdown_guard = SupervisorShutdownOnDrop::new(Arc::clone(&supervisor));
    let outcome = tokio::time::timeout(DELEGATED_AGENT_TIMEOUT, engine.execute_turn(prompt, "")).await;
    let mut result = map_agent_invocation_outcome(name, outcome, DELEGATED_AGENT_TIMEOUT.as_secs());
    let shutdown = supervisor.shutdown().await;
    shutdown_guard.disarm();
    if shutdown.sessions.is_empty() {
        result
    } else {
        let unresolved = shutdown.sessions.iter().any(|session| {
            matches!(
                &session.outcome,
                nomi_process_runtime::ProcessOutcome::Lost { cleanup, .. } if !cleanup.reaped
            )
        });
        result.text.push_str(&format!(
            "\n\nSupervised process cleanup retired {} active session(s).",
            shutdown.sessions.len()
        ));
        if unresolved {
            result
                .text
                .push_str(" At least one process tree could not be proven reaped.");
            result.is_error = true;
        }
        result
    }
}

struct SupervisorShutdownOnDrop {
    supervisor: Arc<ProcessSupervisor>,
    armed: bool,
}

impl SupervisorShutdownOnDrop {
    fn new(supervisor: Arc<ProcessSupervisor>) -> Self {
        Self {
            supervisor,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SupervisorShutdownOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        enqueue_supervisor_shutdown(Arc::clone(&self.supervisor));
    }
}

fn enqueue_supervisor_shutdown(supervisor: Arc<ProcessSupervisor>) {
    use std::sync::OnceLock;
    use tokio::sync::mpsc;

    static CLEANUP_RELAY: OnceLock<mpsc::UnboundedSender<Arc<ProcessSupervisor>>> =
        OnceLock::new();
    let relay = CLEANUP_RELAY.get_or_init(|| {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Arc<ProcessSupervisor>>();
        std::thread::Builder::new()
            .name("nomi-delegation-process-cleanup".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build delegated Agent process cleanup runtime");
                runtime.block_on(async move {
                    while let Some(supervisor) = receiver.recv().await {
                        let _ = supervisor.shutdown().await;
                    }
                });
            })
            .expect("start delegated Agent process cleanup relay");
        sender
    });
    let _ = relay.send(supervisor);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FanoutWorkspacePlan {
    isolate_mutating: bool,
    warn_shared_mutation: bool,
}

impl FanoutWorkspacePlan {
    fn for_scopes(scopes: &[ToolScope], is_git_repo: bool) -> Self {
        let mutation_capable = scopes
            .iter()
            .filter(|scope| scope.may_mutate_workspace())
            .count();
        let has_parallel_mutation = mutation_capable >= 2;
        Self {
            isolate_mutating: has_parallel_mutation && is_git_repo,
            warn_shared_mutation: has_parallel_mutation && !is_git_repo,
        }
    }

    fn isolates(self, scope: &ToolScope) -> bool {
        self.isolate_mutating && scope.may_mutate_workspace()
    }

    fn warns(self, scope: &ToolScope) -> bool {
        self.warn_shared_mutation && scope.may_mutate_workspace()
    }
}

fn append_shared_mutation_warning(output: &mut AgentInvocationOutput) {
    output.text.push_str(
        "\n\nWARNING: This mutation-capable Agent shared the parent workspace because the workspace is not a Git repository. Concurrent writes may conflict; review the resulting workspace state before relying on this output.",
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceEffect {
    ReadOnly,
    MayMutate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildToolKind {
    Read,
    Write,
    Edit,
    Bash,
    Grep,
    Glob,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildToolDescriptor {
    kind: ChildToolKind,
    name: &'static str,
    workspace_effect: WorkspaceEffect,
}

const CHILD_TOOL_CATALOG: [ChildToolDescriptor; 6] = [
    ChildToolDescriptor {
        kind: ChildToolKind::Read,
        name: "Read",
        workspace_effect: WorkspaceEffect::ReadOnly,
    },
    ChildToolDescriptor {
        kind: ChildToolKind::Write,
        name: "Write",
        workspace_effect: WorkspaceEffect::MayMutate,
    },
    ChildToolDescriptor {
        kind: ChildToolKind::Edit,
        name: "Edit",
        workspace_effect: WorkspaceEffect::MayMutate,
    },
    ChildToolDescriptor {
        kind: ChildToolKind::Bash,
        name: "Bash",
        workspace_effect: WorkspaceEffect::MayMutate,
    },
    ChildToolDescriptor {
        kind: ChildToolKind::Grep,
        name: "Grep",
        workspace_effect: WorkspaceEffect::ReadOnly,
    },
    ChildToolDescriptor {
        kind: ChildToolKind::Glob,
        name: "Glob",
        workspace_effect: WorkspaceEffect::ReadOnly,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum ToolScope {
    Unrestricted,
    Restricted(std::collections::BTreeSet<String>),
}

impl ToolScope {
    fn from_config(allowed: &[String]) -> Self {
        if allowed.is_empty() {
            Self::Unrestricted
        } else {
            Self::Restricted(allowed.iter().cloned().collect())
        }
    }

    fn from_exact_tools(allowed: &[String]) -> Self {
        Self::from_config(allowed)
    }

    fn from_policy(policy: AgentToolPolicy) -> Self {
        if policy == AgentToolPolicy::Full {
            return Self::Unrestricted;
        }
        Self::Restricted(
            CHILD_TOOL_CATALOG
                .iter()
                .filter(|descriptor| match policy {
                    AgentToolPolicy::Full => true,
                    AgentToolPolicy::ReadOnly => {
                        descriptor.workspace_effect == WorkspaceEffect::ReadOnly
                    }
                    AgentToolPolicy::ReadShell => {
                        descriptor.workspace_effect == WorkspaceEffect::ReadOnly
                            || descriptor.kind == ChildToolKind::Bash
                    }
                })
                .map(|descriptor| descriptor.name.to_owned())
                .collect(),
        )
    }

    fn intersect(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Unrestricted, scope) | (scope, Self::Unrestricted) => scope.clone(),
            (Self::Restricted(left), Self::Restricted(right)) => {
                Self::Restricted(left.intersection(right).cloned().collect())
            }
        }
    }

    fn allows(&self, name: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(allowed) => allowed.contains(name),
        }
    }

    fn may_mutate_workspace(&self) -> bool {
        CHILD_TOOL_CATALOG.iter().any(|descriptor| {
            descriptor.workspace_effect == WorkspaceEffect::MayMutate
                && self.allows(descriptor.name)
        })
    }
}

fn build_tool_registry(
    scope: ToolScope,
    cwd: &Path,
    parent_capability: &CapabilityPolicy,
    write_root: Option<&Path>,
    supervisor: Arc<ProcessSupervisor>,
) -> Result<ToolRegistry, String> {
    let cwd = cwd
        .canonicalize()
        .map_err(|error| format!("invalid child cwd {}: {error}", cwd.display()))?;
    let mut capability = parent_capability.clone();
    let mut canonical_roots = Vec::with_capacity(capability.cwd_roots.len());
    for root in &capability.cwd_roots {
        canonical_roots.push(root.canonicalize().map_err(|error| {
            format!(
                "invalid inherited capability root {}: {error}",
                root.display()
            )
        })?);
    }
    if !canonical_roots.iter().any(|root| cwd.starts_with(root)) {
        return Err(format!(
            "child cwd {} is outside inherited capability roots",
            cwd.display()
        ));
    }
    capability.cwd_roots = vec![cwd.clone()];
    let inherited_write_root = match write_root {
        Some(root) => narrow_root_to_child(&cwd, root, "write")?,
        None => cwd.clone(),
    };
    if let nomi_process_runtime::SandboxPolicy::MacSeatbelt { write_roots } =
        &mut capability.sandbox
    {
        let mut narrowed = Vec::with_capacity(write_roots.len());
        for root in write_roots.iter() {
            narrowed.push(narrow_root_to_child(&cwd, root, "Seatbelt write")?);
        }
        *write_roots = narrowed;
    }
    let mut registry = ToolRegistry::new();
    for descriptor in CHILD_TOOL_CATALOG {
        if !scope.allows(descriptor.name) {
            continue;
        }
        let tool: Box<dyn nomi_tools::Tool> = match descriptor.kind {
            ChildToolKind::Read => Box::new(ReadTool::new(None, Some(cwd.clone()))),
            ChildToolKind::Write => Box::new(
                WriteTool::new(None)
                    .with_write_root(Some(inherited_write_root.clone()))
                    .with_cwd(Some(cwd.clone())),
            ),
            ChildToolKind::Edit => Box::new(
                EditTool::new(None)
                    .with_write_root(Some(inherited_write_root.clone()))
                    .with_cwd(Some(cwd.clone())),
            ),
            ChildToolKind::Bash => Box::new(BashTool::new(
                Arc::clone(&supervisor),
                cwd.clone(),
                capability.clone(),
            )),
            ChildToolKind::Grep => Box::new(GrepTool::new(cwd.clone())),
            ChildToolKind::Glob => Box::new(GlobTool::new(cwd.clone())),
        };
        registry.register(tool);
    }
    Ok(registry)
}

fn narrow_root_to_child(cwd: &Path, root: &Path, label: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("invalid inherited {label} root {}: {error}", root.display()))?;
    if cwd.starts_with(&root) {
        Ok(cwd.to_path_buf())
    } else if root.starts_with(cwd) {
        Ok(root)
    } else {
        Err(format!(
            "child cwd {} and inherited {label} root {} do not overlap",
            cwd.display(),
            root.display()
        ))
    }
}

fn translate_root_to_worktree(
    source: &Path,
    worktree: &Path,
    root: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("invalid inherited {label} root {}: {error}", root.display()))?;
    if source.starts_with(&root) {
        return Ok(worktree.to_path_buf());
    }
    let relative = root.strip_prefix(source).map_err(|_| {
        format!(
            "source workspace {} and inherited {label} root {} do not overlap",
            source.display(),
            root.display()
        )
    })?;
    let translated = worktree.join(relative);
    if translated.is_dir() {
        Ok(translated)
    } else {
        Err(format!(
            "translated {label} root does not exist in isolated worktree: {}",
            translated.display()
        ))
    }
}

#[cfg(test)]
mod phase7_tests {
    use super::{
        CHILD_TOOL_CATALOG, ChildToolKind, FanoutWorkspacePlan, LocalAgentInvocationRunner,
        ToolScope, WorkspaceEffect, build_tool_registry, map_agent_invocation_outcome,
    };
    use crate::engine::{AgentError, AgentResult};
    use async_trait::async_trait;
    use nomi_process_runtime::{CapabilityPolicy, ProcessSupervisor, SandboxPolicy, SupervisorConfig};
    use nomi_config::compat::ProviderCompat;
    use nomi_config::config::{Config, ProviderType};
    use nomi_providers::{LlmProvider, ProviderError};
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::{StopReason, TokenUsage};
    use nomi_types::agent::{AgentInvocationInput, AgentToolPolicy};
    use std::collections::{BTreeSet, VecDeque};
    use std::sync::{Arc, Mutex};

    fn test_registry(
        scope: ToolScope,
        cwd: &std::path::Path,
        capability: CapabilityPolicy,
    ) -> Result<nomi_tools::registry::ToolRegistry, String> {
        build_tool_registry(
            scope,
            cwd,
            &capability,
            Some(cwd),
            ProcessSupervisor::new(SupervisorConfig::default()),
        )
    }

    fn test_config() -> Config {
        Config {
            provider_label: "openai".into(),
            provider: ProviderType::OpenAI,
            api_key: "sk-test".into(),
            base_url: "http://localhost:0".into(),
            model: "gpt-test-model".into(),
            max_tokens: 1024,
            max_turns: Some(5),
            system_prompt: None,
            project_instructions: Default::default(),
            thinking: None,
            prompt_caching: false,
            compat: ProviderCompat::openai_defaults(),
            tools: Default::default(),
            session: Default::default(),
            compact: Default::default(),
            plan: Default::default(),
            file_cache: Default::default(),
            hooks: Default::default(),
            bedrock: None,
            vertex: None,
            mcp: Default::default(),
            logging: Default::default(),
        }
    }

    struct SequenceProvider {
        turns: Mutex<VecDeque<Vec<LlmEvent>>>,
    }

    impl SequenceProvider {
        fn new(texts: &[&str]) -> Self {
            let turns = texts
                .iter()
                .map(|text| {
                    vec![
                        LlmEvent::TextDelta((*text).to_owned()),
                        LlmEvent::Done {
                            stop_reason: StopReason::EndTurn,
                            usage: TokenUsage::default(),
                        },
                    ]
                })
                .collect();
            Self {
                turns: Mutex::new(turns),
            }
        }

        fn from_turns(turns: Vec<Vec<LlmEvent>>) -> Self {
            Self {
                turns: Mutex::new(turns.into()),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for SequenceProvider {
        async fn stream(
            &self,
            _request: &LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
            let events = self
                .turns
                .lock()
                .unwrap()
                .pop_front()
                .expect("one provider turn per invocation");
            let (sender, receiver) = tokio::sync::mpsc::channel(events.len().max(1));
            for event in events {
                sender.try_send(event).unwrap();
            }
            Ok(receiver)
        }
    }

    fn invocation(name: &str) -> AgentInvocationInput {
        AgentInvocationInput {
            name: name.to_owned(),
            prompt: format!("task for {name}"),
            max_turns: 5,
            max_tokens: 1024,
            system_prompt: None,
            model: None,
            effort: None,
            tool_policy: AgentToolPolicy::Full,
            exact_tools: Vec::new(),
        }
    }

    fn read_only_invocation(name: &str) -> AgentInvocationInput {
        AgentInvocationInput {
            tool_policy: AgentToolPolicy::ReadOnly,
            ..invocation(name)
        }
    }

    #[tokio::test]
    async fn local_runner_invokes_one_agent_primitive() {
        let cwd = tempfile::tempdir().unwrap();
        let runner = LocalAgentInvocationRunner::new(
            Arc::new(SequenceProvider::new(&["done"])),
            test_config(),
            cwd.path().to_path_buf(),
        );
        let output = runner.invoke_one(invocation("one")).await;
        assert_eq!(output.name, "one");
        assert_eq!(output.text, "done");
        assert!(!output.is_error);
    }

    #[tokio::test]
    async fn local_fanout_keeps_every_invocation_output() {
        let cwd = tempfile::tempdir().unwrap();
        let runner = LocalAgentInvocationRunner::new(
            Arc::new(SequenceProvider::new(&["A", "B", "C"])),
            test_config(),
            cwd.path().to_path_buf(),
        );
        let outputs = runner
            .execute_fanout(vec![
                read_only_invocation("one"),
                read_only_invocation("two"),
                read_only_invocation("three"),
            ])
            .await;
        assert_eq!(outputs.len(), 3);
        assert!(outputs.iter().all(|output| !output.is_error));
        assert_eq!(
            outputs
                .iter()
                .map(|output| output.text.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["A", "B", "C"])
        );
    }

    #[test]
    fn child_tool_catalog_is_the_single_workspace_effect_source() {
        let names = CHILD_TOOL_CATALOG
            .iter()
            .map(|descriptor| descriptor.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names,
            BTreeSet::from(["Bash", "Edit", "Glob", "Grep", "Read", "Write"]),
            "the scheduler and registry must use the same complete catalog"
        );
        for descriptor in CHILD_TOOL_CATALOG {
            let expected = match descriptor.kind {
                ChildToolKind::Read | ChildToolKind::Grep | ChildToolKind::Glob => {
                    WorkspaceEffect::ReadOnly
                }
                ChildToolKind::Write | ChildToolKind::Edit | ChildToolKind::Bash => {
                    WorkspaceEffect::MayMutate
                }
            };
            assert_eq!(descriptor.workspace_effect, expected, "{}", descriptor.name);
        }
    }

    #[test]
    fn fanout_workspace_plan_uses_effective_authority_and_isolates_only_writers() {
        let read_only = ToolScope::from_policy(AgentToolPolicy::ReadOnly);
        let write_only = ToolScope::Restricted(BTreeSet::from(["Write".to_owned()]));
        let read_shell = ToolScope::from_policy(AgentToolPolicy::ReadShell);

        assert!(!read_only.may_mutate_workspace());
        assert!(write_only.may_mutate_workspace());
        assert!(
            read_shell.may_mutate_workspace(),
            "Bash is mutation-capable even under the read_shell policy"
        );

        let one_writer =
            FanoutWorkspacePlan::for_scopes(&[write_only.clone(), read_only.clone()], true);
        assert!(!one_writer.isolates(&write_only));
        assert!(!one_writer.isolates(&read_only));

        let parallel_writers = FanoutWorkspacePlan::for_scopes(
            &[write_only.clone(), read_shell.clone(), read_only.clone()],
            true,
        );
        assert!(parallel_writers.isolates(&write_only));
        assert!(parallel_writers.isolates(&read_shell));
        assert!(!parallel_writers.isolates(&read_only));

        let non_git =
            FanoutWorkspacePlan::for_scopes(&[write_only.clone(), read_shell, read_only.clone()], false);
        assert!(non_git.warns(&write_only));
        assert!(!non_git.warns(&read_only));
    }

    #[test]
    fn parent_policy_and_exact_tools_can_remove_mutation_authority() {
        let parent_read_only = ToolScope::Restricted(BTreeSet::from(["Read".to_owned()]));
        let requested_full = ToolScope::from_policy(AgentToolPolicy::Full);
        let exact_read = ToolScope::from_exact_tools(&["Read".to_owned()]);
        let effective = parent_read_only
            .intersect(&requested_full)
            .intersect(&exact_read);

        assert_eq!(
            effective,
            ToolScope::Restricted(BTreeSet::from(["Read".to_owned()]))
        );
        assert!(!effective.may_mutate_workspace());
    }

    #[test]
    fn delegated_and_synthesis_agents_never_inherit_executable_parent_hooks() {
        let cwd = tempfile::tempdir().unwrap();
        let mut config = test_config();
        config.hooks.post_tool_use.push(nomi_config::hooks::HookDef {
            name: "would-mutate-after-read".to_owned(),
            tool_match: vec!["Read".to_owned()],
            file_match: Vec::new(),
            command: "write-outside-capability".to_owned(),
            timeout_ms: 1,
        });
        config.hooks.stop.push(nomi_config::hooks::HookDef {
            name: "would-mutate-on-stop".to_owned(),
            tool_match: Vec::new(),
            file_match: Vec::new(),
            command: "write-outside-capability".to_owned(),
            timeout_ms: 1,
        });
        let runner = LocalAgentInvocationRunner::new(
            Arc::new(SequenceProvider::new(&[])),
            config,
            cwd.path().to_path_buf(),
        );

        let child = runner.config_for_invocation(&read_only_invocation("synthesizer"));

        assert!(child.hooks.pre_tool_use.is_empty());
        assert!(child.hooks.post_tool_use.is_empty());
        assert!(child.hooks.stop.is_empty());
    }

    #[tokio::test]
    async fn read_only_agent_tool_execution_cannot_trigger_a_mutating_parent_hook() {
        let cwd = tempfile::tempdir().unwrap();
        let readable = cwd.path().join("readable.txt");
        let marker = cwd.path().join("hook-must-not-run.txt");
        std::fs::write(&readable, "safe\n").unwrap();
        let mut config = test_config();
        config.hooks.post_tool_use.push(nomi_config::hooks::HookDef {
            name: "mutating-read-hook".to_owned(),
            tool_match: vec!["Read".to_owned()],
            file_match: Vec::new(),
            command: format!("echo hook-ran > \"{}\"", marker.display()),
            timeout_ms: 5_000,
        });
        let provider = SequenceProvider::from_turns(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "read-one".to_owned(),
                    name: "Read".to_owned(),
                    input: serde_json::json!({"file_path": readable}),
                    extra: None,
                },
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    usage: TokenUsage::default(),
                },
            ],
            vec![
                LlmEvent::TextDelta("done".to_owned()),
                LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                },
            ],
        ]);
        let runner = LocalAgentInvocationRunner::new(
            Arc::new(provider),
            config,
            cwd.path().to_path_buf(),
        );

        let output = runner.invoke_one(read_only_invocation("reader")).await;

        assert!(!output.is_error, "{}", output.text);
        assert_eq!(output.text, "done");
        assert!(
            !marker.exists(),
            "read_only and synthesis Agents must not inherit raw-shell parent hooks"
        );
    }

    #[tokio::test]
    async fn non_git_parallel_writers_receive_a_visible_receipt_warning() {
        let cwd = tempfile::tempdir().unwrap();
        let runner = LocalAgentInvocationRunner::new(
            Arc::new(SequenceProvider::new(&["A", "B", "C"])),
            test_config(),
            cwd.path().to_path_buf(),
        );

        let outputs = runner
            .execute_fanout(vec![
                invocation("writer-one"),
                invocation("writer-two"),
                read_only_invocation("reader"),
            ])
            .await;

        assert_eq!(outputs.len(), 3);
        for output in &outputs[..2] {
            assert!(
                output
                    .text
                    .contains("shared the parent workspace because the workspace is not a Git repository"),
                "{}: {}",
                output.name,
                output.text
            );
        }
        assert_eq!(outputs[2].name, "reader");
        assert!(!outputs[2].text.contains("WARNING:"), "{}", outputs[2].text);
    }

    #[tokio::test]
    async fn map_agent_invocation_outcome_timeout_is_error() {
        // Produce a genuine Elapsed by timing out a never-completing future.
        let elapsed: Result<Result<AgentResult, AgentError>, _> =
            tokio::time::timeout(std::time::Duration::from_nanos(1), std::future::pending()).await;
        assert!(elapsed.is_err(), "precondition: should have elapsed");

        let r = map_agent_invocation_outcome("agent-x".to_string(), elapsed, 300);
        assert!(r.is_error, "a timed-out delegated Agent must be reported as an error");
        assert_eq!(r.name, "agent-x");
        assert!(r.text.contains("timed out"), "text: {}", r.text);
        assert_eq!(r.turns, 0);
    }

    #[test]
    fn map_agent_invocation_outcome_ok_maps_result() {
        let ok: Result<Result<AgentResult, AgentError>, tokio::time::error::Elapsed> =
            Ok(Ok(AgentResult {
                text: "done".to_string(),
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
                turns: 3,
            }));
        let r = map_agent_invocation_outcome("a".to_string(), ok, 300);
        assert!(!r.is_error);
        assert_eq!(r.text, "done");
        assert_eq!(r.turns, 3);
    }

    #[test]
    fn map_agent_invocation_outcome_engine_error_is_error() {
        let err: Result<Result<AgentResult, AgentError>, tokio::time::error::Elapsed> =
            Ok(Err(AgentError::UserAborted));
        let r = map_agent_invocation_outcome("a".to_string(), err, 300);
        assert!(r.is_error);
        assert!(r.text.contains("Delegated Agent error"), "text: {}", r.text);
    }

    #[test]
    fn policy_and_exact_tool_set_are_both_monotonic() {
        let read_only = ToolScope::from_policy(AgentToolPolicy::ReadOnly);
        let bash_only = ToolScope::from_exact_tools(&["Bash".to_owned()]);
        assert_eq!(
            read_only.intersect(&bash_only),
            ToolScope::Restricted(BTreeSet::new()),
            "a disjoint exact skill tool set must deny all, never widen read_only"
        );
    }

    #[test]
    fn tc_7_40_build_tool_registry_empty_allowed_registers_all() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let registry = test_registry(
            ToolScope::Unrestricted,
            &cwd,
            CapabilityPolicy::local_owner(cwd.clone()),
        )
        .unwrap();
        for name in &["Read", "Write", "Edit", "Bash", "Grep", "Glob"] {
            assert!(
                registry.get(name).is_some(),
                "tool '{name}' should be registered"
            );
        }
        assert!(
            registry.get(&["shared", "tasks"].join("_")).is_none(),
            "embedded coordination must not add another model tool"
        );
    }

    #[test]
    fn tc_7_43_build_tool_registry_filters_to_allowed() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let registry = test_registry(
            ToolScope::Restricted(BTreeSet::from(["Bash".to_string(), "Read".to_string()])),
            &cwd,
            CapabilityPolicy::local_owner(cwd.clone()),
        )
        .unwrap();
        assert!(registry.get("Bash").is_some());
        assert!(registry.get("Read").is_some());
        assert!(registry.get("Write").is_none());
    }

    #[test]
    fn tool_scope_intersection_is_monotonic_and_disjoint_means_deny_all() {
        let read = ToolScope::Restricted(BTreeSet::from(["Read".to_string()]));
        let bash = ToolScope::Restricted(BTreeSet::from(["Bash".to_string()]));
        let unrestricted = ToolScope::Unrestricted;

        assert_eq!(unrestricted.intersect(&read), read);
        assert_eq!(
            read.intersect(&bash),
            ToolScope::Restricted(BTreeSet::new())
        );
    }

    #[test]
    fn disjoint_parent_and_exact_scopes_register_no_builtin_tools() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let parent = ToolScope::Restricted(BTreeSet::from(["Read".to_string()]));
        let exact = ToolScope::Restricted(BTreeSet::from(["Bash".to_string()]));
        let registry = test_registry(
            parent.intersect(&exact),
            &cwd,
            CapabilityPolicy::local_owner(cwd.clone()),
        )
        .unwrap();

        assert!(registry.tool_names().is_empty());
    }

    #[tokio::test]
    async fn inherited_deny_execution_is_not_downgraded() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let capability = CapabilityPolicy {
            cwd_roots: vec![cwd.clone()],
            sandbox: SandboxPolicy::DenySpawn,
        };
        let registry = test_registry(
            ToolScope::Restricted(BTreeSet::from(["Bash".to_string()])),
            &cwd,
            capability,
        )
        .unwrap();
        let bash = registry.get("Bash").expect("Bash should be registered");
        let result = bash
            .execute(serde_json::json!({"command": "echo must_not_run"}))
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("capability_denied"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn child_registry_inherits_seatbelt_and_cannot_write_outside_child_cwd() {
        // `tempfile::tempdir()` lives inside Darwin's trusted user temporary
        // directory, which the Seatbelt profile intentionally keeps writable.
        // Place this fixture beside the checkout so the sibling path actually
        // exercises the child-root restriction.
        let parent = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let child = parent.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let parent_root = parent.path().canonicalize().unwrap();
        let child_root = child.canonicalize().unwrap();
        let outside = parent_root.join("outside-child.marker");
        let registry = build_tool_registry(
            ToolScope::Restricted(BTreeSet::from(["Bash".to_string()])),
            &child_root,
            &CapabilityPolicy {
                cwd_roots: vec![parent_root.clone()],
                sandbox: SandboxPolicy::MacSeatbelt {
                    write_roots: vec![parent_root],
                },
            },
            None,
            ProcessSupervisor::new(SupervisorConfig::default()),
        )
        .expect("child registry");
        let bash = registry.get("Bash").expect("Bash");

        let inside = child_root.join("inside-child.marker");
        let allowed = bash
            .execute(serde_json::json!({
                "command": format!("printf allowed > '{}'", inside.display())
            }))
            .await;
        assert!(!allowed.is_error, "{}", allowed.content);
        assert!(inside.exists());

        let denied = bash
            .execute(serde_json::json!({
                "command": format!("printf denied > '{}'", outside.display())
            }))
            .await;
        assert!(denied.is_error, "{}", denied.content);
        assert!(!outside.exists());
    }

    #[test]
    fn child_cwd_outside_inherited_roots_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let capability = CapabilityPolicy::local_owner(
            parent.path().canonicalize().unwrap(),
        );
        let result = test_registry(
            ToolScope::Unrestricted,
            outside.path(),
            capability,
        );

        assert!(result.is_err());
    }

    #[test]
    fn isolated_worktree_translates_authority_to_the_exact_verified_worktree() {
        struct NeverCalledProvider;

        #[async_trait]
        impl LlmProvider for NeverCalledProvider {
            async fn stream(
                &self,
                _request: &LlmRequest,
            ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
                panic!("isolated capability validation must happen before provider use")
            }
        }

        fn git(args: &[&str], cwd: &std::path::Path) {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let parent = tempfile::tempdir().unwrap();
        git(&["init", "-q"], parent.path());
        git(&["config", "user.email", "t@t"], parent.path());
        git(&["config", "user.name", "t"], parent.path());
        std::fs::write(parent.path().join("tracked.txt"), "tracked\n").unwrap();
        let nested = parent.path().join("packages").join("a");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("nested.txt"), "nested\n").unwrap();
        git(&["add", "-A"], parent.path());
        git(&["commit", "-q", "-m", "init"], parent.path());
        let nested_root = nested.canonicalize().unwrap();
        let backend = LocalAgentInvocationRunner::new(
            Arc::new(NeverCalledProvider),
            test_config(),
            nested_root.clone(),
        )
        .with_process_capability(
            CapabilityPolicy::local_owner(nested_root.clone()),
            Some(nested_root),
            Vec::new(),
        );
        let worktree =
            nomi_tools::worktree::Worktree::create(&nested).expect("create worktree");
        let expected = worktree.verified_cwd(&nested).expect("verified worktree cwd");

        let isolated = backend
            .clone_for_isolated_worktree(&worktree)
            .expect("trusted Worktree handle may delegate exact authority");
        assert_eq!(isolated.cwd, expected);
        assert_eq!(isolated.process_capability.cwd_roots, vec![expected.clone()]);
        assert_eq!(isolated.write_root, Some(expected));
        assert_ne!(
            isolated.cwd,
            worktree.verified_path().expect("worktree root"),
            "nested parent cwd must not delegate the whole repository worktree"
        );
    }

    #[test]
    fn child_cwd_with_disjoint_inherited_write_root_is_rejected() {
        let capability_root = tempfile::tempdir().unwrap();
        let child = capability_root.path().join("child");
        let sibling_write_root = capability_root.path().join("write-only-sibling");
        std::fs::create_dir(&child).unwrap();
        std::fs::create_dir(&sibling_write_root).unwrap();
        let capability =
            CapabilityPolicy::local_owner(capability_root.path().canonicalize().unwrap());
        let result = build_tool_registry(
            ToolScope::Unrestricted,
            &child,
            &capability,
            Some(&sibling_write_root),
            ProcessSupervisor::new(SupervisorConfig::default()),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ordinary_child_without_parent_write_root_is_still_scoped_to_its_cwd() {
        let parent = tempfile::tempdir().unwrap();
        let child = parent.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let capability =
            CapabilityPolicy::local_owner(parent.path().canonicalize().unwrap());
        let registry = build_tool_registry(
            ToolScope::Restricted(BTreeSet::from(["Write".to_string()])),
            &child,
            &capability,
            None,
            ProcessSupervisor::new(SupervisorConfig::default()),
        )
        .expect("child registry");
        let outside = parent.path().join("outside.txt");

        let result = registry
            .get("Write")
            .expect("Write")
            .execute(serde_json::json!({
                "file_path": outside.to_string_lossy(),
                "content": "must not escape"
            }))
            .await;

        assert!(result.is_error, "{}", result.content);
        assert!(!outside.exists());
    }

    #[test]
    fn tc_7_delegated_agent_config_original_fields_intact() {
        let config = AgentInvocationInput {
            name: "test-agent".to_string(),
            prompt: "do the task".to_string(),
            max_turns: 5,
            max_tokens: 1024,
            system_prompt: Some("you are helpful".to_string()),
            model: None,
            effort: None,
            tool_policy: AgentToolPolicy::Full,
            exact_tools: Vec::new(),
        };
        assert_eq!(config.name, "test-agent");
        assert_eq!(config.max_turns, 5);
    }

    #[tokio::test]
    async fn execute_bounded_caps_concurrency_and_returns_all() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        use std::time::Duration;
        use tokio::sync::Semaphore;

        let cap = 3;
        let n = 12;
        let peak = Arc::new(AtomicUsize::new(0));
        let inflight = Arc::new(AtomicUsize::new(0));
        let sem = Arc::new(Semaphore::new(cap));

        let tasks: Vec<_> = (0..n)
            .map(|i| {
                let peak = peak.clone();
                let inflight = inflight.clone();
                async move {
                    let now = inflight.fetch_add(1, SeqCst) + 1;
                    peak.fetch_max(now, SeqCst);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    inflight.fetch_sub(1, SeqCst);
                    i
                }
            })
            .collect();

        let results = super::execute_bounded(sem, tasks).await;
        assert_eq!(results.len(), n, "every task must produce a result");
        let mut values: Vec<usize> = results.into_iter().map(|r| r.expect("no join error")).collect();
        values.sort_unstable();
        assert_eq!(values, (0..n).collect::<Vec<_>>(), "results preserve all tasks in order");
        assert!(
            peak.load(SeqCst) <= cap,
            "peak concurrency {} must not exceed the cap {cap}",
            peak.load(SeqCst)
        );
        assert!(peak.load(SeqCst) >= 2, "tasks should have run concurrently up to the cap");
    }

    #[tokio::test]
    async fn execute_bounded_aborts_in_flight_tasks_when_dropped() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        use std::time::Duration;
        use tokio::sync::Semaphore;

        let completed = Arc::new(AtomicUsize::new(0));
        let sem = Arc::new(Semaphore::new(8));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let completed = completed.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    completed.fetch_add(1, SeqCst);
                }
            })
            .collect();

        // Start the fan-out, then drop its future after 50ms (simulating the
        // parent turn being cancelled mid-flight).
        {
            let fut = super::execute_bounded(sem, tasks);
            tokio::select! {
                _ = fut => {}
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        } // `fut` dropped here → JoinSet drops → tasks aborted

        // Give any *non-aborted* task time to reach its increment.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(
            completed.load(SeqCst),
            0,
            "dropping execute_bounded must abort in-flight delegated Agent tasks (no detached compute)"
        );
    }

    #[test]
    fn token_budget_blocks_once_exhausted() {
        use super::TokenBudget;
        let b = TokenBudget::new(100);
        assert!(b.can_begin());
        b.record(60);
        assert_eq!(b.remaining(), 40);
        assert!(b.can_begin(), "40 left → another delegated Agent may start");
        b.record(60); // saturating past zero
        assert_eq!(b.remaining(), 0);
        assert!(!b.can_begin(), "exhausted budget blocks further delegated Agents");
    }
}
