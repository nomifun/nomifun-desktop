//! In-turn host settlement check for Plugin delivery obligations.
//!
//! The engine consults this port before an otherwise complete turn settles.
//! It answers only from the same durable delivery facts the post-turn
//! completion gate reads, so a missing install or verification relays
//! actionable feedback into the same turn instead of pausing silently, and a
//! proven delivery answers Delivered. User/safe-point flows keep pausing
//! through the post-turn gate, which remains the final authority.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::plugin::plugin_action_id;
use nomifun_agent_contracts::PluginId;
use nomifun_agent_runtime::{AgentCompletionCheck, AgentCompletionCheckPort, AgentEngineError, AgentEngineEvent, AgentExecutionPressure, AgentExecutionStopReason};
use nomifun_chat_model_broker::ChatCausality;
use nomifun_common::AppError;

use super::engine_session_host::{EngineSessionHost, EngineTurnReceipt, PlannedConversationCase, PluginDeliveryState};

/// The turn's admitted receipt and its bound Plugin tools; each check reads
/// only the durable Plugin delivery facts the post-turn completion gate
/// reads.
pub(super) struct PluginDeliveryCheck {
    session_host: Arc<EngineSessionHost>,
    receipt: Arc<EngineTurnReceipt>,
    /// Unified Plugin Action capability ids → model tool names bound on this
    /// turn's plan.
    plugin_tools: BTreeMap<String, String>,
    /// Replay-derived watchdog only; canonical tool facts and the native
    /// checkpoint remain the sole persisted execution/recovery chain.
    progress: Mutex<VerificationProgress>,
}

impl PluginDeliveryCheck {
    pub(super) fn new(
        session_host: Arc<EngineSessionHost>,
        receipt: Arc<EngineTurnReceipt>,
        plugin_tools: BTreeMap<String, String>,
    ) -> Self {
        let required = receipt.request_payload().get("plugin_delivery").is_some_and(|value| !value.is_null());
        Self { session_host, receipt, plugin_tools, progress: Mutex::new(VerificationProgress::new(required)) }
    }
}

const MAX_FAILED_CASE_ATTEMPTS: u8 = 3;
const MAX_TRACKED_CASES: usize = 512;
const MAX_CREATION_SPAN_STEPS: u16 = 64;

#[derive(Default)]
struct VerificationProgress {
    pending: BTreeMap<String, (String, String, bool)>,
    failures: BTreeMap<(String, String), u8>,
    passed: std::collections::BTreeSet<(String, String)>,
    blocked_report: bool,
    full: bool,
    delivery_required: bool,
    latest_step: u16,
    active_start_step: Option<u16>,
}

impl VerificationProgress {
    fn new(required: bool) -> Self {
        Self { delivery_required: required, active_start_step: required.then_some(0), ..Self::default() }
    }

    fn observe(&mut self, event: &AgentEngineEvent) {
        match event {
            // An explicitly accepted owner input starts a new repair span;
            // checkpoint recovery by itself never buys automatic retries.
            AgentEngineEvent::SteeringInputs { inputs } if !inputs.is_empty() => {
                let latest = self.latest_step;
                *self = Self::new(self.delivery_required);
                self.latest_step = latest;
                self.active_start_step = self.delivery_required.then_some(latest);
            }
            AgentEngineEvent::ModelStepStarted { step, .. } => self.latest_step = *step,
            AgentEngineEvent::ToolCallCompleted { call, .. } => {
                if let (Some(draft), Some(case)) = (call.arguments.0["draft_id"].as_str(), call.arguments.0["case_name"].as_str()) {
                    self.pending.insert(call.call_id.as_ref().to_owned(), (draft.to_owned(), case.to_owned(), false));
                }
            }
            AgentEngineEvent::ToolStarted { step, call_id, capability_id, action_id, .. }
                if *step > 0 && capability_id.as_ref() == "plugin.development" => {
                if matches!(action_id.as_ref(), "plugin.development/open" | "plugin.development/plan"
                    | "plugin.development/apply" | "plugin.development/check" | "plugin.development/preview"
                    | "plugin.development/test_ui" | "plugin.development/test_action" | "plugin.development/install") {
                    self.active_start_step.get_or_insert(step.saturating_sub(1));
                }
                if matches!(action_id.as_ref(), "plugin.development/test_ui" | "plugin.development/test_action")
                    && let Some(call) = self.pending.get_mut(call_id.as_ref()) { call.2 = true; }
            }
            AgentEngineEvent::ToolCompleted { result, .. } => {
                let Some((draft, case, admitted)) = self.pending.remove(result.call_id.as_ref()) else { return; };
                if !admitted || result.is_error { return; }
                let Ok(output) = serde_json::from_str::<serde_json::Value>(&result.output_text()) else { return; };
                // These are host-authored result fields, correlated with the
                // admitted module action. Plugin diagnostics cannot add cases.
                if output["planned"] != serde_json::json!(true)
                    || output["draft_id"].as_str() != Some(draft.as_str())
                    || output["case_name"].as_str() != Some(case.as_str()) { return; }
                let key = (draft.clone(), case);
                if self.failures.len() + self.passed.len() >= MAX_TRACKED_CASES
                    && !self.failures.contains_key(&key) && !self.passed.contains(&key) {
                    self.full = true; return;
                }
                if output["passed"] == serde_json::json!(true) {
                    if self.passed.insert(key) {
                        self.failures.retain(|(id, _), _| id != &draft);
                    }
                } else if output["passed"] == serde_json::json!(false) {
                    let attempts = self.failures.entry(key).or_default();
                    *attempts = attempts.saturating_add(1);
                }
            }
            AgentEngineEvent::CompletionReported { report } | AgentEngineEvent::CompletionCandidateRecorded { report } => {
                self.blocked_report = report.is_blocked();
            }
            _ => {}
        }
    }

    fn stalled(&self) -> bool {
        self.full || self.failures.values().any(|attempts| *attempts >= MAX_FAILED_CASE_ATTEMPTS)
            || self.active_start_step.is_some_and(|start| self.latest_step.saturating_sub(start) >= MAX_CREATION_SPAN_STEPS)
    }
}

/// Once the resumed turn actually binds the planned Plugin Action, relay the
/// exact call the plan expects instead of pausing on the same gap again.
fn pending_conversation_feedback(
    plugin_tools: &BTreeMap<String, String>,
    case: &PlannedConversationCase,
) -> Option<String> {
    let stable = plugin_action_id(&PluginId::from(case.plugin_id.clone()), &case.action);
    let model_name = plugin_tools.get(&stable)?;
    let input = serde_json::to_string(&case.input).unwrap_or_default();
    let expected = serde_json::to_string(&case.expected_output).unwrap_or_default();
    Some(format!(
        "PLUGIN_CURRENT_CONVERSATION_PENDING: the installed tool `{model_name}` \
         (Action {stable}) is attached to this conversation now. Call it once with \
         exactly this input: {input}; the plan expects {expected}. Then give the \
         final answer."
    ))
}

impl std::fmt::Debug for PluginDeliveryCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginDeliveryCheck").finish_non_exhaustive()
    }
}

/// Map a durable delivery gap onto the settlement answer. User approval and
/// an unbound same-conversation Action pause at the gate; actionable gaps
/// reject with feedback inside the turn.
fn gap_outcome(
    gap: &super::engine_session_host::PluginDeliveryGap,
    plugin_tools: &BTreeMap<String, String>,
) -> AgentCompletionCheck {
    match gap.reason {
        "PLUGIN_AUTHORIZATION_REQUIRED" => AgentCompletionCheck::HostPause,
        "PLUGIN_CURRENT_CONVERSATION_PENDING" => gap
            .current_conversation
            .as_ref()
            .and_then(|case| pending_conversation_feedback(plugin_tools, case))
            .map(AgentCompletionCheck::Reject)
            .unwrap_or(AgentCompletionCheck::HostPause),
        _ => AgentCompletionCheck::Reject(format!("{}: {}", gap.reason, gap.detail)),
    }
}

#[async_trait]
impl AgentCompletionCheckPort for PluginDeliveryCheck {
    fn observe(&self, event: &AgentEngineEvent) -> Result<(), AgentEngineError> {
        self.progress.lock().map_err(|_| AgentEngineError::InvalidContract("Plugin verification watchdog poisoned".into()))?.observe(event);
        Ok(())
    }

    async fn execution_pressure(&self) -> Result<AgentExecutionPressure, AgentEngineError> {
        let stalled = self.progress.lock().map_err(|_| AgentEngineError::InvalidContract("Plugin verification watchdog poisoned".into()))?.stalled();
        // Check durable delivery only when the watchdog would stop. A late
        // successful install/consumption must still be allowed to settle.
        let settled = stalled && matches!(self.session_host.plugin_delivery_status(self.receipt.as_ref()).await,
            Ok(PluginDeliveryState::Delivered | PluginDeliveryState::Dormant));
        Ok(AgentExecutionPressure { renew_window: false,
            stop: (stalled && !settled).then_some(AgentExecutionStopReason::PluginVerificationStalled) })
    }

    async fn check(&self, causality: &ChatCausality) -> Result<AgentCompletionCheck, AgentEngineError> {
        if causality.agent_session_id.as_ref() != self.receipt.session().session().conversation_id
            || causality.turn_operation_id.as_ref() != self.receipt.operation_id()
            || causality.causation_event_id.as_ref() != self.receipt.root_message_id()
        {
            tracing::warn!("Plugin delivery check causality mismatch; the post-turn gate decides");
            return Ok(AgentCompletionCheck::Settle);
        }
        let status: Result<_, AppError> =
            self.session_host.plugin_delivery_status(self.receipt.as_ref()).await;
        match status {
            // User approval is a safe-point flow that keeps pausing through
            // the final gate. Same-conversation consumption only pauses while
            // the installed Action is still unbound; once the rebuilt turn
            // exposes it, the check rejects with the planned call.
            Ok(PluginDeliveryState::Gap(gap)) => {
                let blocked = self.progress.lock().map_err(|_| AgentEngineError::InvalidContract("Plugin verification watchdog poisoned".into()))?.blocked_report;
                Ok(if blocked { AgentCompletionCheck::HostPause } else { gap_outcome(&gap, &self.plugin_tools) })
            }
            Ok(PluginDeliveryState::Delivered) => Ok(AgentCompletionCheck::Delivered),
            Ok(PluginDeliveryState::Dormant) => Ok(AgentCompletionCheck::Settle),
            Err(error) => {
                // The post-turn completion gate still decides; never fail the
                // turn on a settlement-check read error.
                tracing::warn!(
                    agent_session_id = self.receipt.session().session().conversation_id,
                    error = %nomi_redact::redact_secrets_owned(error.to_string()),
                    "Plugin delivery check failed; the post-turn gate decides",
                );
                Ok(AgentCompletionCheck::Settle)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::engine_session_host::PluginDeliveryGap;
    use super::*;

    fn verification_events(id: &str, draft: &str, case: &str, planned: bool, passed: bool, revision: u64) -> Vec<AgentEngineEvent> {
        vec![
            AgentEngineEvent::ToolCallCompleted { step: 1, call: nomifun_chat_model_broker::ChatToolCall {
                call_id: id.into(), name: "development_test_ui".into(), provider_metadata: None,
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"draft_id":draft,"case_name":case})),
            } },
            AgentEngineEvent::ToolStarted { step: 1, call_id: id.into(),
                capability_id: "plugin.development".into(), action_id: "plugin.development/test_ui".into() },
            AgentEngineEvent::ToolCompleted { step: 1, result: nomifun_agent_runtime::AgentToolResult::text(id.into(),
                json!({"draft_id":draft,"case_name":case,"planned":planned,"passed":passed,"revision":revision}).to_string(), false) },
        ]
    }

    #[test]
    fn planned_case_stall_ignores_revision_churn_and_diagnostic_cases() {
        let mut progress = VerificationProgress::default();
        for revision in 1..=10 {
            for event in verification_events(&format!("probe-{revision}"), "draft", "probe", false, false, revision) { progress.observe(&event); }
        }
        assert!(!progress.stalled(), "diagnostic probes are never acceptance progress or failures");
        for revision in 11..=13 {
            for event in verification_events(&format!("required-{revision}"), "draft", "persist", true, false, revision) { progress.observe(&event); }
        }
        assert!(progress.stalled(), "three failed planned attempts stop even though every revision changed");
    }

    #[test]
    fn only_new_planned_case_progress_in_the_same_draft_resets_stall() {
        let mut progress = VerificationProgress::default();
        for attempt in 1..=3 {
            for event in verification_events(&format!("fail-{attempt}"), "draft", "persist", true, false, attempt) { progress.observe(&event); }
        }
        for event in verification_events("probe-pass", "draft", "probe", false, true, 4) { progress.observe(&event); }
        for event in verification_events("other-pass", "other-draft", "persist", true, true, 4) { progress.observe(&event); }
        assert!(progress.stalled());
        for event in verification_events("add-pass", "draft", "add", true, true, 4) { progress.observe(&event); }
        assert!(!progress.stalled());
        for attempt in 5..=7 {
            for event in verification_events(&format!("retry-{attempt}"), "draft", "persist", true, false, attempt) { progress.observe(&event); }
        }
        for event in verification_events("repeat-add", "draft", "add", true, true, 8) { progress.observe(&event); }
        assert!(progress.stalled(), "repeating a settled pass cannot buy endless failed retries");
    }

    #[test]
    fn watchdog_replays_canonical_failures_and_only_accepted_input_resets_them() {
        let events: Vec<_> = (1..=3).flat_map(|attempt|verification_events(&format!("case-{attempt}"), "draft", "persist", true, false, attempt)).collect();
        let mut replay = VerificationProgress::default();
        for event in &events { replay.observe(event); }
        assert!(replay.stalled(), "recovery must retain the pause condition");
        replay.observe(&AgentEngineEvent::SteeringInputs { inputs: vec![] });
        assert!(replay.stalled());
        replay.observe(&AgentEngineEvent::SteeringInputs { inputs: vec![nomifun_agent_runtime::AgentSteeringInput {
            receipt_operation_id: "reply".into(), message_id: "input".into(), text: "Continue with a fix".into(),
            files: vec![], inject_skills: vec![], image_count: 0, prepared_images: vec![], prepared_skill_instructions: vec![],
        }] });
        assert!(!replay.stalled());
    }

    #[test]
    fn creation_span_has_a_hard_model_bound_but_neutral_draft_context_does_not() {
        let mut required = VerificationProgress::new(true);
        let mut neutral = VerificationProgress::new(false);
        for step in 1..=MAX_CREATION_SPAN_STEPS {
            let event = AgentEngineEvent::ModelStepStarted { step, operation_id: format!("model-{step}").into() };
            required.observe(&event); neutral.observe(&event);
        }
        assert!(required.stalled());
        assert!(!neutral.stalled(), "having draft context cannot limit an unrelated conversation");
        neutral.observe(&AgentEngineEvent::ToolStarted { step: MAX_CREATION_SPAN_STEPS,
            call_id: "read".into(), capability_id: "plugin.development".into(), action_id: "plugin.development/read".into() });
        assert!(neutral.active_start_step.is_none());
        neutral.observe(&AgentEngineEvent::ToolStarted { step: MAX_CREATION_SPAN_STEPS,
            call_id: "apply".into(), capability_id: "plugin.development".into(), action_id: "plugin.development/apply".into() });
        for step in MAX_CREATION_SPAN_STEPS + 1..MAX_CREATION_SPAN_STEPS * 2 {
            neutral.observe(&AgentEngineEvent::ModelStepStarted { step, operation_id: format!("model-{step}").into() });
        }
        assert!(neutral.stalled(), "actual authoring starts a bounded creation span");
    }

    fn case() -> PlannedConversationCase {
        PlannedConversationCase {
            plugin_id: "019b0000-0000-7000-8000-000000000123".into(),
            action: "trim_and_uppercase".into(),
            input: json!({"text": "  hello  "}),
            expected_output: json!({"text": "HELLO"}),
        }
    }

    #[test]
    fn pending_conversation_relays_the_call_once_the_tool_is_bound() {
        let case = case();
        let stable = plugin_action_id(
            &PluginId::from(case.plugin_id.clone()),
            &case.action,
        );
        let tools = BTreeMap::from([(stable.clone(), "plugin_attached".to_owned())]);
        let feedback = pending_conversation_feedback(&tools, &case).unwrap();
        assert!(feedback.contains("plugin_attached"), "{feedback}");
        assert!(feedback.contains(&stable), "{feedback}");
        assert!(feedback.contains(r#"{"text":"  hello  "}"#), "{feedback}");
        assert!(feedback.contains(r#"{"text":"HELLO"}"#), "{feedback}");
        assert!(pending_conversation_feedback(&BTreeMap::new(), &case).is_none());
        assert!(
            pending_conversation_feedback(
                &BTreeMap::from([("plugin:019b0000-0000-7000-8000-000000000999/other".to_owned(), "other".to_owned())]),
                &case,
            )
            .is_none()
        );
    }

    #[test]
    fn gap_outcome_pauses_only_host_owned_gaps() {
        let case = case();
        let stable = plugin_action_id(
            &PluginId::from(case.plugin_id.clone()),
            &case.action,
        );
        let tools = BTreeMap::from([(stable.clone(), "plugin_attached".to_owned())]);
        let pending = PluginDeliveryGap {
            reason: "PLUGIN_CURRENT_CONVERSATION_PENDING",
            detail: "draft is delivered".to_owned(),
            current_conversation: Some(case),
        };
        let outcome = gap_outcome(&pending, &tools);
        assert!(
            matches!(outcome, AgentCompletionCheck::Reject(ref feedback)
                if feedback.contains("plugin_attached") && feedback.contains(&stable)),
            "{outcome:?}"
        );
        assert_eq!(
            gap_outcome(&pending, &BTreeMap::new()),
            AgentCompletionCheck::HostPause,
            "the gate pauses while the installed Action is still unbound"
        );
        let approval = PluginDeliveryGap {
            reason: "PLUGIN_AUTHORIZATION_REQUIRED",
            detail: "approval needed".to_owned(),
            current_conversation: None,
        };
        assert_eq!(gap_outcome(&approval, &tools), AgentCompletionCheck::HostPause);
        let missing = PluginDeliveryGap {
            reason: "PLUGIN_DELIVERY_REQUIRED",
            detail: "not installed".to_owned(),
            current_conversation: None,
        };
        assert_eq!(
            gap_outcome(&missing, &tools),
            AgentCompletionCheck::Reject(
                "PLUGIN_DELIVERY_REQUIRED: not installed".to_owned()
            )
        );
    }
}
