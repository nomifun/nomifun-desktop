//! Fresh, bounded workspace search. Cached UI inventories are not evidence
//! that a newly-created source file does not exist.
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

use crate::{AgentSessionWorkspaceBinding, FileService, WORKSPACE_READ_OPERATION};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTextSearchRequest {
    pub query: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct AgentTextSearchResult {
    pub query: String,
    pub matches: Vec<AgentTextMatch>,
    pub truncated: bool,
    pub incomplete_reasons: BTreeSet<String>,
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub source_bytes_read: usize,
    pub notice: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AgentTextMatch {
    pub path: String,
    pub line: usize,
    pub column_bytes: usize,
    /// Offset of the first match on this line, usable with read_file plus the
    /// returned whole-source sha256. No pretend full-file search cursor.
    pub byte_offset: usize,
    pub sha256: String,
    pub text: String,
    pub text_start_column_bytes: usize,
    pub truncated: bool,
}

const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

impl FileService {
    pub async fn search_text_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentTextSearchRequest,
    ) -> Result<AgentTextSearchResult, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        let limit = request.limit.unwrap_or(100);
        if request.query.trim().is_empty()
            || request.query.len() > 4096
            || request.query.chars().count() > 1024
            || request.query.contains(['\r', '\n'])
            || !(1..=200).contains(&limit)
            || request
                .path
                .as_ref()
                .is_some_and(|path| path.len() > 4096 || path.chars().any(char::is_control))
        {
            return Err(AppError::BadRequest("Search requires a nonempty single-line literal query (at most 1024 characters / 4096 bytes), a relative path and limit 1..200".into()));
        }
        // Directory search accepts the conventional workspace-root spelling.
        // Do not broaden the shared file/mutation resolver (e.g. delete '.').
        let relative = match request.path.as_deref() { None | Some(".") => "", Some(path) => path };
        let target = scope.resolve_relative_path(relative)?;
        let authority = scope.authority();
        let root = scope.workspace_root().to_owned();
        tokio::task::spawn_blocking(move || {
            let root = crate::path_safety::validate_path_authority(&root.to_string_lossy(), &authority)?;
            let target = crate::path_safety::validate_path_authority(&target.to_string_lossy(), &authority)?;
            let mut result = AgentTextSearchResult {
                query: request.query, matches: Vec::new(), truncated: false,
                incomplete_reasons: BTreeSet::new(), files_scanned: 0, files_skipped: 0,
                source_bytes_read: 0,
                notice: "Fresh bounded literal UTF-8 search; hidden/ignore rules apply to directory walks, symlink entries are skipped. Not a filesystem snapshot. Empty matches are not proof of workspace-wide absence. Narrow path/query if incomplete; use byte_offset and sha256 with read_file for surrounding source.",
            };
            let started = Instant::now();
            let mut attempted_files = 0usize;
            let filter_root = root.clone();
            let mut builder = ignore::WalkBuilder::new(target);
            builder
                .follow_links(false)
                .parents(false)
                .git_global(false)
                .git_exclude(false)
                .max_depth(Some(64))
                .filter_entry(move |entry| {
                    entry
                        .path()
                        .strip_prefix(&filter_root)
                        .ok()
                        .and_then(|relative| relative.components().next())
                        .is_none_or(|component| {
                            !crate::artifact_store::is_workspace_owner_component(
                                component.as_os_str(),
                            )
                        })
                });
            let walker = builder.build();
            for (entry_index, entry) in walker.enumerate() {
                if entry_index >= 20_000 || attempted_files >= 2048 || started.elapsed() >= Duration::from_secs(5) {
                    result.incomplete_reasons.insert("scan_budget".into());
                    break;
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => { result.files_skipped += 1; result.incomplete_reasons.insert("unreadable_entry".into()); continue; }
                };
                if entry.depth() >= 64 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
                    result.incomplete_reasons.insert("depth_limit".into());
                }
                if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
                    result.files_skipped += 1;
                    result.incomplete_reasons.insert("symlink_entry".into());
                    continue;
                }
                if !entry.file_type().is_some_and(|kind| kind.is_file()) { continue; }
                attempted_files += 1;
                let remaining = MAX_SOURCE_BYTES.saturating_sub(result.source_bytes_read);
                if remaining <= 1 { result.incomplete_reasons.insert("source_byte_budget".into()); break; }
                let Some(path) = entry.path().strip_prefix(&root).ok().and_then(|path| path.to_str()) else {
                    result.files_skipped += 1; result.incomplete_reasons.insert("unrepresentable_path".into()); continue;
                };
                let path = path.replace('\\', "/");
                if path.len() > 4096 || path.chars().any(char::is_control) {
                    result.files_skipped += 1; result.incomplete_reasons.insert("unrepresentable_path".into()); continue;
                }
                let source = crate::agent_text_read::read_source(entry.path(), &authority,
                    MAX_FILE_BYTES.min(remaining - 1), &mut result.source_bytes_read);
                let (text, sha256) = match source {
                    Ok(Some(source)) => source,
                    // Never turn a failed, binary, oversized or disappeared
                    // file into a claim that it contained no matches.
                    Ok(None) | Err(_) => { result.files_skipped += 1; result.incomplete_reasons.insert("unreadable_changed_binary_or_oversized_file".into()); continue; }
                };
                result.files_scanned += 1;
                for (line_index, (byte_offset, source_line)) in crate::agent_patch_lines::logical_lines(&text).enumerate() {
                    let line = source_line.text;
                    if let Some(at) = line.find(&result.query) {
                        if result.matches.len() >= limit {
                            result.incomplete_reasons.insert("match_limit".into());
                            break;
                        }
                        // Include the actual match even when it occurs far
                        // beyond the beginning of a generated/minified line.
                        let mut start = at.saturating_sub(256);
                        while !line.is_char_boundary(start) { start += 1; }
                        let mut end = (at + result.query.len() + 256).min(line.len());
                        while !line.is_char_boundary(end) { end -= 1; }
                        result.matches.push(AgentTextMatch {
                            path: path.clone(), line: line_index + 1, column_bytes: at + 1,
                            byte_offset: byte_offset + at, sha256: sha256.clone(),
                            text: line[start..end].to_owned(), text_start_column_bytes: start + 1,
                            truncated: start > 0 || end < line.len(),
                        });
                        // Keep room for final counters/reasons; bound JSON
                        // escaping as well as visible result text.
                        if !crate::agent_text_read::fits_text_result(&result, 1024)? {
                            result.matches.pop();
                            result.incomplete_reasons.insert("result_byte_budget".into());
                            break;
                        }
                    }
                }
                if result.incomplete_reasons.contains("match_limit") || result.incomplete_reasons.contains("result_byte_budget") { break; }
            }
            result.truncated = !result.incomplete_reasons.is_empty();
            if !crate::agent_text_read::fits_text_result(&result, 0)? {
                return Err(AppError::Internal("Search result envelope exceeded".into()));
            }
            Ok(result)
        }).await.map_err(|e| AppError::Internal(format!("workspace search task failed: {e}")))?
    }
}
