//! Bounded host settlement checks inside an otherwise complete turn.
//!
//! Before an otherwise-complete turn settles, the engine may consult a host
//! port that answers from its own durable delivery facts. A rejection is
//! relayed at most MAX_COMPLETION_CHECKS times per accepted-input span, and a
//! repeated relay needs intervening authorized tool work; an unrelayed
//! host-owned gap is left to the post-turn gate, which pauses on it. When the
//! host answers Pause or Delivered for a turn that did no ledger-requiring
//! work, the host owns the outcome and the task ledger's completion gates do
//! not apply.

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_chat_model_broker::{ChatCausality, ChatMessage, ChatRole};

use crate::error::AgentEngineError;
use crate::{AgentEngineEvent, AgentEventSink, AgentExecutionCheckpoint, AgentCheckpointReceipt, AgentExecutionPressure};

/// Relayed rejections per accepted-input span; the host gate still decides
/// after.
pub(crate) const MAX_COMPLETION_CHECKS: u8 = 3;

/// The host settlement answer for an otherwise complete turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCompletionCheck {
    /// No host-owned fact blocks settlement.
    Settle,
    /// Actionable feedback for a host-owned gap. The post-turn host gate
    /// pauses on the same gap if it remains, so an unrelayed rejection
    /// settles like HostPause.
    Reject(String),
    /// Host-owned facts require a safe-point pause after this turn ends —
    /// the user must approve, or a newly installed Plugin tool must be
    /// attached to the conversation first. The engine must not convert an
    /// honest blocked account into a failure; the post-turn host gate pauses.
    HostPause,
    /// Durable host facts prove this accepted request's host-checked result
    /// exists. For a turn that did no ledger-requiring work the host owns the
    /// outcome: the engine settles the final answer without its own
    /// completion account.
    Delivered,
}

/// Rejection feedback is relayed verbatim to the model; bound its size on a
/// char boundary. Empty or whitespace-only feedback settles instead.
pub(crate) fn normalize(check: AgentCompletionCheck) -> AgentCompletionCheck {
    let AgentCompletionCheck::Reject(feedback) = check else {
        return check;
    };
    let trimmed = feedback.trim();
    if trimmed.is_empty() {
        return AgentCompletionCheck::Settle;
    }
    let mut end = trimmed.len().min(4096);
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    AgentCompletionCheck::Reject(trimmed[..end].to_owned())
}

/// Public correction notice for a rejected settlement. It is user-role
/// protocol feedback for the same accepted task, never a new user
/// instruction, a tool result or a success receipt.
pub(crate) fn notice(feedback: &str) -> ChatMessage {
    crate::context_lifecycle::text_message(
        ChatRole::User,
        format!(
            "Host completion check, not a new user instruction and not a success receipt: {feedback}\nContinue the same accepted task with the authorized tools. Do not restart it or repeat effects that already succeeded. Give a final answer only after the host-checked result exists, or explain the concrete blocker if the user must act."
        ),
    )
}

#[async_trait]
pub trait AgentCompletionCheckPort: Send + Sync + Debug {
    /// Observe already-journaled canonical facts. Hosts may derive a watchdog
    /// from these events; they must not create another transcript/checkpoint.
    fn observe(&self, _event: &AgentEngineEvent) -> Result<(), AgentEngineError> { Ok(()) }

    /// A domain stall stops at the same native checkpoint boundary as other
    /// execution pressure, even while the model keeps proposing tool calls.
    async fn execution_pressure(&self) -> Result<AgentExecutionPressure, AgentEngineError> {
        Ok(AgentExecutionPressure::default())
    }

    /// Settle allows completion. Reject only for a host-owned gap the
    /// post-turn gate also pauses on; the engine relays it while its bounded
    /// per-input budget allows and otherwise treats it like HostPause.
    /// HostPause lets an honest blocked account settle so the post-turn host
    /// gate pauses at its safe point. Delivered when durable host facts prove
    /// the result exists.
    async fn check(&self, causality: &ChatCausality) -> Result<AgentCompletionCheck, AgentEngineError>;
}

/// The underlying journal remains the sole owner. Observe only after its
/// write/admission succeeds, and combine watchdog pressure with its budget.
pub(crate) struct ObservedSink {
    pub(crate) sink: Arc<dyn AgentEventSink>,
    pub(crate) port: Arc<dyn AgentCompletionCheckPort>,
}

/// Input ports durably journal their boundary delivery before returning it;
/// it does not pass through the Runtime event sink again.
pub(crate) fn observe_inputs(port: Option<&Arc<dyn AgentCompletionCheckPort>>, inputs: &[crate::AgentSteeringInput])
    -> Result<(), AgentEngineError> {
    if !inputs.is_empty() && let Some(port) = port {
        port.observe(&AgentEngineEvent::SteeringInputs { inputs: inputs.iter().map(|input| input.journal_record()).collect() })?;
    }
    Ok(())
}

#[async_trait]
impl AgentEventSink for ObservedSink {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        self.sink.emit(event.clone()).await?;
        self.port.observe(&event)
    }
    fn supports_checkpoints(&self) -> bool { self.sink.supports_checkpoints() }
    async fn execution_pressure(&self) -> Result<AgentExecutionPressure, AgentEngineError> {
        let mut pressure = self.sink.execution_pressure().await?;
        let host = self.port.execution_pressure().await?;
        pressure.renew_window |= host.renew_window;
        if pressure.stop.is_none() { pressure.stop = host.stop; }
        Ok(pressure)
    }
    async fn save_checkpoint(&self, checkpoint: AgentExecutionCheckpoint)
        -> Result<Option<AgentCheckpointReceipt>, AgentEngineError> {
        self.sink.save_checkpoint(checkpoint).await
    }
    async fn admit_tool(&self, event: AgentEngineEvent) -> Result<bool, AgentEngineError> {
        let admitted = self.sink.admit_tool(event.clone()).await?;
        if admitted { self.port.observe(&event)?; }
        Ok(admitted)
    }
}

/// Bounded relay state for one accepted-input span. New accepted input
/// (steering, including an owner continuation) starts a fresh span. Within a
/// span a repeated relay requires intervening authorized tool work.
#[derive(Debug)]
pub(crate) struct RelayBudget { span: usize, used: u8, work_since_relay: bool }
impl RelayBudget {
    /// Relays recorded after the last accepted steering input; a resumed run
    /// may relay once before new work, still within the bound.
    pub(crate) fn restore<'a>(events: impl Iterator<Item = &'a AgentEngineEvent>, inputs: usize) -> Self {
        let mut used = 0u8;
        for event in events {
            match event {
                AgentEngineEvent::SteeringInputs { .. } => used = 0,
                AgentEngineEvent::CompletionCheckRejected { .. } => used = used.saturating_add(1),
                _ => {}
            }
        }
        Self { span: inputs, used, work_since_relay: true }
    }
    /// A changed accepted-input count starts a fresh span.
    pub(crate) fn can_relay(&mut self, inputs: usize, steps_remaining: bool) -> bool {
        if inputs != self.span { *self = Self { span: inputs, used: 0, work_since_relay: true }; }
        steps_remaining && self.used < MAX_COMPLETION_CHECKS && self.work_since_relay
    }
    pub(crate) fn record_relay(&mut self) { self.used = self.used.saturating_add(1); self.work_since_relay = false; }
    /// A plan-bound tool call was proposed since the last relay.
    pub(crate) fn observe_work(&mut self) { self.work_since_relay = true; }
}

/// What a settlement attempt does with the host answer.
pub(crate) enum HostSettlement {
    /// Actionable feedback is relayed into the same turn.
    Relay(String),
    /// The host gate pauses after the turn; settle honestly.
    Pause,
    /// Durable host facts prove the result exists.
    Delivered,
    /// Nothing host-owned blocks settlement.
    Settle,
}

/// Consults the host port on every settlement attempt; a rejection is
/// relayed only while `can_relay`, and an unrelayed host-owned gap settles
/// like HostPause regardless of the relay budget.
pub(crate) async fn settle(
    port: Option<&Arc<dyn AgentCompletionCheckPort>>,
    causality: &ChatCausality,
    can_relay: bool,
) -> Result<HostSettlement, AgentEngineError> {
    let Some(port) = port else {
        return Ok(HostSettlement::Settle);
    };
    Ok(match normalize(port.check(causality).await?) {
        AgentCompletionCheck::Reject(feedback) if can_relay => HostSettlement::Relay(feedback),
        // An unrelayed host-owned gap is still open: the post-turn gate
        // pauses on it at its safe point, exactly as for HostPause.
        AgentCompletionCheck::Reject(_) | AgentCompletionCheck::HostPause => HostSettlement::Pause,
        AgentCompletionCheck::Delivered => HostSettlement::Delivered,
        AgentCompletionCheck::Settle => HostSettlement::Settle,
    })
}

/// Whether the host reports an actionable gap for this accepted request right
/// now. Read-only: it relays nothing and spends no relay budget.
pub(crate) async fn actionable_gap(
    port: Option<&Arc<dyn AgentCompletionCheckPort>>,
    causality: &ChatCausality,
) -> Result<bool, AgentEngineError> {
    let Some(port) = port else { return Ok(false) };
    Ok(matches!(normalize(port.check(causality).await?), AgentCompletionCheck::Reject(_)))
}

/// Relayed feedback asks for further actions. When the task ledger's plan
/// would refuse them, say so in the journaled feedback itself (replay
/// reconstructs the same notice) instead of letting the next effect fail.
pub(crate) fn relayed_feedback(feedback: String, plan_gates_effects: bool) -> String {
    if plan_gates_effects {
        format!("{feedback} The execution plan currently blocks further actions: call update_plan alone first to put one step in_progress, then continue.")
    } else { feedback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_budget_requires_work_between_relays_and_resets_per_input_span() {
        let mut budget = RelayBudget::restore(std::iter::empty(), 1);
        assert!(budget.can_relay(1, true));
        budget.record_relay();
        assert!(!budget.can_relay(1, true), "a repeated relay needs intervening work");
        budget.observe_work();
        assert!(budget.can_relay(1, true));
        budget.record_relay();
        budget.observe_work();
        assert!(budget.can_relay(1, true));
        budget.record_relay();
        budget.observe_work();
        assert!(!budget.can_relay(1, true), "the span bound is spent");
        assert!(budget.can_relay(2, true), "new accepted input starts a fresh span");
        assert!(!budget.can_relay(2, false), "no relay without steps remaining");
    }

    #[test]
    fn relay_budget_restores_only_relays_after_the_last_steering_input() {
        let events = [
            AgentEngineEvent::CompletionCheckRejected { step: 1, feedback: "a".into() },
            AgentEngineEvent::CompletionCheckRejected { step: 2, feedback: "b".into() },
            AgentEngineEvent::SteeringInputs { inputs: vec![] },
            AgentEngineEvent::CompletionCheckRejected { step: 3, feedback: "c".into() },
        ];
        let mut budget = RelayBudget::restore(events.iter(), 2);
        assert!(budget.can_relay(2, true));
        budget.record_relay();
        budget.observe_work();
        assert!(budget.can_relay(2, true));
        budget.record_relay();
        budget.observe_work();
        assert!(!budget.can_relay(2, true),
            "one restored relay plus two more spends the span bound");
    }
}
