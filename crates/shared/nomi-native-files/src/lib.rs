//! Read-only native acquisition shared by login-user owners and the native
//! administrator worker. No Session, authorization, database, network or GUI
//! state lives here; callers retain the existing execution and cancellation owner.
use std::{collections::BTreeSet, path::{Path, PathBuf}};
use serde::{Deserialize, Serialize};

mod source;
mod scope;
#[cfg(unix)]
mod unix_directory;
/// Native handle operations shared with the login-user file owner. They do
/// not confer authority; the handle always uses the caller's OS token.
#[cfg(windows)]
#[path = "windows_directory.rs"]
pub mod windows;
pub use source::{acquire_source_path, acquire_source_with_hooks, acquire_source_with_adapter};
pub use scope::{NativeDirectoryAccess, NativeDirectoryEntry, NativeEntryKind, acquire_instruction_scope, acquire_instruction_scope_with_access, acquire_instruction_scope_with_semantics, acquire_instruction_scope_with_semantics_and_access};

pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

/// Content observations follow the final link. Entry mutations address the
/// directory entry itself; descendant links are never part of that subtree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePathSemantics {
    #[default]
    Follow,
    Entry,
}
impl NativePathSemantics {
    pub fn is_follow(&self) -> bool { *self == Self::Follow }
}

#[derive(Debug, thiserror::Error)]
pub enum NativeFileError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Changed(String),
    #[error("{operation}: {source}")]
    Io { operation: String, #[source] source: std::io::Error },
    #[error("{0}")]
    Unavailable(String),
}
impl NativeFileError {
    pub fn io(operation: impl Into<String>, source: std::io::Error) -> Self { Self::Io { operation: operation.into(), source } }
    pub fn observation(&self) -> NativeFileFailure {
        let (code, kind, raw_os_error) = match self {
            Self::InvalidInput(_) => ("INVALID_PAYLOAD", None, None),
            Self::Changed(_) => ("FILE_CHANGED_DURING_READ", None, None),
            Self::Unavailable(_) => ("NATIVE_FILE_ACQUIRE_UNAVAILABLE", None, None),
            Self::Io { source, .. } => (
                match source.kind() {
                    std::io::ErrorKind::PermissionDenied => "SYSTEM_ACCESS_DENIED",
                    std::io::ErrorKind::NotFound => "RESOURCE_NOT_FOUND",
                    std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotADirectory
                        | std::io::ErrorKind::IsADirectory | std::io::ErrorKind::AlreadyExists => "INVALID_PAYLOAD",
                    _ => "NATIVE_FILE_READ_FAILED",
                }, Some(format!("{:?}", source.kind())), source.raw_os_error()),
        };
        NativeFileFailure { code: code.into(), message: self.to_string(), kind, raw_os_error }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFileFailure {
    pub code: String,
    pub message: String,
    pub kind: Option<String>,
    pub raw_os_error: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeReadRequest {
    Source { path: String, max_bytes: usize },
    InstructionScope { path: String, recursive: bool, #[serde(default, skip_serializing_if = "NativePathSemantics::is_follow")] path_semantics: NativePathSemantics },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeReadAcquisition {
    Source { source: Option<NativeSource> },
    InstructionScope { scope: NativeInstructionScope },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSource {
    pub canonical_path: String,
    #[serde(with = "source_bytes")]
    pub bytes: Vec<u8>,
    pub sha256: String,
}
mod source_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > crate::MAX_SOURCE_BYTES.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("native source exceeds its encoded byte limit"));
        }
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        if bytes.len() > crate::MAX_SOURCE_BYTES { return Err(serde::de::Error::custom("native source exceeds 8 MiB")); }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeInstructionScope {
    pub canonical_path: String,
    pub kind: String,
    pub recursive: bool,
    #[serde(default, skip_serializing_if = "NativePathSemantics::is_follow")]
    pub path_semantics: NativePathSemantics,
    pub directories: BTreeSet<String>,
    pub complete: bool,
    pub incomplete_reasons: BTreeSet<String>,
    pub entries_scanned: usize,
}

pub fn acquire(request: NativeReadRequest) -> Result<NativeReadAcquisition, NativeFileError> {
    match request {
        NativeReadRequest::Source { path, max_bytes } => acquire_source_path(Path::new(&path), max_bytes)
            .map(|source| NativeReadAcquisition::Source { source }),
        NativeReadRequest::InstructionScope { path, recursive, path_semantics } => acquire_instruction_scope_with_semantics(Path::new(&path), recursive, path_semantics)
            .map(|scope| NativeReadAcquisition::InstructionScope { scope }),
    }
}

pub(crate) fn validate_path(path: &Path) -> Result<(), NativeFileError> {
    let Some(path_text) = path.to_str() else { return Err(NativeFileError::InvalidInput("native read path is not UTF-8".into())); };
    if path_text.is_empty() || path_text.len() > 4096 || path_text.contains('\0') || !path.is_absolute() {
        return Err(NativeFileError::InvalidInput("native acquisition requires a bounded absolute path without NUL".into()));
    }
    #[cfg(windows)]
    windows::normalize_path(path).map_err(|error| NativeFileError::io("validating native Windows file path", error))?;
    Ok(())
}
pub(crate) fn path_string(path: &Path) -> Result<String, NativeFileError> {
    path.to_str().map(str::to_owned).ok_or_else(|| NativeFileError::InvalidInput("canonical native path is not UTF-8".into()))
}

/// Only genuinely missing components may be appended. An unreadable parent,
/// dangling link, non-directory ancestor or failed lookup is never absence.
pub(crate) fn resolve_missing(path: &Path, max_missing_depth: usize, canonicalize: impl Fn(&Path) -> Result<PathBuf, NativeFileError>) -> Result<(PathBuf, bool), NativeFileError> {
    let mut candidate = path;
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::symlink_metadata(candidate) {
            Ok(_) => {
                let mut real = canonicalize(candidate)?;
                if !suffix.is_empty() && !real.is_dir() {
                    return Err(NativeFileError::InvalidInput("native read ancestor is not a directory".into()));
                }
                let absent = !suffix.is_empty();
                for part in suffix.into_iter().rev() {
                    #[cfg(windows)]
                    { real = windows::join_component(&real, &part).map_err(|error| NativeFileError::io("joining native missing path", error))?; }
                    #[cfg(not(windows))]
                    real.push(part);
                }
                return Ok((real, absent));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if suffix.len() >= max_missing_depth { return Err(NativeFileError::InvalidInput("native missing path depth exceeds the acquisition budget".into())); }
                // A removed UNC share or unavailable drive has no existing
                // filesystem root from which absence can be proved. Preserve
                // that real lookup failure; the requested path was valid and
                // this is neither InvalidInput nor a successful Source None.
                let Some(name) = candidate.file_name() else {
                    return Err(NativeFileError::io("resolving native read root", error));
                };
                suffix.push(name.to_owned());
                let Some(parent) = candidate.parent() else {
                    return Err(NativeFileError::io("resolving native read ancestor", error));
                };
                candidate = parent;
            }
            Err(error) => return Err(NativeFileError::io("inspecting native read path", error)),
        }
    }
}
