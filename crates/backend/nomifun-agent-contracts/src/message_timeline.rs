//! Read-only references into canonical message bodies and tool calls.
//! These values are presentation metadata, never a second transcript or execution fact.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
