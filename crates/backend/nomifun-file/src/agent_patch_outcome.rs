//! Publication observations, not a multi-file transaction or filesystem lock.
use nomifun_common::AppError;
use serde::Serialize;

/// All indices are zero-based positions in the original request's `files`.
/// Index groups keep even the maximum 64-file failure small enough for the
/// shared engine's durable error budget, without leaking absolute paths/text.
#[derive(Debug, Default, Serialize)]
pub struct AgentPatchFailureObservation {
    pub failed_file: Option<usize>,
    pub published: Vec<usize>,
    pub restored: Vec<usize>,
    pub restore_published_unconfirmed: Vec<usize>,
    pub retained_created: Vec<usize>,
    pub skipped_changed_or_unreadable: Vec<usize>,
    pub rollback_failed: Vec<usize>,
    pub temporary_cleanup_unconfirmed: Vec<usize>,
}

#[derive(Debug)]
pub struct AgentSessionPatchFailure {
    pub error: AppError,
    pub observation: AgentPatchFailureObservation,
}

impl From<AppError> for AgentSessionPatchFailure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            observation: AgentPatchFailureObservation::default(),
        }
    }
}

/// Set immediately after rename/hard-link succeeds, before fallible cleanup.
#[derive(Debug)]
pub(crate) struct PatchPublicationFailure {
    pub error: AppError,
    pub published: bool,
    pub temporary_cleanup_unconfirmed: bool,
}

impl From<AppError> for PatchPublicationFailure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            published: false,
            temporary_cleanup_unconfirmed: false,
        }
    }
}
