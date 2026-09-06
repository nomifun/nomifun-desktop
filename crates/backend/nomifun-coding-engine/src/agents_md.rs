//! Bounded Workspace `AGENTS.md` discovery.
//!
//! The Coding Engine receives text only through a workspace owner port. It
//! never falls back to the host process current directory and never resolves
//! native paths itself.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::CodingEngineError;

const DEFAULT_MAX_FILE_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_DEPTH: usize = 32;

#[async_trait]
pub trait CodingWorkspaceReader: Send + Sync {
    async fn read_text(
        &self,
        workspace_relative_path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, CodingEngineError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsMdPolicy {
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub max_depth: usize,
}

impl Default for AgentsMdPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

impl AgentsMdPolicy {
    pub fn validate(self) -> Result<Self, CodingEngineError> {
        if self.max_file_bytes == 0
            || self.max_total_bytes == 0
            || self.max_depth == 0
            || self.max_file_bytes > self.max_total_bytes
            || self.max_depth > 256
        {
            return Err(CodingEngineError::WorkspaceContext(
                "AGENTS.md policy limits are invalid".to_owned(),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsMdLayer {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsMdContext {
    pub layers: Vec<AgentsMdLayer>,
    pub combined: String,
    pub total_bytes: usize,
    pub warnings: Vec<String>,
}

pub async fn load_agents_md(
    reader: &dyn CodingWorkspaceReader,
    working_directory: &str,
    policy: AgentsMdPolicy,
    cancellation: CancellationToken,
) -> Result<AgentsMdContext, CodingEngineError> {
    let policy = policy.validate()?;
    let working_directory = normalize_workspace_directory(working_directory)?;
    let candidates = agents_candidates(&working_directory, policy.max_depth)?;
    let mut context = AgentsMdContext::default();

    for path in candidates {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let content = match reader.read_text(&path, cancellation.clone()).await {
            Ok(content) => content,
            Err(CodingEngineError::Cancelled) => return Err(CodingEngineError::Cancelled),
            Err(error) => {
                context
                    .warnings
                    .push(format!("could not read {path}: {error}"));
                continue;
            }
        };
        let Some(mut content) = content else {
            continue;
        };

        let mut truncated = false;
        if content.len() > policy.max_file_bytes {
            truncate_utf8(&mut content, policy.max_file_bytes);
            truncated = true;
            context
                .warnings
                .push(format!("{path} exceeded the per-file context limit"));
        }

        let header = format!("Instructions from {path}:\n");
        let separator_bytes = usize::from(!context.combined.is_empty()) * 2;
        let fixed_bytes = header.len().saturating_add(separator_bytes);
        let remaining = policy
            .max_total_bytes
            .saturating_sub(context.combined.len());
        if remaining <= fixed_bytes {
            context
                .warnings
                .push("AGENTS.md total context limit was reached".to_owned());
            break;
        }
        let available_content = remaining - fixed_bytes;
        if content.len() > available_content {
            truncate_utf8(&mut content, available_content);
            truncated = true;
            context
                .warnings
                .push(format!("{path} was truncated by the total context limit"));
        }
        if content.is_empty() {
            break;
        }

        if !context.combined.is_empty() {
            context.combined.push_str("\n\n");
        }
        context.combined.push_str(&header);
        context.combined.push_str(&content);
        context.layers.push(AgentsMdLayer {
            path,
            content,
            truncated,
        });
    }
    context.total_bytes = context.combined.len();
    Ok(context)
}

fn normalize_workspace_directory(value: &str) -> Result<String, CodingEngineError> {
    let value = value.trim();
    if value.is_empty() || value == "." {
        return Ok(String::new());
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.ends_with('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.contains(':')
    {
        return Err(CodingEngineError::WorkspaceContext(
            "working directory must be a normalized workspace-relative path".to_owned(),
        ));
    }
    let segments = value.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(CodingEngineError::WorkspaceContext(
            "working directory contains an invalid path segment".to_owned(),
        ));
    }
    Ok(segments.join("/"))
}

fn agents_candidates(
    working_directory: &str,
    max_depth: usize,
) -> Result<Vec<String>, CodingEngineError> {
    let segments = if working_directory.is_empty() {
        Vec::new()
    } else {
        working_directory.split('/').collect::<Vec<_>>()
    };
    if segments.len() > max_depth {
        return Err(CodingEngineError::WorkspaceContext(format!(
            "working directory depth {} exceeds the AGENTS.md limit {max_depth}",
            segments.len()
        )));
    }
    let mut candidates = vec!["AGENTS.md".to_owned()];
    let mut prefix = String::new();
    for segment in segments {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        candidates.push(format!("{prefix}/AGENTS.md"));
    }
    Ok(candidates)
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    struct MemoryWorkspace {
        files: BTreeMap<String, String>,
        reads: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CodingWorkspaceReader for MemoryWorkspace {
        async fn read_text(
            &self,
            workspace_relative_path: &str,
            _cancellation: CancellationToken,
        ) -> Result<Option<String>, CodingEngineError> {
            self.reads
                .lock()
                .unwrap()
                .push(workspace_relative_path.to_owned());
            Ok(self.files.get(workspace_relative_path).cloned())
        }
    }

    #[tokio::test]
    async fn loads_root_to_current_directory_instructions_in_order() {
        let workspace = MemoryWorkspace {
            files: BTreeMap::from([
                ("AGENTS.md".to_owned(), "root".to_owned()),
                ("crates/AGENTS.md".to_owned(), "crates".to_owned()),
                (
                    "crates/runtime/AGENTS.md".to_owned(),
                    "runtime".to_owned(),
                ),
            ]),
            reads: Mutex::new(Vec::new()),
        };

        let context = load_agents_md(
            &workspace,
            "crates/runtime",
            AgentsMdPolicy::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            context
                .layers
                .iter()
                .map(|layer| layer.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "AGENTS.md",
                "crates/AGENTS.md",
                "crates/runtime/AGENTS.md"
            ]
        );
        assert!(context.combined.find("root").unwrap() < context.combined.find("runtime").unwrap());
    }

    #[tokio::test]
    async fn rejects_workspace_escape_without_calling_the_owner() {
        let workspace = MemoryWorkspace {
            files: BTreeMap::new(),
            reads: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            load_agents_md(
                &workspace,
                "../outside",
                AgentsMdPolicy::default(),
                CancellationToken::new(),
            )
            .await,
            Err(CodingEngineError::WorkspaceContext(_))
        ));
        assert!(workspace.reads.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn total_limit_is_utf8_safe_and_reported() {
        let workspace = MemoryWorkspace {
            files: BTreeMap::from([("AGENTS.md".to_owned(), "界".repeat(128))]),
            reads: Mutex::new(Vec::new()),
        };
        let context = load_agents_md(
            &workspace,
            ".",
            AgentsMdPolicy {
                max_file_bytes: 128,
                max_total_bytes: 128,
                max_depth: 1,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(context.total_bytes <= 128);
        assert!(context.layers[0].truncated);
        assert!(!context.warnings.is_empty());
    }
}
