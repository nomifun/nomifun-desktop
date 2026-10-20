//! Ephemeral file targets for a single admitted invocation. These display-only
//! values contain no source text and are not persisted Session facts.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ToolFileOperation { Read, Write, Patch, Delete, Rename }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ToolFileSummaryKind { InstructionScope }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ToolFileEntry {
    /// Original requested path; preserve legal whitespace and native spelling.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub destination: Option<String>,
    /// Applied patch lines for this invocation, never a current workspace diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub added_lines: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub removed_lines: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ToolFileSummary {
    pub operation: ToolFileOperation,
    /// Directory instruction metadata does not claim to read every file below it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<ToolFileSummaryKind>,
    /// One target for simple actions; at most the native patch contract's 64.
    pub files: Vec<ToolFileEntry>,
}
