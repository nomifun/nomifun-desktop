//! Read-only file presentation for an exact canonical Turn.
use nomifun_common::FileChangeOperation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnPublishedFile {
    pub artifact_id: String,
    pub path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnFileItem {
    /// Workspace identity and normalized source path; filenames are not identities.
    pub identity: String,
    pub relative_path: String,
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub operation: FileChangeOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Immutable delivery locator, separate from the mutable source locator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<TurnPublishedFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnFileSummary {
    /// Accepted request's UUID, shared by the conversation timeline.
    pub turn_id: String,
    pub complete: bool,
    pub files: Vec<TurnFileItem>,
}
