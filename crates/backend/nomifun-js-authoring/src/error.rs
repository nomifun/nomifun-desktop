use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthoringError {
    #[error("source store limits are invalid")]
    InvalidLimits,
    #[error("invalid {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("invalid SHA-256 digest: {value}")]
    InvalidDigest { value: String },
    #[error("managed source path is unsafe: {path}")]
    UnsafeManagedPath { path: PathBuf },
    #[error("unsafe source path {path}: {reason}")]
    UnsafeSourcePath { path: String, reason: String },
    #[error("source path is forbidden by the fixed build profile: {path} ({reason})")]
    ForbiddenSourceEntry { path: String, reason: String },
    #[error("duplicate or Windows case-colliding source path: {path}")]
    PathCollision { path: String },
    #[error("source project already exists")]
    ProjectAlreadyExists,
    #[error("source project was not found")]
    ProjectNotFound,
    #[error("source project ownership metadata does not match the requested scope")]
    ScopeMismatch,
    #[error("source project metadata is invalid: {0}")]
    InvalidScopeRecord(String),
    #[error("source snapshot changed (expected {expected}, observed {observed})")]
    SourceChanged { expected: String, observed: String },
    #[error("source operation was canceled")]
    Canceled,
    #[error("source contains too many files ({observed} > {limit})")]
    TooManyFiles { observed: usize, limit: usize },
    #[error("source file is too large: {path} ({observed} > {limit})")]
    FileTooLarge {
        path: String,
        observed: u64,
        limit: u64,
    },
    #[error("source tree exceeds the total byte limit ({observed} > {limit})")]
    TotalSizeExceeded { observed: u64, limit: u64 },
    #[error("package.json is invalid or unsupported: {0}")]
    InvalidPackageJson(String),
    #[error("dependency contract is invalid: {0}")]
    InvalidDependency(String),
    #[error("exact dependency lock is invalid: {0}")]
    InvalidDependencyLock(String),
    #[error("canonical serialization failed: {0}")]
    CanonicalSerialization(String),
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: io::Error) -> AuthoringError {
    AuthoringError::Io {
        path: path.into(),
        source,
    }
}
