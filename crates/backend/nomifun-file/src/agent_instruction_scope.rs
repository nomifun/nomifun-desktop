//! Bounded instruction-scope discovery through the Session filesystem owner.
//! Metadata only; neither shell parsing nor a filesystem snapshot/write lease.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

use crate::{AgentSessionWorkspaceBinding, FileService, WORKSPACE_READ_OPERATION};

// The permit stays with a blocking scan even if the caller stops waiting.
static SCOPE_READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstructionScopeRequest {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Serialize)]
pub struct AgentInstructionScope {
    pub path: String,
    pub canonical_path: String,
    pub kind: &'static str,
    pub recursive: bool,
    /// Ancestor loading is engine policy; these are directories to inspect.
    pub directories: BTreeSet<String>,
    pub complete: bool,
    pub incomplete_reasons: BTreeSet<String>,
    pub entries_scanned: usize,
    pub notice: &'static str,
}

impl FileService {
    pub async fn instruction_scope_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentInstructionScopeRequest,
    ) -> Result<AgentInstructionScope, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        validate_relative(&request.path)?;
        let target = scope.resolve_relative_path(&request.path)?;
        let root = scope.workspace_root().to_owned();
        let authority = scope.authority();
        let permit = SCOPE_READS
            .acquire()
            .await
            .map_err(|_| AppError::Conflict("Instruction scope reader is closed".into()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let root = crate::path_safety::validate_path_authority(&root.to_string_lossy(), &authority)?;
            let canonical = resolve_missing(&target, &authority)?;
            let (kind, directory) = match std::fs::metadata(&canonical) {
                Ok(metadata) if metadata.is_dir() => ("directory", canonical.clone()),
                Ok(metadata) if metadata.is_file() => ("file", canonical.parent().ok_or_else(invalid)?.to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
                    ("missing", canonical.parent().ok_or_else(invalid)?.to_owned()),
                _ => return Err(invalid()),
            };
            let mut result = AgentInstructionScope {
                path: request.path, canonical_path: relative(&root, &canonical)?, kind,
                recursive: request.recursive, directories: BTreeSet::from([relative(&root, &directory)?]),
                complete: true, incomplete_reasons: BTreeSet::new(), entries_scanned: 0,
                notice: "Metadata observation only, not a snapshot, permission grant or proof of shell access scope. Load ancestor AGENTS.override.md/AGENTS.md for each directory. Recursive discovery includes hidden/ignored directories, never follows descendant symlinks and reports them as incomplete. Use canonical paths and recheck after effects; external concurrent renames remain possible.",
            };
            if request.recursive && kind == "directory" {
                let started = Instant::now();
                let mut pending = vec![(directory, 0usize)];
                while let Some((directory, depth)) = pending.pop() {
                    if result.entries_scanned >= 20_000 || started.elapsed() >= Duration::from_secs(5) {
                        result.incomplete_reasons.insert("scan_budget".into()); break;
                    }
                    let resolved = crate::path_safety::validate_path_authority(&directory.to_string_lossy(), &authority)?;
                    if resolved != directory { return Err(AppError::Conflict("INSTRUCTION_SCOPE_CHANGED: directory identity changed".into())); }
                    let entries = match std::fs::read_dir(&directory) {
                        Ok(entries) => entries,
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
                        let file_type = entry.file_type().map_err(|_| invalid())?;
                        // Win32 junctions are reparse points too; canonical
                        // comparison below rejects aliases even if is_symlink
                        // does not identify a platform-specific directory link.
                        if file_type.is_symlink() {
                            result.incomplete_reasons.insert("symlink_entry".into()); continue;
                        }
                        let name = entry.file_name();
                        let instruction_name = name.to_str().is_some_and(|name| {
                            ["AGENTS.md", "AGENTS.override.md"].iter().any(|expected| {
                                name == *expected || (cfg!(windows) && name.eq_ignore_ascii_case(expected))
                            })
                        });
                        if instruction_name {
                            if !file_type.is_file() { result.incomplete_reasons.insert("invalid_instruction_file".into()); }
                            result.directories.insert(relative(&root, &directory)?);
                            if result.directories.len() > 64 {
                                result.incomplete_reasons.insert("instruction_directory_budget".into()); break;
                            }
                        }
                        if file_type.is_dir() {
                            if depth >= 32 {
                                result.incomplete_reasons.insert("depth_limit".into());
                            } else if pending.len() >= 2048 {
                                result.incomplete_reasons.insert("directory_budget".into());
                            } else {
                                pending.push((entry.path(), depth + 1));
                            }
                        }
                    }
                    if result.incomplete_reasons.contains("scan_budget")
                        || result.incomplete_reasons.contains("instruction_directory_budget") { break; }
                }
            }
            if resolve_missing(&target, &authority)? != canonical {
                return Err(AppError::Conflict("INSTRUCTION_SCOPE_CHANGED: target identity changed".into()));
            }
            result.complete = result.incomplete_reasons.is_empty();
            if !crate::agent_text_read::fits_text_result(&result, 0)? {
                return Err(AppError::BadRequest("Instruction scope result exceeds the bounded envelope; narrow the path".into()));
            }
            Ok(result)
        }).await.map_err(|error| AppError::Internal(format!("instruction discovery failed: {error}")))?
    }
}

fn invalid() -> AppError {
    AppError::BadRequest("Invalid or unreadable workspace instruction scope".into())
}

fn validate_relative(path: &str) -> Result<(), AppError> {
    if path == "." {
        return Ok(());
    }
    if path.is_empty()
        || path.len() > 4096
        || path.trim() != path
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || crate::path_safety::is_unsafe_path_segment(part)
        })
    {
        return Err(invalid());
    }
    Ok(())
}

fn relative(root: &Path, path: &Path) -> Result<String, AppError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid())?
        .to_str()
        .ok_or_else(invalid)?;
    let relative = relative.replace('\\', "/");
    if relative.is_empty() {
        return Ok(String::new());
    }
    validate_relative(&relative)?;
    Ok(relative)
}

/// Missing child paths inherit the nearest existing, confined real parent.
/// A dangling symlink or denied parent is not absence and never gets adopted.
fn resolve_missing(path: &Path, authority: &crate::PathAuthority) -> Result<PathBuf, AppError> {
    let mut candidate = path;
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(candidate) {
            Ok(_) => {
                let mut real = crate::path_safety::validate_path_authority(
                    &candidate.to_string_lossy(),
                    authority,
                )?;
                if !suffix.is_empty() && !real.is_dir() {
                    return Err(invalid());
                }
                for part in suffix.into_iter().rev() {
                    real.push(part);
                }
                return Ok(real);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if suffix.len() >= 64 {
                    return Err(invalid());
                }
                suffix.push(candidate.file_name().ok_or_else(invalid)?.to_owned());
                candidate = candidate.parent().ok_or_else(invalid)?;
            }
            Err(_) => return Err(invalid()),
        }
    }
}
