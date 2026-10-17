use std::{collections::BTreeSet, path::{Path, PathBuf}, time::{Duration, Instant}};
use crate::{NativeFileError, NativeInstructionScope};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeEntryKind { Directory, File, Link, Other }
#[derive(Debug)]
pub struct NativeDirectoryEntry { pub path: PathBuf, pub kind: NativeEntryKind }

/// Existing native owners may retain their platform-specific pinned directory
/// adapters. Traversal, scope rules and bounds remain one implementation here.
pub trait NativeDirectoryAccess {
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, NativeFileError>;
    fn metadata(&self, path: &Path) -> Result<Option<NativeEntryKind>, NativeFileError>;
    fn entries(&self, path: &Path) -> Result<Box<dyn Iterator<Item = Result<NativeDirectoryEntry, NativeFileError>>>, NativeFileError>;
}

pub fn acquire_instruction_scope(path: &Path, recursive: bool) -> Result<NativeInstructionScope, NativeFileError> {
    #[cfg(unix)]
    { acquire_instruction_scope_with_access(path, recursive, &crate::unix_directory::NativeUnixAccess, || {}, || {}, |_| {}, |_| {}) }
    #[cfg(not(unix))]
    { let _ = (path, recursive); Err(NativeFileError::Unavailable("native privileged instruction discovery needs this platform's directory adapter".into())) }
}

pub fn acquire_instruction_scope_with_access(
    path: &Path, recursive: bool, access: &impl NativeDirectoryAccess,
    before_metadata: impl FnOnce(), after_metadata: impl FnOnce(),
    mut before_listing: impl FnMut(&Path), mut after_listing: impl FnMut(&Path),
) -> Result<NativeInstructionScope, NativeFileError> {
    crate::validate_path(path)?;
    let (canonical, _) = crate::resolve_missing(path, 64, |path| access.canonicalize(path))?;
    before_metadata();
    let metadata = access.metadata(&canonical);
    after_metadata();
    let (kind, directory) = match metadata? {
        Some(NativeEntryKind::Link) => return Err(NativeFileError::Changed("INSTRUCTION_SCOPE_CHANGED: target became a symbolic link".into())),
        Some(NativeEntryKind::Directory) => ("directory", canonical.clone()),
        Some(NativeEntryKind::File) => ("file", canonical.parent().ok_or_else(invalid)?.to_owned()),
        None => ("missing", canonical.parent().ok_or_else(invalid)?.to_owned()),
        _ => return Err(invalid()),
    };
    let mut result = NativeInstructionScope {
        canonical_path: crate::path_string(&canonical)?, kind: kind.into(), recursive,
        directories: BTreeSet::from([crate::path_string(&directory)?]),
        complete: true, incomplete_reasons: BTreeSet::new(), entries_scanned: 0,
    };
    if recursive && kind == "directory" {
        let started = Instant::now();
        let mut pending = vec![(directory, 0usize)];
        while let Some((directory, depth)) = pending.pop() {
            if result.entries_scanned >= 20_000 || started.elapsed() >= Duration::from_secs(5) {
                result.incomplete_reasons.insert("scan_budget".into()); break;
            }
            if access.canonicalize(&directory)? != directory {
                return Err(NativeFileError::Changed("INSTRUCTION_SCOPE_CHANGED: directory identity changed".into()));
            }
            before_listing(&directory);
            let entries = match access.entries(&directory) {
                Ok(entries) => entries,
                Err(error @ NativeFileError::Changed(_)) => return Err(error),
                Err(_) => { result.incomplete_reasons.insert("unreadable_directory".into()); continue; }
            };
            for entry in entries {
                result.entries_scanned += 1;
                if result.entries_scanned > 20_000 || started.elapsed() >= Duration::from_secs(5) {
                    result.incomplete_reasons.insert("scan_budget".into()); break;
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => { result.incomplete_reasons.insert("unreadable_entry".into()); continue; }
                };
                if entry.kind == NativeEntryKind::Link { result.incomplete_reasons.insert("symlink_entry".into()); continue; }
                let instruction = entry.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                    ["AGENTS.md", "AGENTS.override.md"].iter().any(|expected| name == *expected || (cfg!(windows) && name.eq_ignore_ascii_case(expected)))
                });
                if instruction {
                    if entry.kind != NativeEntryKind::File { result.incomplete_reasons.insert("invalid_instruction_file".into()); }
                    result.directories.insert(crate::path_string(&directory)?);
                    if result.directories.len() > 64 { result.incomplete_reasons.insert("instruction_directory_budget".into()); break; }
                }
                if entry.kind == NativeEntryKind::Directory {
                    if depth >= 32 { result.incomplete_reasons.insert("depth_limit".into()); }
                    else if pending.len() >= 2048 { result.incomplete_reasons.insert("directory_budget".into()); }
                    else { pending.push((entry.path, depth + 1)); }
                }
            }
            after_listing(&directory);
            if result.incomplete_reasons.contains("scan_budget") || result.incomplete_reasons.contains("instruction_directory_budget") { break; }
        }
    }
    if crate::resolve_missing(path, 64, |path| access.canonicalize(path))?.0 != canonical {
        return Err(NativeFileError::Changed("INSTRUCTION_SCOPE_CHANGED: target identity changed".into()));
    }
    result.complete = result.incomplete_reasons.is_empty();
    Ok(result)
}
fn invalid() -> NativeFileError { NativeFileError::InvalidInput("invalid or unreadable native instruction scope".into()) }
