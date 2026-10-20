//! Read-only references into canonical message bodies and tool calls.
//! These values are presentation metadata, never a second transcript or execution fact.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Read-only content ownership, shared by live buffering and history. A text
/// delta is not proof that the same model step has finished its reasoning body.
/// This classifier adds no persisted event or recovery contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningDisplayEvent { Reasoning, Narration, Invocation, Boundary, Other }

pub fn reasoning_display_event(kind: &str) -> ReasoningDisplayEvent {
    match kind {
        "reasoning_delta" => ReasoningDisplayEvent::Reasoning,
        "output_text_delta" => ReasoningDisplayEvent::Narration,
        // An already-known invocation does not create another history boundary.
        "tool_call_delta" | "tool_call_completed" | "tool_started" => ReasoningDisplayEvent::Invocation,
        "model_step_started" | "execution_resumed" | "completion_delivered"
        | "model_output_truncated" | "model_response_rejected"
        | "delivery_review_superseded" | "voice_model_step_superseded"
        | "silent_completion" | "turn_completed" | "user_input_requested"
        | "turn_waiting" | "turn_input_required" | "turn_cancelled" | "turn_paused"
        | "turn_failed" | "turn/steer-accepted" => ReasoningDisplayEvent::Boundary,
        // Results, usage and physical buffer flushes cannot take content ownership.
        _ => ReasoningDisplayEvent::Other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum TimelineSegmentKind { Reasoning, Narration, Tool }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum TimelineSegmentState { Running, Completed, Failed, Cancelled, Interrupted }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct MessageTimelineSegment {
    /// The first canonical event in this uninterrupted source segment.
    pub segment_id: String,
    /// Stable process anchor: the accepted root, or preceding narration segment.
    /// Narration itself is independent of process disclosure.
    pub group_id: String,
    #[ts(type = "number")]
    pub first_seq: u64,
    #[ts(type = "number")]
    pub last_seq: u64,
    pub kind: TimelineSegmentKind,
    pub state: TimelineSegmentState,
    /// Half-open range in the owning projection's content, measured in UTF-16
    /// code units, matching JavaScript String.slice. Absent for tool segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub start_utf16: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub end_utf16: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct MessageTimeline {
    /// Exact committed Session cursor of the read snapshot used by this view.
    #[ts(type = "number")]
    pub through_seq: u64,
    pub segments: Vec<MessageTimelineSegment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export, export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct MessageTimelineUpdate {
    /// Correlation identity of the canonical owning message projection.
    pub source_message_id: String,
    /// Tool consumers match this within the exact Turn, because live display
    /// message identities need not equal historical tool projection identities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub call_id: Option<String>,
    pub timeline: MessageTimeline,
}

#[cfg(test)]
mod reasoning_display_tests {
    use super::*;

    #[test]
    fn ordinary_narration_and_late_results_do_not_own_reasoning_boundaries() {
        assert_eq!(reasoning_display_event("reasoning_delta"), ReasoningDisplayEvent::Reasoning);
        assert_eq!(reasoning_display_event("output_text_delta"), ReasoningDisplayEvent::Narration);
        for kind in ["tool_completed", "host_tool_settled", "tool/result-recorded", "message/completed", "usage", "future_display_notice"] {
            assert_eq!(reasoning_display_event(kind), ReasoningDisplayEvent::Other, "{kind}");
        }
        for kind in ["tool_call_delta", "tool_started", "tool_call_completed"] {
            assert_eq!(reasoning_display_event(kind), ReasoningDisplayEvent::Invocation, "{kind}");
        }
        for kind in ["model_step_started", "execution_resumed", "turn/steer-accepted",
            "completion_delivered", "user_input_requested", "turn_waiting", "turn_paused", "turn_failed", "turn_completed"] {
            assert_eq!(reasoning_display_event(kind), ReasoningDisplayEvent::Boundary, "{kind}");
        }
    }
}
