//! File system operations: read/write, path safety, file watching, snapshots, and zip.
pub mod browse;
mod artifact_store;
mod agent_instruction_scope;
mod agent_patch_lines;
mod agent_patch_outcome;
mod agent_patch_source;
pub use agent_patch_source::AgentSessionPatchSource;
pub use artifact_store::{
    ARTIFACT_RELATIVE_ROOT, MAX_ARTIFACT_READ_BYTES, PublishedWorkspaceArtifact,
    WORKSPACE_OWNER_DIRECTORY, WorkspaceArtifactRead, WorkspaceArtifactStore,
    artifact_publication_outcome_unknown, is_workspace_owner_component,
};
pub use agent_patch_outcome::{AgentPatchFailureObservation, AgentSessionPatchFailure};
pub use agent_instruction_scope::{AgentInstructionScope, AgentInstructionScopeRequest};
mod agent_text_read;
pub use agent_text_read::{AgentTextReadRequest, AgentTextPage};
mod agent_text_search;
pub use agent_text_search::{AgentTextMatch, AgentTextSearchRequest, AgentTextSearchResult};
pub mod path_safety;
pub mod resource;
pub mod routes;
pub mod service;
pub mod snapshot_service;
pub mod traits;
pub mod types;
pub mod watch_service;
pub mod workspace_listing;
mod vcs_stage;

pub use path_safety::{PathAuthority, has_traversal, validate_path, validate_path_for_write};
pub use resource::{
    AgentSessionWorkspaceBinding, DELETE_OPERATION as WORKSPACE_DELETE_OPERATION,
    READ_OPERATION as WORKSPACE_READ_OPERATION, WRITE_OPERATION as WORKSPACE_WRITE_OPERATION,
    WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER, workspace_binding,
};
pub use routes::{FileRouterState, file_routes};
pub use service::{
    AgentSessionFilePatch, AgentSessionPatchHunk, AgentSessionPatchLine,
    AgentSessionPatchRequest, AgentSessionPatchResult, AgentSessionPatchFileResult,
    FileService,
};
pub use snapshot_service::SnapshotService;
pub use traits::{
    FileServiceRef, FileWatchServiceRef, IFileService, IFileWatchService, ISnapshotService, SnapshotServiceRef,
};
pub use types::{
    CompareResult, ContentUpdateEvent, ContentUpdateOperation, CopyResult, DirOrFile, FileChangeInfo, FileMetadata,
    FileWatchEvent, OfficeFileAddedEvent, SnapshotInfo, SnapshotMode, WorkspaceFlatFile, ZipEntry,
};
pub use watch_service::FileWatchService;
pub use workspace_listing::{MAX_DIR_DEPTH, list_workspace_level};
pub use vcs_stage::{WorkspaceVcsStageOwner, vcs_stage_outcome_unknown};
