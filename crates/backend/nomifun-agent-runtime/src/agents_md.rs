//! Bounded Workspace `AGENTS.md` discovery.
//!
//! The Nomi Runtime receives text only through a workspace owner port. It
//! never falls back to the host process current directory and never resolves
//! native paths itself.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::AgentEngineError;

const DEFAULT_MAX_FILE_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_DEPTH: usize = 32;

#[async_trait]
pub trait AgentWorkspaceReader: Send + Sync {
    async fn read_text(
        &self,
        workspace_relative_path: &str,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, AgentEngineError>;
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
    pub fn validate(self) -> Result<Self, AgentEngineError> {
        if self.max_file_bytes == 0
            || self.max_total_bytes == 0
            || self.max_depth == 0
            || self.max_file_bytes > self.max_total_bytes
            || self.max_depth > 256
        {
            return Err(AgentEngineError::WorkspaceContext(
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
    reader: &dyn AgentWorkspaceReader,
    working_directory: &str,
    policy: AgentsMdPolicy,
    cancellation: CancellationToken,
) -> Result<AgentsMdContext, AgentEngineError> {
    let policy = policy.validate()?;
    let working_directory = normalize_native_instruction_path(working_directory)?;
    let candidates = agents_candidates(&working_directory, policy.max_depth)?;
    let mut context = AgentsMdContext::default();

    for path in candidates {
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        // Only replace the leaf; a directory may itself be named AGENTS.md.
        let override_path = format!(
            "{}AGENTS.override.md",
            path.strip_suffix("AGENTS.md").unwrap_or_default()
        );
        let (path, result) = match reader.read_text(&override_path, cancellation.clone()).await {
            // Presence determines precedence, not content. An intentionally
            // empty override suppresses the sibling AGENTS.md, not descendants.
            Ok(Some(content)) => (override_path, Ok(Some(content))),
            Ok(_) => (
                path.clone(),
                reader.read_text(&path, cancellation.clone()).await,
            ),
            Err(error @ (AgentEngineError::Cancelled | AgentEngineError::EventSink(_))) => {
                return Err(error);
            }
            Err(error) => {
                context
                    .warnings
                    .push(format!("could not read {override_path}: {error}"));
                (
                    path.clone(),
                    reader.read_text(&path, cancellation.clone()).await,
                )
            }
        };
        let content = match result {
            Ok(content) => content,
            Err(error @ (AgentEngineError::Cancelled | AgentEngineError::EventSink(_))) => {
                return Err(error);
            }
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
        if content.trim().is_empty() {
            continue;
        }

        let mut truncated = false;
        if content.len() > policy.max_file_bytes {
            truncate_utf8(&mut content, policy.max_file_bytes);
            truncated = true;
            context
                .warnings
                .push(format!("{path} exceeded the per-file context limit"));
        }

        let header = format!("Instructions from {}:\n", serde_json::to_string(&path).expect("path serializes"));
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

/// Validate native path spelling without reading the host filesystem. Actual
/// canonical identity belongs to the Session file owner, including links and
/// parent components. Runtime must not impose a workspace containment policy.
pub(crate) fn normalize_native_instruction_path(value: &str) -> Result<String, AgentEngineError> {
    if value.len() > 4096 || value.contains('\0') {
        return Err(AgentEngineError::WorkspaceContext("native instruction path is invalid or exceeds 4096 bytes".into()));
    }
    if value.is_empty() || value == "." { return Ok(String::new()); }
    let path = std::path::Path::new(value);
    #[cfg(windows)]
    if (path.has_root() || matches!(path.components().next(), Some(std::path::Component::Prefix(_)))) && !path.is_absolute() {
        return Err(AgentEngineError::WorkspaceContext("use an absolute drive/UNC or Session-relative instruction path".into()));
    }
    path.components().collect::<std::path::PathBuf>().to_str().map(str::to_owned)
        .ok_or_else(|| AgentEngineError::WorkspaceContext("instruction path is not UTF-8".into()))
}

pub(crate) fn parent_directory(path: &str) -> String {
    std::path::Path::new(path).parent().and_then(std::path::Path::to_str).unwrap_or("").to_owned()
}

/// Project-relative scopes are relative only to the Session default directory.
/// Absolute native scopes have their own ancestry and never inherit the
/// project's root instruction merely because its relative label is empty.
pub(crate) fn path_in_scope(path: &str, root: &str) -> bool {
    let path = std::path::Path::new(path);
    let root = std::path::Path::new(root);
    path.is_absolute() == root.is_absolute() && path.starts_with(root)
}

fn agents_candidates(working_directory: &str, max_depth: usize) -> Result<Vec<String>, AgentEngineError> {
    let directory = std::path::Path::new(working_directory);
    let mut ancestors = directory.ancestors().collect::<Vec<_>>();
    // Empty Session-relative scope is still the project default root.
    if ancestors.is_empty() { ancestors.push(std::path::Path::new("")); }
    if ancestors.len().saturating_sub(1) > max_depth {
        return Err(AgentEngineError::WorkspaceContext(format!("instruction path depth exceeds the AGENTS.md limit {max_depth}")));
    }
    ancestors.reverse();
    if directory.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
        ancestors.retain(|ancestor| !ancestor.as_os_str().is_empty());
    }
    ancestors.into_iter().map(|parent| parent.join("AGENTS.md").to_str().map(str::to_owned)
        .ok_or_else(|| AgentEngineError::WorkspaceContext("instruction path is not UTF-8".into()))).collect()
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

    fn native_path(path: &str) -> String {
        std::path::Path::new(path)
            .components()
            .collect::<std::path::PathBuf>()
            .to_str()
            .unwrap()
            .to_owned()
    }

    struct MemoryWorkspace {
        files: BTreeMap<String, String>,
        reads: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AgentWorkspaceReader for MemoryWorkspace {
        async fn read_text(
            &self,
            workspace_relative_path: &str,
            _cancellation: CancellationToken,
        ) -> Result<Option<String>, AgentEngineError> {
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
                (native_path("crates/AGENTS.md"), "crates".to_owned()),
                (native_path("crates/runtime/AGENTS.md"), "runtime".to_owned()),
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
                .map(|layer| layer.path.clone())
                .collect::<Vec<_>>(),
            vec![native_path("AGENTS.md"), native_path("crates/AGENTS.md"), native_path("crates/runtime/AGENTS.md")]
        );
        assert!(context.combined.find("root").unwrap() < context.combined.find("runtime").unwrap());
    }

    #[tokio::test]
    async fn native_parent_relative_instruction_paths_are_read_through_the_owner() {
        let workspace = MemoryWorkspace {
            files: BTreeMap::new(),
            reads: Mutex::new(Vec::new()),
        };
        let result = load_agents_md(
                &workspace,
                "../outside",
                AgentsMdPolicy::default(),
                CancellationToken::new(),
            )
            .await.unwrap();
        assert!(result.layers.is_empty());
        assert!(workspace.reads.lock().unwrap().iter().any(|path| path == &native_path("../outside/AGENTS.md")));
    }

    #[tokio::test]
    async fn absolute_native_scopes_load_their_own_ancestry_without_project_root_aliasing() {
        let absolute = if cfg!(windows) { r"C:\external\nested" } else { "/external/nested" };
        let candidates = agents_candidates(absolute, 32).unwrap();
        let leaf = std::path::Path::new(absolute).join("AGENTS.md").to_str().unwrap().to_owned();
        let workspace = MemoryWorkspace { files: BTreeMap::from([(leaf.clone(), "external rules".into()),
            ("AGENTS.md".into(), "unrelated project rules".into())]), reads: Mutex::new(Vec::new()) };
        let context = load_agents_md(&workspace, absolute, AgentsMdPolicy::default(), CancellationToken::new()).await.unwrap();
        assert_eq!(context.layers.len(), 1);
        assert_eq!(context.layers[0].path, leaf);
        assert!(!candidates.iter().any(|path| path == "AGENTS.md"));
        assert!(!path_in_scope(absolute, ""));
        assert!(path_in_scope(&leaf, absolute));
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
