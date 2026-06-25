use nomifun_common::FileChangeOperation;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// contentUpdate operation (distinct from snapshot FileChangeOperation)
// ---------------------------------------------------------------------------

/// Operation type for `fileStream.contentUpdate` events.
///
/// API Spec mandates exactly two values: `write` and `delete`.
/// This is intentionally separate from [`FileChangeOperation`] which tracks
/// git-style changes (Create/Modify/Delete) in the snapshot system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentUpdateOperation {
    Write,
    Delete,
}

// ---------------------------------------------------------------------------
// File tree / directory browsing
// ---------------------------------------------------------------------------

/// A node in the directory tree (file or directory with optional children).
///
/// Used internally by `IFileService::get_files_by_dir`. Converted to
/// `DirOrFileResponse` at the API boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct DirOrFile {
    pub name: String,
    pub full_path: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub children: Vec<DirOrFile>,
}

/// A flat file entry in a workspace listing.
///
/// Used by `IFileService::list_workspace_files`. No children — just path info.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceFlatFile {
    pub name: String,
    pub full_path: String,
    pub relative_path: String,
}

// ---------------------------------------------------------------------------
// File metadata
// ---------------------------------------------------------------------------

/// Metadata for a single file or directory.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mime_type: String,
    pub last_modified: i64,
    pub is_directory: bool,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Payload for the `fileStream.contentUpdate` WebSocket event.
///
/// Emitted after `write_file` (operation = Write) or `remove_entry`
/// (operation = Delete).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentUpdateEvent {
    pub file_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub workspace: String,
    pub relative_path: String,
    pub operation: ContentUpdateOperation,
}

/// Payload for the `fileWatch.fileChanged` WebSocket event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWatchEvent {
    pub file_path: String,
    pub event_type: String,
}

/// Payload for the `workspaceOfficeWatch.fileAdded` WebSocket event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeFileAddedEvent {
    pub file_path: String,
    pub workspace: String,
}

// ---------------------------------------------------------------------------
// Workspace snapshot
// ---------------------------------------------------------------------------

/// Snapshot mode for a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotMode {
    /// Directory already has a `.git` — use it directly.
    GitRepo,
    /// No `.git` — a temporary repo is created under `/tmp/nomifun-snapshot-*`.
    Snapshot,
    /// Snapshot tracking was refused for this workspace (e.g. drive/fs root,
    /// a well-known system directory, or too large to safely snapshot).
    /// `reason` is a human-readable explanation surfaced to the client.
    Disabled { reason: String },
}

/// Information about a workspace snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub mode: SnapshotMode,
    pub branch: Option<String>,
}

/// A single file change detected by the snapshot system.
#[derive(Debug, Clone, PartialEq)]
pub struct FileChangeInfo {
    pub file_path: String,
    pub relative_path: String,
    pub operation: FileChangeOperation,
}

/// Result of comparing workspace changes against the baseline.
#[derive(Debug, Clone)]
pub struct CompareResult {
    pub staged: Vec<FileChangeInfo>,
    pub unstaged: Vec<FileChangeInfo>,
}

// ---------------------------------------------------------------------------
// ZIP
// ---------------------------------------------------------------------------

/// A single entry to include in a ZIP archive.
#[derive(Debug, Clone)]
pub enum ZipEntry {
    /// In-memory text content.
    Text { name: String, content: String },
    /// Read from a file on disk.
    Disk { name: String, file_path: String },
}

/// Result of a batch copy operation.
#[derive(Debug, Clone)]
pub struct CopyResult {
    pub copied_files: Vec<String>,
    pub failed_files: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_update_event_serialization() {
        let event = ContentUpdateEvent {
            file_path: "/ws/src/main.rs".into(),
            content: Some("fn main() {}".into()),
            workspace: "/ws".into(),
            relative_path: "src/main.rs".into(),
            operation: ContentUpdateOperation::Write,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["file_path"], "/ws/src/main.rs");
        assert_eq!(json["content"], "fn main() {}");
        assert_eq!(json["workspace"], "/ws");
        assert_eq!(json["relative_path"], "src/main.rs");
        assert_eq!(json["operation"], "write");
    }

    #[test]
    fn content_update_event_delete_omits_content() {
        let event = ContentUpdateEvent {
            file_path: "/ws/old.txt".into(),
            content: None,
            workspace: "/ws".into(),
            relative_path: "old.txt".into(),
            operation: ContentUpdateOperation::Delete,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("content").is_none());
        assert_eq!(json["operation"], "delete");
    }

    #[test]
    fn file_watch_event_serialization() {
        let event = FileWatchEvent {
            file_path: "/path/to/file.txt".into(),
            event_type: "change".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["file_path"], "/path/to/file.txt");
        assert_eq!(json["event_type"], "change");
    }

    #[test]
    fn office_file_added_event_serialization() {
        let event = OfficeFileAddedEvent {
            file_path: "/ws/report.docx".into(),
            workspace: "/ws".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["file_path"], "/ws/report.docx");
        assert_eq!(json["workspace"], "/ws");
    }

    #[test]
    fn content_update_event_deserialization() {
        let raw = json!({
            "file_path": "/ws/a.txt",
            "content": "hello",
            "workspace": "/ws",
            "relative_path": "a.txt",
            "operation": "write"
        });
        let event: ContentUpdateEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(event.file_path, "/ws/a.txt");
        assert_eq!(event.content.as_deref(), Some("hello"));
        assert_eq!(event.operation, ContentUpdateOperation::Write);
    }

    #[test]
    fn snapshot_mode_equality() {
        assert_eq!(SnapshotMode::GitRepo, SnapshotMode::GitRepo);
        assert_ne!(SnapshotMode::GitRepo, SnapshotMode::Snapshot);
    }

    #[test]
    fn compare_result_empty() {
        let result = CompareResult {
            staged: vec![],
            unstaged: vec![],
        };
        assert!(result.staged.is_empty());
        assert!(result.unstaged.is_empty());
    }

    #[test]
    fn file_change_info_equality() {
        let a = FileChangeInfo {
            file_path: "/ws/a.txt".into(),
            relative_path: "a.txt".into(),
            operation: FileChangeOperation::Create,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn dir_or_file_with_children() {
        let dir = DirOrFile {
            name: "src".into(),
            full_path: "/project/src".into(),
            relative_path: "src".into(),
            is_dir: true,
            children: vec![DirOrFile {
                name: "main.rs".into(),
                full_path: "/project/src/main.rs".into(),
                relative_path: "src/main.rs".into(),
                is_dir: false,
                children: vec![],
            }],
        };
        assert!(dir.is_dir);
        assert_eq!(dir.children.len(), 1);
        assert!(!dir.children[0].is_dir);
        assert!(dir.children[0].children.is_empty());
    }
}
