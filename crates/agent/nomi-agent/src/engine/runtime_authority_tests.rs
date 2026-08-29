// ---------------------------------------------------------------------------
// Accepted-turn runtime authority — snapshot / restore regressions
//
// `AcceptedTurnRoot` restores the durable half of a rejected turn. These tests
// hooks, a different model / effort, plan mode, or goal progress must not keep
// any of it once the turn is retracted, and a clean success must keep all of it.
// ---------------------------------------------------------------------------

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use nomi_config::config::{CliArgs, Config};
use nomi_config::hooks::{HookDef, HookEngine, HooksConfig};
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, StopReason};
use nomi_types::skill_types::{ContextModifier, EffortLevel, PlanModeTransition};

use super::{AgentEngine, CompletionAdjudication, CompletionEvidenceContext};
use crate::goal::state::GoalStatus;
use crate::output::null_sink::NullSink;
use crate::session::SessionManager;

const ROOT_MODEL: &str = "root-model";
const SKILL_MODEL: &str = "skill-model";

fn requirement(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_owned(),
    }]
}

/// Pass A answers a harmless question; every later pass claims a file was
/// created. The claim has no machine evidence, so the terminal pass is the A2
/// verdict that retracts the whole accepted turn.
struct RaceTailFalseCompletionProvider {
    calls: AtomicUsize,
}

impl RaceTailFalseCompletionProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for RaceTailFalseCompletionProvider {
    async fn stream(
        &self,
        _request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let text = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            "The current status is available."
        } else {
            "Created miniapp.html."
        };
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(LlmEvent::TextDelta(text.to_owned())).unwrap();
        tx.try_send(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

/// Pass A succeeds; the next pass fails at the transport, which is the shape a
/// provider error and a dropped/cancelled turn both present to the engine.
struct RaceTailProviderErrorProvider {
    failed: AtomicBool,
}

#[async_trait::async_trait]
impl LlmProvider for RaceTailProviderErrorProvider {
    async fn stream(
        &self,
        _request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        if self.failed.swap(true, Ordering::SeqCst) {
            return Err(ProviderError::Api {
                status: 500,
                message: "upstream refused the request".to_owned(),
            });
        }
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(LlmEvent::TextDelta(
            "The current status is available.".to_owned(),
        ))
        .unwrap();
        tx.try_send(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

fn authority_test_config(workspace: &Path, session_directory: &Path) -> Config {
    let mut config = Config::resolve(&CliArgs {
        provider: Some("openai".to_owned()),
        api_key: Some("sk-test".to_owned()),
        base_url: None,
        model: Some(ROOT_MODEL.to_owned()),
        max_tokens: None,
        max_turns: Some(1),
        system_prompt: None,
        profile: None,
        project_dir: Some(workspace.to_path_buf()),
    })
    .unwrap();
    config.session.enabled = true;
    config.session.directory = session_directory.to_string_lossy().into_owned();
    config.compact.enabled = false;
    config.hooks = HooksConfig {
        pre_tool_use: vec![hook("baseline-guard")],
        ..Default::default()
    };
    config
}

fn hook(name: &str) -> HookDef {
    HookDef {
        name: name.to_owned(),
        tool_match: Vec::new(),
        file_match: Vec::new(),
        command: "echo ok".to_owned(),
        timeout_ms: 30_000,
    }
}

fn authority_engine(
    workspace: &Path,
    session_directory: &Path,
    session_id: &str,
    provider: Arc<dyn LlmProvider>,
) -> AgentEngine {
    let config = authority_test_config(workspace, session_directory);
    let mut engine = new_authority_engine(workspace, config, provider);
    engine
        .init_session("openai", &workspace.to_string_lossy(), Some(session_id))
        .unwrap();
    engine
}

/// Rebuild the same engine from config plus the persisted session, which is what
/// a runtime refresh does. Skill grants were never durable, so this is the
/// baseline a rejected turn's live runtime has to match.
fn reloaded_authority_engine(
    workspace: &Path,
    session_directory: &Path,
    session_id: &str,
    provider: Arc<dyn LlmProvider>,
) -> AgentEngine {
    let config = authority_test_config(workspace, session_directory);
    let session = SessionManager::new(session_directory.to_path_buf(), 20)
        .load(session_id)
        .unwrap();
    let hooks = HookEngine::new(config.hooks.clone(), workspace.to_path_buf());
    let mut engine = AgentEngine::resume_with_provider(
        provider,
        config,
        ToolRegistry::new(),
        Arc::new(NullSink),
        session,
        workspace.to_path_buf(),
    );
    engine.hooks = Some(hooks);
    engine.plan_active_flag = Some(Arc::new(AtomicBool::new(false)));
    // `max_auto_continuations: 0` keeps the engine from injecting goal
    // continuations, so turn termination stays the thing under test.
    engine.set_goal("ship the feature".to_owned(), 0);
    engine
}

fn new_authority_engine(
    workspace: &Path,
    config: Config,
    provider: Arc<dyn LlmProvider>,
) -> AgentEngine {
    let hooks = HookEngine::new(config.hooks.clone(), workspace.to_path_buf());
    let mut engine = AgentEngine::new_with_provider(
        provider,
        config,
        ToolRegistry::new(),
        Arc::new(NullSink),
        workspace.to_path_buf(),
    );
    engine.hooks = Some(hooks);
    engine.plan_active_flag = Some(Arc::new(AtomicBool::new(false)));
    engine.set_goal("ship the feature".to_owned(), 0);
    engine
}

/// Everything an inline Skill can reach mid-turn, applied through the same
/// production entry points the Skill tool result uses.
fn grant_skill_authority(engine: &mut AgentEngine) {
    engine.apply_context_modifiers(&[Some(ContextModifier {
        model: Some(SKILL_MODEL.to_owned()),
        effort: Some(EffortLevel::High),
        allowed_tools: vec!["Bash".to_owned()],
        plan_mode_transition: Some(PlanModeTransition::Enter),
    })]);
    engine
        .hooks
        .as_mut()
        .unwrap()
        .merge_hooks(HooksConfig {
            post_tool_use: vec![hook("skill-injected")],
            ..Default::default()
        });
    // What `update_goal` and an auto-continuation do to the shared state.
    {
        let goal = engine.goal.as_ref().unwrap().shared_state();
        let mut state = goal.lock().unwrap();
        state.status = GoalStatus::Blocked;
        state.auto_continuations = 3;
    }
    engine.compact_state.record_failure();
    engine.compact_state.last_input_tokens = 123_456;
}

/// Observable authority surface, so one assertion can cover the whole snapshot.
#[derive(Debug, PartialEq)]
struct AuthoritySurface {
    model: String,
    effort: Option<String>,
    hook_names: Vec<String>,
    plan_active: bool,
    plan_flag: bool,
    goal_status: GoalStatus,
    goal_auto_continuations: usize,
    compact_failures: u32,
    compact_watermark: u64,
}

fn surface(engine: &AgentEngine) -> AuthoritySurface {
    let hooks = engine.hooks.as_ref().unwrap().hooks_config();
    AuthoritySurface {
        model: engine.model.clone(),
        effort: engine.current_reasoning_effort.clone(),
        hook_names: hooks
            .pre_tool_use
            .iter()
            .chain(hooks.post_tool_use.iter())
            .chain(hooks.stop.iter())
            .map(|hook| hook.name.clone())
            .collect(),
        plan_active: engine.plan_state.is_active,
        plan_flag: engine
            .plan_active_flag
            .as_ref()
            .unwrap()
            .load(Ordering::Acquire),
        goal_status: engine.goal.as_ref().unwrap().snapshot_state().status,
        goal_auto_continuations: engine
            .goal
            .as_ref()
            .unwrap()
            .snapshot_state()
            .auto_continuations,
        compact_failures: engine.compact_state.consecutive_failures,
        compact_watermark: engine.compact_state.last_input_tokens,
    }
}

/// Run the harmless first pass of one accepted turn and return its context, so
/// the caller can mutate authority the way an inline Skill would and then drive
/// the failing race-tail pass against the same accepted root.
async fn run_first_pass(
    engine: &mut AgentEngine,
    source_message_id: &str,
) -> CompletionEvidenceContext {
    let mut context = CompletionEvidenceContext::new(requirement("Explain the current status."));
    let first = engine
        .execute_turn_with_completion_evidence_context(
            requirement("Explain the current status."),
            "attempt-a",
            source_message_id,
            None,
            Some(&mut context),
        )
        .await
        .unwrap();
    assert!(
        first.completion_adjudication.is_none(),
        "pass A must be an ordinary accepted answer"
    );
    context
}

#[tokio::test]
async fn a2_verdict_retracts_every_authority_the_turn_granted_itself() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-a2",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    let root = surface(&engine);

    let mut context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);
    assert_ne!(surface(&engine), root, "the fixture must actually mutate authority");

    let second_requirement = requirement("Create miniapp.html.");
    context.requirement.extend(second_requirement.clone());
    let second = engine
        .execute_turn_with_completion_evidence_context(
            second_requirement,
            "attempt-b",
            "source-root",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();

    assert!(matches!(
        second.completion_adjudication,
        Some(CompletionAdjudication::UnbackedStateChangeClaim { .. })
    ));
    // The root, not pass A's post-state: a race-tail pass must reuse the single
    // snapshot taken before the first pass of the accepted turn.
    assert_eq!(surface(&engine), root);
    assert_eq!(engine.model, ROOT_MODEL);
    assert!(engine.messages.is_empty(), "both passes rewind to the accepted root");
}

#[tokio::test]
async fn a2_rollback_leaves_the_runtime_equivalent_to_a_fresh_reload() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-reload",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );

    let mut context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);
    let second_requirement = requirement("Create miniapp.html.");
    context.requirement.extend(second_requirement.clone());
    engine
        .execute_turn_with_completion_evidence_context(
            second_requirement,
            "attempt-b",
            "source-root",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();

    // A reload rebuilds the engine from config and the persisted session. Skill
    // grants were never durable, so the live runtime must look identical.
    let reloaded = reloaded_authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-reload",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    assert_eq!(surface(&engine), surface(&reloaded));
}

#[tokio::test]
async fn a_clean_success_keeps_the_authority_the_turn_established() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-success",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );

    let context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);
    let granted = surface(&engine);

    assert!(engine.finalize_committed_completion_turn(&context));

    assert_eq!(
        surface(&engine),
        granted,
        "a committed turn keeps its legitimate modifier, hook, and goal updates"
    );
    assert_eq!(engine.model, SKILL_MODEL);
}

#[tokio::test]
async fn a_host_sealed_success_keeps_the_authority_the_turn_established() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-sealed",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );

    let context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);
    let granted = surface(&engine);

    // The desktop success path: seal before any artifact or terminal is visible.
    assert!(engine.seal_completion_for_host_terminal(&context));

    assert_eq!(surface(&engine), granted);
}

#[tokio::test]
async fn a_host_transaction_failure_retracts_authority() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-host-fail",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    let root = surface(&engine);

    let context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);

    // The manager's artifact / delivery / session-commit failure entry point.
    assert!(engine.restore_uncommitted_completion_turn(&context));
    assert_eq!(surface(&engine), root);
}

#[tokio::test]
async fn a_cancelled_attempt_retracts_authority() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-cancel",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    let root = surface(&engine);

    let context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);

    engine.abort_current_turn("Tool execution canceled by user");
    // The manager's cancellation / provider-error entry point.
    assert!(engine.restore_uncommitted_completion_attempt(&context));
    assert_eq!(surface(&engine), root);
}

#[tokio::test]
async fn a_provider_error_retracts_authority() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-provider-err",
        Arc::new(RaceTailProviderErrorProvider {
            failed: AtomicBool::new(false),
        }),
    );
    let root = surface(&engine);

    let mut context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);

    let error = engine
        .execute_turn_with_completion_evidence_context(
            requirement("Keep going."),
            "attempt-b",
            "source-root",
            None,
            Some(&mut context),
        )
        .await
        .expect_err("the second pass must surface the provider failure");
    assert!(matches!(error, super::AgentError::Provider(_)));
    assert_eq!(surface(&engine), root);
}

#[tokio::test]
async fn goal_progress_is_restored_on_rejection_and_kept_on_success() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-goal",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    // The tool's handle must observe the restore, not just the engine's.
    let tool_handle = engine.goal.as_ref().unwrap().shared_state();

    let rejected = run_first_pass(&mut engine, "source-root").await;
    {
        let mut state = tool_handle.lock().unwrap();
        state.status = GoalStatus::Complete;
        state.auto_continuations = 4;
    }
    assert!(engine.restore_uncommitted_completion_turn(&rejected));
    {
        let observed = tool_handle.lock().unwrap();
        assert_eq!(observed.status, GoalStatus::Active);
        assert_eq!(observed.auto_continuations, 0);
    }

    let committed = run_first_pass(&mut engine, "source-next").await;
    {
        let mut state = tool_handle.lock().unwrap();
        state.status = GoalStatus::Complete;
        state.auto_continuations = 4;
    }
    assert!(engine.finalize_committed_completion_turn(&committed));
    {
        let observed = tool_handle.lock().unwrap();
        assert_eq!(observed.status, GoalStatus::Complete);
        assert_eq!(observed.auto_continuations, 4);
    }
}


#[tokio::test]
async fn the_hook_rollback_preserves_the_supervised_shell() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut engine = authority_engine(
        workspace.path(),
        sessions.path(),
        "authority-hooks",
        Arc::new(RaceTailFalseCompletionProvider::new()),
    );
    let supervisor = Arc::new(nomi_process_runtime::ProcessSupervisor::new(
        Default::default(),
    ));
    engine.set_process_supervisor(Arc::clone(&supervisor));

    let context = run_first_pass(&mut engine, "source-root").await;
    grant_skill_authority(&mut engine);
    assert!(engine.restore_uncommitted_completion_turn(&context));

    assert_eq!(
        engine
            .hooks
            .as_ref()
            .unwrap()
            .hooks_config()
            .post_tool_use
            .len(),
        0,
        "the skill's merged hook is gone"
    );
    // A replaced config must not have detached the engine's process authority.
    let retained = engine
        .process_supervisor_handle()
        .expect("the engine keeps its process supervisor across a hook rollback");
    assert!(Arc::ptr_eq(&retained, &supervisor));
}
