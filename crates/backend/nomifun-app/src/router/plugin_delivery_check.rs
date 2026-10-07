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
use nomifun_agent_runtime::{AgentCompletionCheck, AgentCompletionCheckPort, AgentEngineError, AgentEngineEvent};
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
    /// Native completion reports may yield to the owner; no plugin-specific
    /// retry ledger or execution limit is maintained.
    blocked_report: Mutex<bool>,
}

impl PluginDeliveryCheck {
    pub(super) fn new(
        session_host: Arc<EngineSessionHost>,
        receipt: Arc<EngineTurnReceipt>,
        plugin_tools: BTreeMap<String, String>,
    ) -> Self {
        Self { session_host, receipt, plugin_tools, blocked_report: Mutex::new(false) }
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

/// Map a durable delivery gap onto the settlement answer. An unbound same-conversation Action pause at the gate; actionable gaps
/// reject with feedback inside the turn.
fn gap_outcome(
    gap: &super::engine_session_host::PluginDeliveryGap,
    plugin_tools: &BTreeMap<String, String>,
) -> AgentCompletionCheck {
    match gap.reason {
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
        if let AgentEngineEvent::CompletionReported { report } | AgentEngineEvent::CompletionCandidateRecorded { report } = event {
            *self.blocked_report.lock().map_err(|_| AgentEngineError::InvalidContract("Plugin completion state poisoned".into()))? = report.is_blocked();
        }
        Ok(())
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
            // Tool attachment is a safe-point flow that keeps pausing through
            // the final gate. Same-conversation consumption only pauses while
            // the installed Action is still unbound; once the rebuilt turn
            // exposes it, the check rejects with the planned call.
            Ok(PluginDeliveryState::Gap(gap)) => {
                let blocked = *self.blocked_report.lock().map_err(|_| AgentEngineError::InvalidContract("Plugin completion state poisoned".into()))?;
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
