//! Workspace observations used only by file presentation, never by Runtime evidence.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::OperationId;

pub const TURN_WORKSPACE_FILES_OBSERVED: &str = "turn/workspace-files-observed";
pub const MAX_TURN_OBSERVED_FILES: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObservedFileOperation { Create, Modify, Delete }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedWorkspaceFile {
    pub path: String,
    pub operation: ObservedFileOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Captured start-side fingerprint; this is presentation data, not effect evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnWorkspaceFilesObservedV1 {
    pub operation_id: OperationId,
    pub execution_generation: u64,
    /// Exact root observed during this Turn; relocation is resolved on presentation reads.
    pub workspace_root: String,
    pub workspace_root_sha256: String,
    pub complete: bool,
    pub files: Vec<ObservedWorkspaceFile>,
    /// Owner-mutated paths whose included start/end fingerprints are equal.
    pub unchanged_paths: Vec<String>,
}

impl TurnWorkspaceFilesObservedV1 {
    pub fn validate(&self) -> Result<(), String> {
        if self.workspace_root.is_empty() || self.workspace_root.len() > 4096
            || self.workspace_root.chars().any(char::is_control)
            || !digest(&self.workspace_root_sha256) || self.files.len() > MAX_TURN_OBSERVED_FILES
            || self.unchanged_paths.len() > MAX_TURN_OBSERVED_FILES
            || crate::digest_bytes(self.workspace_root.as_bytes()).as_ref() != self.workspace_root_sha256 {
            return Err("invalid Turn workspace observation".into());
        }
        let mut paths = std::collections::BTreeSet::new();
        for file in &self.files {
            if !relative_path(&file.path)
                || !paths.insert(&file.path) || file.sha256.as_ref().is_some_and(|value| !digest(value))
                || file.source_sha256.as_ref().is_some_and(|value| !digest(value))
                || (file.operation == ObservedFileOperation::Create && file.source_sha256.is_some())
                || (file.operation == ObservedFileOperation::Delete && (file.size_bytes.is_some() || file.sha256.is_some())) {
                return Err("invalid observed workspace file".into());
            }
        }
        for path in &self.unchanged_paths {
            if !relative_path(path) || !paths.insert(path) {
                return Err("invalid unchanged workspace path".into());
            }
        }
        Ok(())
    }
}

fn relative_path(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.starts_with('/')
        && !value.contains('\\') && !value.chars().any(char::is_control)
        && value.as_bytes().get(1) != Some(&b':')
        && !value.split('/').any(|part| part.is_empty() || matches!(part, "." | "..")
            || part.eq_ignore_ascii_case(".nomifun") || part.eq_ignore_ascii_case(".git"))
}

fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
