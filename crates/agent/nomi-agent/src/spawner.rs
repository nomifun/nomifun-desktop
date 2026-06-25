use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use nomi_config::config::Config;
use nomi_providers::LlmProvider;
use nomi_tools::bash::BashTool;
use nomi_tools::edit::EditTool;
use nomi_tools::glob::GlobTool;
use nomi_tools::grep::GrepTool;
use nomi_tools::read::ReadTool;
use nomi_tools::registry::ToolRegistry;
use nomi_tools::write::WriteTool;
use nomi_types::message::TokenUsage;

use crate::engine::AgentEngine;
use crate::output::OutputSink;
use crate::output::null_sink::NullSink;

// Re-export from nomi-types — single source of truth
pub use nomi_types::spawner::{ForkOverrides, Spawner, SubAgentConfig, SubAgentResult};

/// Spawns independent child agents that share the parent's LLM provider.
///
/// Sub-agents use a [`NullSink`] so their streaming output is silently
/// discarded.  Results are collected via `engine.run()` and returned to the
/// parent which emits them as a single `tool_result` event — matching the
/// Claude Code pattern where only the parent writes to stdout.
pub struct AgentSpawner {
    provider: Arc<dyn LlmProvider>,
    base_config: Config,
    cwd: PathBuf,
    /// Shared across `clone_for_spawn` so the concurrency cap is global to this
    /// spawner (all fan-outs draw from the same permit pool).
    concurrency: Arc<Semaphore>,
    /// Optional shared token ceiling (opt-in; None = uncapped).
    token_budget: Option<Arc<TokenBudget>>,
}

impl AgentSpawner {
    pub fn new(provider: Arc<dyn LlmProvider>, config: Config, cwd: PathBuf) -> Self {
        Self {
            provider,
            base_config: config,
            cwd,
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_SUBAGENTS)),
            token_budget: None,
        }
    }

    /// Enable a shared cumulative token ceiling for all sub-agents (§3.4).
    pub fn with_token_budget(mut self, budget: Option<Arc<TokenBudget>>) -> Self {
        self.token_budget = budget;
        self
    }

    /// Spawn a single sub-agent and wait for result.
    pub async fn spawn_one(&self, sub_config: SubAgentConfig) -> SubAgentResult {
        self.spawn_one_with_board(sub_config, None).await
    }

    /// Spawn a single sub-agent, optionally giving it the shared task board so it
    /// can coordinate with siblings (TIER 2, §3.4).
    async fn spawn_one_with_board(
        &self,
        sub_config: SubAgentConfig,
        board: Option<Arc<crate::taskboard::TaskBoard>>,
    ) -> SubAgentResult {
        // Shared token ceiling (opt-in): refuse to start a new sub-agent once
        // the budget is spent, rather than launching work that won't be paid for.
        if let Some(budget) = &self.token_budget
            && !budget.can_begin()
        {
            return SubAgentResult {
                name: sub_config.name,
                text: "Sub-agent skipped: shared token budget exhausted.".to_string(),
                usage: TokenUsage::default(),
                turns: 0,
                is_error: true,
            };
        }

        let mut config = self.base_config.clone();
        config.max_turns = Some(sub_config.max_turns);
        config.max_tokens = sub_config.max_tokens;
        if let Some(sp) = sub_config.system_prompt.clone() {
            config.system_prompt = Some(sp);
        }
        config.session.enabled = false;
        config.tools.auto_approve = true;

        tracing::info!(target: "nomi_agent", cwd = %self.cwd.display(), "sub-agent spawned with workspace cwd");

        let agent_name = sub_config.name.clone();
        let board_arg = board.map(|b| (b, agent_name.as_str()));
        let tools = build_tool_registry(&sub_config.allowed_tools, &self.cwd, board_arg);
        let output: Arc<dyn OutputSink> = Arc::new(NullSink);
        let engine = AgentEngine::new_with_provider(
            self.provider.clone(),
            config,
            tools,
            output,
            self.cwd.clone(),
        );

        let result = run_subagent(engine, sub_config.name, &sub_config.prompt).await;
        // Charge the shared budget for what this sub-agent actually consumed.
        if let Some(budget) = &self.token_budget {
            budget.record(result.usage.input_tokens + result.usage.output_tokens);
        }
        result
    }

    /// Spawn multiple sub-agents in parallel, bounded by the shared concurrency
    /// permit pool so a large fan-out cannot start every engine at once.
    pub async fn spawn_parallel(&self, sub_configs: Vec<SubAgentConfig>) -> Vec<SubAgentResult> {
        let tasks: Vec<_> = sub_configs
            .into_iter()
            .map(|config| {
                let spawner = self.clone_for_spawn();
                async move { spawner.spawn_one(config).await }
            })
            .collect();

        run_bounded(self.concurrency.clone(), tasks)
            .await
            .into_iter()
            .map(|joined| {
                joined.unwrap_or_else(|e| SubAgentResult {
                    name: "unknown".to_string(),
                    text: format!("Task join error: {}", e),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                })
            })
            .collect()
    }

    /// Like [`Self::spawn_parallel`] but every sub-agent shares one in-memory
    /// task board (TIER 2 coordination, §3.4): siblings can claim work to avoid
    /// duplication and report progress to one another via the `shared_tasks` tool.
    pub async fn spawn_parallel_coordinated(
        &self,
        sub_configs: Vec<SubAgentConfig>,
    ) -> Vec<SubAgentResult> {
        let board = Arc::new(crate::taskboard::TaskBoard::new());
        let tasks: Vec<_> = sub_configs
            .into_iter()
            .map(|config| {
                let spawner = self.clone_for_spawn();
                let board = board.clone();
                async move { spawner.spawn_one_with_board(config, Some(board)).await }
            })
            .collect();

        run_bounded(self.concurrency.clone(), tasks)
            .await
            .into_iter()
            .map(|joined| {
                joined.unwrap_or_else(|e| SubAgentResult {
                    name: "unknown".to_string(),
                    text: format!("Task join error: {}", e),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                })
            })
            .collect()
    }

    fn clone_for_spawn(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            base_config: self.base_config.clone(),
            cwd: self.cwd.clone(),
            concurrency: self.concurrency.clone(),
            token_budget: self.token_budget.clone(),
        }
    }

    /// Clone the spawner but rooted at a different cwd (used to run a sub-agent
    /// inside an isolated worktree).
    fn clone_with_cwd(&self, cwd: PathBuf) -> Self {
        let mut c = self.clone_for_spawn();
        c.cwd = cwd;
        c
    }

    /// Like [`Self::spawn_parallel`] but each sub-agent runs in its own detached
    /// git worktree, so concurrent edits don't clobber the shared tree or each
    /// other (§3.4 worktree isolation). Each result has the sub-agent's changes
    /// appended as a unified diff for the parent to review/apply; the worktree is
    /// removed afterwards. Falls back to plain `spawn_parallel` when the
    /// workspace is not a git repo.
    pub async fn spawn_parallel_isolated(&self, sub_configs: Vec<SubAgentConfig>) -> Vec<SubAgentResult> {
        if !nomi_tools::worktree::is_git_repo(&self.cwd) {
            tracing::warn!(target: "nomi_agent", "isolate requested but workspace is not a git repo — running without worktree isolation");
            return self.spawn_parallel(sub_configs).await;
        }
        let tasks: Vec<_> = sub_configs
            .into_iter()
            .map(|config| {
                let base = self.clone_for_spawn();
                let repo = self.cwd.clone();
                async move {
                    match nomi_tools::worktree::Worktree::create(&repo) {
                        Ok(wt) => {
                            let spawner = base.clone_with_cwd(wt.path().to_path_buf());
                            let mut result = spawner.spawn_one(config).await;
                            match wt.capture_diff() {
                                Ok(diff) if !diff.trim().is_empty() => {
                                    result.text = format!(
                                        "{}\n\n--- Changes (made in an isolated worktree; apply with ApplyPatch if you want them) ---\n{}",
                                        result.text, diff
                                    );
                                }
                                _ => {}
                            }
                            result // `wt` dropped here → worktree removed
                        }
                        Err(e) => SubAgentResult {
                            name: config.name,
                            text: format!("Worktree isolation failed: {e}"),
                            usage: TokenUsage::default(),
                            turns: 0,
                            is_error: true,
                        },
                    }
                }
            })
            .collect();

        run_bounded(self.concurrency.clone(), tasks)
            .await
            .into_iter()
            .map(|joined| {
                joined.unwrap_or_else(|e| SubAgentResult {
                    name: "unknown".to_string(),
                    text: format!("Task join error: {}", e),
                    usage: TokenUsage::default(),
                    turns: 0,
                    is_error: true,
                })
            })
            .collect()
    }
}

#[async_trait]
impl Spawner for AgentSpawner {
    async fn spawn_fork(
        &self,
        sub_config: SubAgentConfig,
        overrides: ForkOverrides,
    ) -> SubAgentResult {
        let mut config = self.base_config.clone();
        config.max_turns = Some(sub_config.max_turns);
        config.max_tokens = sub_config.max_tokens;
        if let Some(sp) = sub_config.system_prompt.clone() {
            config.system_prompt = Some(sp);
        }
        config.session.enabled = false;
        config.tools.auto_approve = true;
        if let Some(model) = overrides.model.clone() {
            config.model = model;
        }

        let tools = build_tool_registry(&overrides.allowed_tools, &self.cwd, None);
        let output: Arc<dyn OutputSink> = Arc::new(NullSink);
        let mut engine = AgentEngine::new_with_provider(
            self.provider.clone(),
            config,
            tools,
            output,
            self.cwd.clone(),
        );
        engine.set_initial_reasoning_effort(overrides.effort.clone());

        run_subagent(engine, sub_config.name, &sub_config.prompt).await
    }
}

/// Wall-clock cap for a single sub-agent run. Without this, a sub-agent stuck
/// in a tool-call loop (up to its max_turns) or on a hung provider stream blocks
/// the parent indefinitely — a production hang risk. (Phase 2 multi-agent)
const SUBAGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Global cap on concurrently-running sub-agents across all fan-outs from one
/// spawner. Without it a single `Spawn` of N tasks (or several concurrent
/// `Spawn` tool calls sharing the spawner) starts N engines/provider streams at
/// once — a resource and provider-rate-limit storm. (Phase 2 multi-agent §3.4)
const MAX_CONCURRENT_SUBAGENTS: usize = 8;

/// Optional cumulative token ceiling shared across every sub-agent of a spawner
/// (§3.4 "共享 token 预算"). Opt-in: absent → uncapped (default, zero change).
/// A *soft* ceiling — concurrent sub-agents each pass `can_begin` before any of
/// them `record`s, so one fan-out may slightly overshoot; its job is to bound
/// cumulative spend across successive `Spawn` calls / nested fan-outs, not to
/// be an exact per-call limit.
pub struct TokenBudget {
    remaining: std::sync::atomic::AtomicU64,
}

impl TokenBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            remaining: std::sync::atomic::AtomicU64::new(limit),
        }
    }

    /// Whether there is budget left to start another sub-agent.
    pub fn can_begin(&self) -> bool {
        self.remaining.load(std::sync::atomic::Ordering::Relaxed) > 0
    }

    /// Deduct `tokens` from the remaining budget (saturating at 0).
    pub fn record(&self, tokens: u64) {
        let _ = self.remaining.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |cur| Some(cur.saturating_sub(tokens)),
        );
    }

    pub fn remaining(&self) -> u64 {
        self.remaining.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Run `tasks` concurrently with at most `semaphore`'s permits in flight at
/// once. Each task holds a permit for its whole run; excess tasks queue until a
/// slot frees. Returns each task's output (or its `JoinError` if the task
/// panicked) — successful results in input order, the rare panics appended.
///
/// Uses a `JoinSet`, so if THIS future is dropped (e.g. the parent turn is
/// cancelled while a fan-out is in flight) every still-running sub-agent task is
/// aborted instead of detaching and burning compute/tokens unattended.
async fn run_bounded<F, T>(
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
            let _permit = sem.acquire_owned().await.expect("subagent semaphore never closed");
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
    for slot in slots {
        if let Some(v) = slot {
            results.push(Ok(v));
        }
    }
    results.extend(panics.into_iter().map(Err));
    results
}

/// Map a (timeout-wrapped) engine run outcome to a SubAgentResult. Extracted so
/// the timeout/error/success mapping is unit-testable without a live engine.
fn map_subagent_outcome(
    name: String,
    outcome: Result<
        Result<crate::engine::AgentResult, crate::engine::AgentError>,
        tokio::time::error::Elapsed,
    >,
    timeout_secs: u64,
) -> SubAgentResult {
    match outcome {
        Ok(Ok(result)) => SubAgentResult {
            name,
            text: result.text,
            usage: result.usage,
            turns: result.turns,
            is_error: false,
        },
        Ok(Err(e)) => SubAgentResult {
            name,
            text: format!("Sub-agent error: {}", e),
            usage: TokenUsage::default(),
            turns: 0,
            is_error: true,
        },
        Err(_elapsed) => SubAgentResult {
            name,
            text: format!("Sub-agent timed out after {timeout_secs}s and was aborted"),
            usage: TokenUsage::default(),
            turns: 0,
            is_error: true,
        },
    }
}

/// Run an engine to completion under a wall-clock timeout, mapping the outcome.
async fn run_subagent(mut engine: AgentEngine, name: String, prompt: &str) -> SubAgentResult {
    let outcome = tokio::time::timeout(SUBAGENT_TIMEOUT, engine.run(prompt, "")).await;
    map_subagent_outcome(name, outcome, SUBAGENT_TIMEOUT.as_secs())
}

fn build_tool_registry(
    allowed: &[String],
    cwd: &Path,
    board: Option<(Arc<crate::taskboard::TaskBoard>, &str)>,
) -> ToolRegistry {
    let all_tools: Vec<(&str, Box<dyn nomi_tools::Tool>)> = vec![
        ("Read", Box::new(ReadTool::new(None, Some(cwd.to_path_buf())))),
        ("Write", Box::new(WriteTool::new(None))),
        ("Edit", Box::new(EditTool::new(None))),
        ("Bash", Box::new(BashTool::new(cwd.to_path_buf()))),
        ("Grep", Box::new(GrepTool::new(cwd.to_path_buf()))),
        ("Glob", Box::new(GlobTool::new(cwd.to_path_buf()))),
    ];

    let mut registry = ToolRegistry::new();
    for (name, tool) in all_tools {
        if allowed.is_empty() || allowed.iter().any(|a| a.as_str() == name) {
            registry.register(tool);
        }
    }
    // Coordinated fan-out: give this sub-agent the shared task board so siblings
    // can claim work and report progress to one another. Always registered when
    // a board is present (not subject to the role tool-allowlist).
    if let Some((board, agent_name)) = board {
        registry.register(Box::new(crate::taskboard::TaskBoardTool::new(board, agent_name)));
    }
    registry
}

#[cfg(test)]
mod phase7_tests {
    use super::{ForkOverrides, SubAgentConfig, build_tool_registry, map_subagent_outcome};
    use crate::engine::{AgentError, AgentResult};
    use nomi_types::message::{StopReason, TokenUsage};

    #[tokio::test]
    async fn map_subagent_outcome_timeout_is_error() {
        // Produce a genuine Elapsed by timing out a never-completing future.
        let elapsed: Result<Result<AgentResult, AgentError>, _> =
            tokio::time::timeout(std::time::Duration::from_nanos(1), std::future::pending()).await;
        assert!(elapsed.is_err(), "precondition: should have elapsed");

        let r = map_subagent_outcome("agent-x".to_string(), elapsed, 300);
        assert!(r.is_error, "a timed-out sub-agent must be reported as an error");
        assert_eq!(r.name, "agent-x");
        assert!(r.text.contains("timed out"), "text: {}", r.text);
        assert_eq!(r.turns, 0);
    }

    #[test]
    fn map_subagent_outcome_ok_maps_result() {
        let ok: Result<Result<AgentResult, AgentError>, tokio::time::error::Elapsed> =
            Ok(Ok(AgentResult {
                text: "done".to_string(),
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
                turns: 3,
            }));
        let r = map_subagent_outcome("a".to_string(), ok, 300);
        assert!(!r.is_error);
        assert_eq!(r.text, "done");
        assert_eq!(r.turns, 3);
    }

    #[test]
    fn map_subagent_outcome_engine_error_is_error() {
        let err: Result<Result<AgentResult, AgentError>, tokio::time::error::Elapsed> =
            Ok(Err(AgentError::UserAborted));
        let r = map_subagent_outcome("a".to_string(), err, 300);
        assert!(r.is_error);
        assert!(r.text.contains("Sub-agent error"), "text: {}", r.text);
    }

    #[test]
    fn tc_7_1_fork_overrides_default_values() {
        let o = ForkOverrides::default();
        assert!(o.model.is_none());
        assert!(o.effort.is_none());
        assert!(o.allowed_tools.is_empty());
    }

    #[test]
    fn tc_7_40_build_tool_registry_empty_allowed_registers_all() {
        let registry = build_tool_registry(&[], &std::env::temp_dir(), None);
        for name in &["Read", "Write", "Edit", "Bash", "Grep", "Glob"] {
            assert!(
                registry.get(name).is_some(),
                "tool '{name}' should be registered"
            );
        }
    }

    #[test]
    fn tc_7_43_build_tool_registry_filters_to_allowed() {
        let allowed = vec!["Bash".to_string(), "Read".to_string()];
        let registry = build_tool_registry(&allowed, &std::env::temp_dir(), None);
        assert!(registry.get("Bash").is_some());
        assert!(registry.get("Read").is_some());
        assert!(registry.get("Write").is_none());
    }

    #[test]
    fn tc_7_sub_agent_config_original_fields_intact() {
        let config = SubAgentConfig {
            name: "test-agent".to_string(),
            prompt: "do the task".to_string(),
            max_turns: 5,
            max_tokens: 1024,
            system_prompt: Some("you are helpful".to_string()),
            allowed_tools: Vec::new(),
        };
        assert_eq!(config.name, "test-agent");
        assert_eq!(config.max_turns, 5);
    }

    #[tokio::test]
    async fn run_bounded_caps_concurrency_and_returns_all() {
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

        let results = super::run_bounded(sem, tasks).await;
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
    async fn run_bounded_aborts_in_flight_tasks_when_dropped() {
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
            let fut = super::run_bounded(sem, tasks);
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
            "dropping run_bounded must abort in-flight sub-agent tasks (no detached compute)"
        );
    }

    #[test]
    fn token_budget_blocks_once_exhausted() {
        use super::TokenBudget;
        let b = TokenBudget::new(100);
        assert!(b.can_begin());
        b.record(60);
        assert_eq!(b.remaining(), 40);
        assert!(b.can_begin(), "40 left → another sub-agent may start");
        b.record(60); // saturating past zero
        assert_eq!(b.remaining(), 0);
        assert!(!b.can_begin(), "exhausted budget blocks further sub-agents");
    }
}
