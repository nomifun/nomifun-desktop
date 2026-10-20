//! Ephemeral, read-only views of one canonical tool invocation. These DTOs are
//! not Session facts, runtime inputs, process controls, or persisted messages.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ToolDetailSection { Arguments, Output }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ToolDetailAvailability { Available, Pending, Unconfirmed }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ToolDetailFormat { Json, Text }

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDetailQuery {
    /// The accepted input UUID used by conversation messages, not an operation ID.
    pub turn_id: String,
    pub call_id: String,
    pub section: ToolDetailSection,
    pub through_seq: Option<u64>,
    pub expected_result_seq: Option<u64>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ToolDetailPage {
    pub turn_id: String,
    pub call_id: String,
    pub call_event_id: String,
    #[ts(type = "number")]
    pub through_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub result_seq: Option<u64>,
    pub section: ToolDetailSection,
    /// Output absence is independent of the tool's execution status. Available
    /// with zero bytes means a committed empty output, not a missing result.
    pub availability: ToolDetailAvailability,
    pub content: String,
    pub format: ToolDetailFormat,
    /// UTF-8 byte offsets in this immutable presentation section.
    #[ts(type = "number")]
    pub offset: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_cursor: Option<String>,
    pub eof: bool,
    /// Media stays in the existing verified artifact paths, never inline base64.
    pub non_text_parts: u32,
    /// Original native observation bounds, not a claim of complete process
    /// history. Present only for an exact admitted native process Action.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retention: Option<ToolDetailRetention>,
    /// Original explicitly supplied cwd, never the current workspace or disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ToolDetailRetention {
    #[ts(type = "number")]
    pub next_cursor: u64,
    #[ts(type = "number")]
    pub retained_bytes: u64,
    #[ts(type = "number")]
    pub dropped_bytes: u64,
    pub source_encoding: String,
    #[ts(type = "number")]
    pub decode_errors: u64,
}
