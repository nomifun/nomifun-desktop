//! Host-opted Robot reply intent. These turn-local observations never prove
//! physical movement or replace owner effects, checkpoints or completion gates.
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ToolCallId};

use crate::{AgentEffectClass, AgentToolBinding, AgentToolPlan, AgentToolResult};

pub const FINISH_WITHOUT_REPLY_TOOL_NAME: &str = "finish_without_reply";
pub(crate) const TOOL_NAME: &str = FINISH_WITHOUT_REPLY_TOOL_NAME;
pub(crate) const INSTRUCTIONS: &str = "For an unambiguous physical action request with no question or requested spoken confirmation, perform every requested action without public narration, then use finish_without_reply instead of a verbal receipt. You may append it LAST after all final motion/display calls in the SAME batch; the host executes those actions in order and accepts the finish only after their successful tool receipts. For a provider that emits one tool at a time, call finish_without_reply alone after the last successful action. Never use it after only the first action of a sequence, when an answer, clarification, explanation, result or user-requested confirmation is still owed, after a failure or uncertain outcome, or to end non-Robot work. For mixed action-and-question requests, execute the actions and answer normally. The control declares only that no public reply is needed; a device accepting a command is not proof that the physical motion finished. Do not repeat already accepted actions to obtain this control.";

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Explicitly finish this entire physical Robot interaction without a public reply, only when every requested action has been arranged and no answer or confirmation is owed. Submit once, last after motion/display calls in the final batch, or alone after prior successful motion/display calls in this turn. Actual owner receipts, failure, cancellation, steering and host completion gates remain authoritative. This is reply intent, not proof of completed physical motion.".into(),
        input_schema: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({
            "type":"object", "properties":{}, "additionalProperties":false,
        })),
        deferred: false,
    }
}

pub(crate) fn actuator(binding: &AgentToolBinding) -> bool {
    binding.capability_id.as_ref() == "robot"
        && matches!(binding.action_id.as_ref(), "robot/motion" | "robot/display")
        && !matches!(binding.effect_class, AgentEffectClass::ReadOnly)
}

#[derive(Default)]
pub(crate) struct SilentRobotCompletion {
    pub(crate) enabled: bool,
    blocked: bool,
    pub(crate) tool_call_ids: Vec<ToolCallId>,
}

impl SilentRobotCompletion {
    pub(crate) fn new(enabled: bool) -> Self { Self { enabled, ..Self::default() } }

    pub(crate) fn allows_batch(&self, calls: &[ChatToolCall], plan: &AgentToolPlan) -> bool {
        self.enabled && !self.blocked
            && calls.last().is_some_and(|call| call.name == TOOL_NAME)
            && calls.iter().filter(|call| call.name == TOOL_NAME).count() == 1
            && (calls.len() > 1 || !self.tool_call_ids.is_empty())
            && calls[..calls.len() - 1].iter().all(|call|
                plan.binding(&call.name).is_some_and(actuator))
    }

    pub(crate) fn observe(&mut self, call: &ChatToolCall, binding: Option<&AgentToolBinding>,
        attempted: bool, result: &AgentToolResult) {
        if !self.enabled { return; }
        if result.is_error {
            self.blocked = true;
        } else if let Some(binding) = binding {
            if attempted && actuator(binding) { self.tool_call_ids.push(call.call_id.clone()); }
            else { self.blocked = true; }
        } else if !matches!(call.name.as_str(), TOOL_NAME | crate::tool_discovery::TOOL_NAME) {
            // Other controls can load results or establish obligations that
            // this intentionally narrow reply shortcut cannot account for.
            self.blocked = true;
        }
    }

    pub(crate) fn ready(&self) -> bool {
        self.enabled && !self.blocked && !self.tool_call_ids.is_empty()
    }
}
