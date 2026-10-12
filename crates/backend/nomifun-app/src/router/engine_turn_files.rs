//! Bounded file observations at the existing resource Turn boundaries.
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nomifun_agent_contracts::{ObservedFileOperation, ObservedWorkspaceFile, TurnWorkspaceFilesObservedV1, MAX_TURN_OBSERVED_FILES};
use nomifun_common::FileChangeOperation;
use nomifun_file::{WorkspaceFileSnapshot, TurnChangeLimits, capture_workspace_files, compare_workspace_files, unchanged_workspace_paths};
use tokio::sync::Semaphore;

use super::engine_journal::EngineTurnJournal;

/// Observation state is ephemeral; recovery does not reconstruct a historical baseline.
pub(super) struct TurnFileCapture {
    root: String,
    before: Option<WorkspaceFileSnapshot>,
    journal: EngineTurnJournal,
    recovered: bool,
}

struct CancelScan(Arc<AtomicBool>);
impl Drop for CancelScan {
    fn drop(&mut self) { self.0.store(true, Ordering::Release); }
}

async fn capture(root: String) -> Option<WorkspaceFileSnapshot> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    // Busy observers degrade their own coverage rather than queueing a Turn.
    let permit = SLOTS.get_or_init(|| Arc::new(Semaphore::new(2))).clone().try_acquire_owned().ok()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelScan(cancelled.clone());
    let limits = TurnChangeLimits {
        max_duration: Duration::from_millis(750),
        cancellation: Some(cancelled),
        ..TurnChangeLimits::default()
    };
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        capture_workspace_files(&PathBuf::from(root), limits)
    });
    // The collector cooperatively stops traversal and hashing. This timeout
    // also bounds waits for a filesystem syscall on an unresponsive mount.
    tokio::time::timeout(Duration::from_millis(850), task).await.ok()?.ok()
}

impl TurnFileCapture {
    pub(super) async fn begin(root: &str, journal: EngineTurnJournal) -> Self {
        let recovered = journal.recovered();
        // Resume observes its new segment; it cannot recover missing changes
        // between the original baseline and this fresh capture.
        let before = capture(root.to_owned()).await;
        Self { root: root.to_owned(), before, journal, recovered }
    }

    pub(super) async fn finish(self) {
        let Some(after) = capture(self.root.clone()).await else { return; };
        let root_sha256 = after.root_sha256.clone();
        let mut complete = false;
        let mut files = Vec::new();
        let mut unchanged_paths = Vec::new();
        if let Some(before) = self.before {
            let changes = compare_workspace_files(&before, &after);
            complete = changes.complete && !self.recovered;
            let paths = self.journal.workspace_mutation_paths(&root_sha256).await;
            match paths {
                Ok(paths) => unchanged_paths = unchanged_workspace_paths(&before, &after, &paths),
                Err(error) => {
                    complete = false;
                    tracing::warn!(%error, "file presentation could not resolve settled mutation paths");
                }
            }
            files = changes.files.into_iter().map(|file| ObservedWorkspaceFile {
                source_sha256: before.file_sha256(&file.path).map(str::to_owned),
                path: file.path,
                operation: match file.operation {
                    FileChangeOperation::Create => ObservedFileOperation::Create,
                    FileChangeOperation::Modify => ObservedFileOperation::Modify,
                    FileChangeOperation::Delete => ObservedFileOperation::Delete,
                },
                size_bytes: file.size_bytes,
                sha256: file.sha256,
            }).collect();
        }
        if files.len() > MAX_TURN_OBSERVED_FILES || unchanged_paths.len() > MAX_TURN_OBSERVED_FILES {
            complete = false;
            files.truncate(MAX_TURN_OBSERVED_FILES);
            unchanged_paths.truncate(MAX_TURN_OBSERVED_FILES);
        }
        let mut observation = TurnWorkspaceFilesObservedV1 {
            operation_id: self.journal.turn_operation().clone(),
            execution_generation: self.journal.generation(),
            workspace_root: after.root_path().to_string_lossy().into_owned(),
            workspace_root_sha256: root_sha256,
            complete, files, unchanged_paths,
        };
        if let Err(error) = bound_observation(&mut observation) {
            tracing::warn!(%error, "Turn file presentation could not be encoded");
            return;
        }
        if let Err(error) = self.journal.observe_workspace_files(observation).await {
            // This observation grants no completion or cleanup authority.
            tracing::warn!(%error, "Turn file presentation observation was not committed");
        }
    }
}

/// Keep display metadata inline so it cannot consume the native Session's
/// stored-Payload budget. Truncation is visible through the coverage flag.
fn bound_observation(observation: &mut TurnWorkspaceFilesObservedV1) -> Result<(), serde_json::Error> {
    let mut bytes = serde_json::to_vec(observation)?.len();
    let limit = nomifun_agent_session::MAX_INLINE_JSON_BYTES;
    if bytes <= limit { return Ok(()); }
    if observation.complete { bytes += 1; observation.complete = false; }
    while bytes > limit {
        let file_bytes = observation.files.last().map(serde_json::to_vec).transpose()?.map(|value| value.len());
        let path_bytes = observation.unchanged_paths.last().map(serde_json::to_vec).transpose()?.map(|value| value.len());
        match (file_bytes, path_bytes) {
            (Some(size), path) if path.is_none_or(|other| size >= other) => {
                bytes -= size + usize::from(observation.files.len() > 1);
                observation.files.pop();
            }
            (_, Some(size)) => {
                bytes -= size + usize::from(observation.unchanged_paths.len() > 1);
                observation.unchanged_paths.pop();
            }
            _ => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_observation_has_an_inline_byte_budget_and_truthful_coverage() {
        let root = "/workspace".to_owned();
        let mut observation = TurnWorkspaceFilesObservedV1 {
            operation_id: "file-byte-budget".into(), execution_generation: 1,
            workspace_root_sha256: nomifun_agent_contracts::digest_bytes(root.as_bytes()).as_ref().into(),
            workspace_root: root, complete: true,
            files: (0..500).map(|index| ObservedWorkspaceFile {
                path: format!("{index}-{}.docx", "文".repeat(100)), operation: ObservedFileOperation::Create,
                size_bytes: Some(1024), sha256: Some("a".repeat(64)), source_sha256: None,
            }).collect(), unchanged_paths: vec![],
        };
        bound_observation(&mut observation).unwrap();
        observation.validate().unwrap();
        assert!(!observation.complete);
        assert!(!observation.files.is_empty());
        assert!(observation.files.len() < 500);
        assert!(serde_json::to_vec(&observation).unwrap().len() <= nomifun_agent_session::MAX_INLINE_JSON_BYTES);
    }
}
