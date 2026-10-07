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

pub(crate) const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

impl FileService {
    pub async fn search_text_for_agent_session(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentTextSearchRequest,
    ) -> Result<AgentTextSearchResult, AppError> {
        self.search_text_with_hooks(scope, request, || {}, || {}, |_| {}).await
    }

    async fn search_text_with_hooks(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentTextSearchRequest,
        before_walk: impl FnOnce() + Send + 'static,
        after_walk: impl FnOnce() + Send + 'static,
        before_directory: impl FnMut(&std::path::Path) + Send + 'static,
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
                notice: "Fresh bounded literal UTF-8 search; hidden/ignore rules apply to directory walks, symlink entries are skipped. Rule reads share the source-byte budget; incomplete rule verification stops that subtree. Not a filesystem snapshot. Empty matches are not proof of workspace-wide absence. Narrow path/query if incomplete; use byte_offset and sha256 with read_file for surrounding source.",
            };
            let started = Instant::now();
            let mut attempted_files = 0usize;
            before_walk();
            let mut walker = crate::workspace_search_walk::SearchWalk::new(&root, &target, authority.clone(), started, &mut result.incomplete_reasons, before_directory)?;
            while let Some(entry) = walker.next(&mut result.source_bytes_read, &mut result.incomplete_reasons) {
                if attempted_files >= 2048 || started.elapsed() >= Duration::from_secs(5) {
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
            after_walk();
            result.truncated = !result.incomplete_reasons.is_empty();
            if !crate::agent_text_read::fits_text_result(&result, 0)? {
                return Err(AppError::Internal("Search result envelope exceeded".into()));
            }
            Ok(result)
        }).await.map_err(|e| AppError::Internal(format!("workspace search task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, sync::{Arc, Mutex, Once}};

    struct Events;
    impl nomifun_realtime::UserEventSink for Events {
        fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }

    static IGNORE_OPENS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    struct IgnoreReadAudit;
    impl log::Log for IgnoreReadAudit {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool { metadata.target() == "ignore::gitignore" }
        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) { return; }
            let message = record.args().to_string();
            if let Some(path) = message.strip_prefix("opened gitignore file: ")
                && path.contains("search-guard-")
                && let Ok(canonical) = fs::canonicalize(path)
            { IGNORE_OPENS.lock().unwrap().push(canonical); }
        }
        fn flush(&self) {}
    }

    fn fixture() -> tempfile::TempDir {
        static START: Once = Once::new();
        static AUDIT: IgnoreReadAudit = IgnoreReadAudit;
        START.call_once(|| { log::set_logger(&AUDIT).unwrap(); log::set_max_level(log::LevelFilter::Debug); });
        let mut builder = tempfile::Builder::new();
        builder.prefix("search-guard-").disable_cleanup(std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT").is_some());
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    fn scope(root: &std::path::Path) -> AgentSessionWorkspaceBinding {
        crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], root).unwrap()
    }

    fn query(path: Option<&str>) -> AgentTextSearchRequest {
        AgentTextSearchRequest { query: "needle".into(), path: path.map(str::to_owned), limit: Some(100) }
    }

    fn assert_paths(fixture: &std::path::Path, result: &AgentTextSearchResult, expected: &[&str]) {
        let paths = result.matches.iter().map(|item| item.path.as_str()).collect::<BTreeSet<_>>();
        let expected = expected.iter().copied().collect::<BTreeSet<_>>();
        if paths != expected || result.truncated {
            fs::write(fixture.join("observation.json"), serde_json::to_vec_pretty(result).unwrap()).unwrap();
        }
        assert_eq!(paths, expected);
        assert!(!result.truncated, "{:?}", result.incomplete_reasons);
    }

    #[tokio::test]
    async fn search_preserves_ignore_precedence_nested_repositories_and_explicit_files() {
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        for path in [".git", "nested", "repository/.git"] { fs::create_dir_all(root.join(path)).unwrap(); }
        fs::write(root.join(".gitignore"), "\u{feff}*.log\nnested/*.txt\n!.visible-hidden\n").unwrap();
        fs::write(root.join(".ignore"), b"priority.txt\n!override.log\n").unwrap();
        fs::write(root.join("nested/.gitignore"), b"!keep.txt\n!priority.txt\n").unwrap();
        fs::write(root.join("nested/.ignore"), b"!nested-only.txt\n").unwrap();
        fs::write(root.join("repository/.gitignore"), b"own.txt\n").unwrap();
        for path in ["keep.txt", "skip.log", "priority.txt", "override.log", ".hidden", ".visible-hidden",
            "nested/keep.txt", "nested/no.txt", "nested/priority.txt", "nested/nested-only.txt", "nested/code.rs",
            "repository/allow.log", "repository/own.txt", "repository/priority.txt"] {
            fs::write(root.join(path), b"needle").unwrap();
        }
        let service = FileService::new(Arc::new(Events), vec![]);
        let binding = scope(&root);
        let result = service.search_text_for_agent_session(&binding, query(None)).await.unwrap();
        assert_paths(fixture.path(), &result, &["keep.txt", "override.log", ".visible-hidden", "nested/keep.txt",
            "nested/nested-only.txt", "nested/code.rs", "repository/allow.log"]);
        let result = service.search_text_for_agent_session(&binding, query(Some("nested"))).await.unwrap();
        assert_paths(fixture.path(), &result, &["nested/keep.txt", "nested/no.txt", "nested/priority.txt", "nested/nested-only.txt", "nested/code.rs"]);
        for path in ["skip.log", ".hidden"] {
            let result = service.search_text_for_agent_session(&binding, query(Some(path))).await.unwrap();
            assert_paths(fixture.path(), &result, &[path]);
        }
        fs::remove_dir(root.join(".git")).unwrap();
        let result = service.search_text_for_agent_session(&binding, query(None)).await.unwrap();
        assert_paths(fixture.path(), &result, &["keep.txt", "skip.log", "override.log", "nested/keep.txt", "nested/no.txt",
            "nested/nested-only.txt", "nested/code.rs", "repository/allow.log"]);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn search_uses_case_aliased_rule_names_on_insensitive_directories() {
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".GIT")).unwrap();
        fs::write(root.join(".IGNORE"), b"skip.txt\n").unwrap();
        fs::write(root.join(".GITIGNORE"), b"skip.log\n").unwrap();
        for path in ["skip.txt", "skip.log", "keep.txt"] { fs::write(root.join(path), b"needle").unwrap(); }
        let result = FileService::new(Arc::new(Events), vec![]).search_text_for_agent_session(&scope(&root), query(None)).await.unwrap();
        let paths = result.matches.iter().map(|item| item.path.as_str()).collect::<Vec<_>>();
        if result.truncated || paths != ["keep.txt"] {
            fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
            panic!("case-aliased rules changed search behavior; retained fixture: {}", fixture.keep().display());
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn search_keeps_case_sensitive_rule_names_distinct() {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_WRITE_ATTRIBUTES, FileCaseSensitiveInfo, SetFileInformationByHandle};
        use windows_sys::Win32::System::SystemServices::FILE_CS_FLAG_CASE_SENSITIVE_DIR;
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let directory = fs::OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(&root).unwrap();
        let info = FILE_CASE_SENSITIVE_INFO { Flags: FILE_CS_FLAG_CASE_SENSITIVE_DIR };
        assert_ne!(unsafe { SetFileInformationByHandle(directory.as_raw_handle(), FileCaseSensitiveInfo,
            (&info as *const FILE_CASE_SENSITIVE_INFO).cast(), std::mem::size_of_val(&info) as u32) }, 0,
            "case-sensitive fixture: {}", std::io::Error::last_os_error());
        fs::create_dir(root.join(".GIT")).unwrap();
        fs::write(root.join(".IGNORE"), b"skip.txt\n").unwrap();
        fs::write(root.join(".GITIGNORE"), b"skip.log\n").unwrap();
        for path in ["skip.txt", "skip.log", "keep.txt"] { fs::write(root.join(path), b"needle").unwrap(); }
        let service = FileService::new(Arc::new(Events), vec![]);
        assert_paths(fixture.path(), &service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap(),
            &["skip.txt", "skip.log", "keep.txt"]);
        fs::write(root.join(".ignore"), b"skip.txt\n").unwrap();
        assert_paths(fixture.path(), &service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap(),
            &["skip.log", "keep.txt"]);
    }

    #[tokio::test]
    async fn search_preserves_source_offsets_digests_and_explicit_limits() {
        use sha2::{Digest, Sha256};
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let source = "\u{feff}第一行\r\nα needle tail\r\nneedle again\n";
        fs::write(root.join("source.txt"), source).unwrap();
        let service = FileService::new(Arc::new(Events), vec![]);
        let binding = scope(&root);
        let mut request = query(None);
        request.limit = Some(1);
        let result = service.search_text_for_agent_session(&binding, request).await.unwrap();
        assert!(result.truncated);
        assert_eq!(result.incomplete_reasons, BTreeSet::from(["match_limit".into()]));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].line, 2);
        assert_eq!(result.matches[0].column_bytes, 4);
        assert_eq!(result.matches[0].byte_offset, source.find("needle").unwrap());
        assert_eq!(result.matches[0].sha256, format!("{:x}", Sha256::digest(source.as_bytes())));
        assert_eq!(result.source_bytes_read, source.len());
        let mut request = query(None);
        request.query = "absent".into();
        let result = service.search_text_for_agent_session(&binding, request).await.unwrap();
        assert!(result.matches.is_empty());
        assert!(!result.truncated);
        assert_eq!(result.files_scanned, 1);
        let mut request = query(None);
        request.query = "two\nlines".into();
        assert!(service.search_text_for_agent_session(&binding, request).await.is_err());
    }

    #[tokio::test]
    async fn search_reports_unverified_rules_and_charges_rule_bytes() {
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("source.txt"), b"needle").unwrap();
        let service = FileService::new(Arc::new(Events), vec![]);
        for (rules, reason) in [("[z-a]\n".to_owned(), "invalid_ignore_pattern"), ("# bounded\n".repeat(4097), "ignore_rule_budget")] {
            fs::write(root.join(".ignore"), &rules).unwrap();
            let result = service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap();
            assert!(result.truncated);
            assert!(result.incomplete_reasons.contains(reason));
            assert!(result.matches.is_empty());
            assert_eq!(result.files_scanned, 0);
            assert_eq!(result.source_bytes_read, rules.len());
        }
        fs::write(root.join(".ignore"), b"skip.txt\n").unwrap();
        let result = service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap();
        assert_paths(fixture.path(), &result, &["source.txt"]);
        assert_eq!(result.source_bytes_read, b"skip.txt\n".len() + b"needle".len());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn search_skips_junctions_and_windows_hidden_attributes() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("visible.txt"), b"needle").unwrap();
        let hidden = root.join("hidden.txt");
        fs::write(&hidden, b"needle").unwrap();
        let wide: Vec<u16> = hidden.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) }, 0);
        fs::write(outside.join("outside.txt"), b"needle outside").unwrap();
        junction::create(&outside, root.join("escape")).unwrap();
        let service = FileService::new(Arc::new(Events), vec![]);
        let result = service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap();
        assert_eq!(result.matches.iter().map(|item| item.path.as_str()).collect::<Vec<_>>(), ["visible.txt"]);
        assert!(result.truncated);
        assert_eq!(result.incomplete_reasons, BTreeSet::from(["symlink_entry".into()]));
        assert_eq!(result.files_skipped, 1);
        assert_eq!(fs::read(outside.join("outside.txt")).unwrap(), b"needle outside");
        assert_paths(fixture.path(), &service.search_text_for_agent_session(&scope(&root), query(Some("hidden.txt"))).await.unwrap(), &["hidden.txt"]);
    }

    #[tokio::test]
    async fn search_never_opens_ignore_files_above_its_bound_root() {
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::create_dir(fixture.path().join(".git")).unwrap();
        let outside = fixture.path().join(".ignore");
        fs::write(&outside, b"outside-only-pattern\n").unwrap();
        fs::write(root.join("source.txt"), b"needle").unwrap();
        let outside = fs::canonicalize(outside).unwrap();
        ignore::gitignore::Gitignore::new(&outside); // Calibrate the open observer independently.
        assert!(IGNORE_OPENS.lock().unwrap().contains(&outside));
        let start = IGNORE_OPENS.lock().unwrap().len();
        let service = FileService::new(Arc::new(Events), vec![]);
        let result = service.search_text_for_agent_session(&scope(&root), query(None)).await.unwrap();
        let opened_outside = IGNORE_OPENS.lock().unwrap()[start..].contains(&outside);
        if opened_outside {
            fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "opened_outside_ignore": opened_outside, "result": result,
            })).unwrap()).unwrap();
            panic!("search opened an unbound ignore source; retained fixture: {}", fixture.keep().display());
        }
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].path, "source.txt");
        assert!(!result.truncated);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn search_rejects_rules_from_a_replaced_target_directory() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        let target = root.join("target");
        let retained = root.join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(target.join("source.txt"), b"needle").unwrap();
        fs::write(outside.join(".ignore"), b"*\n").unwrap();
        fs::write(outside.join("outside.txt"), b"outside").unwrap();
        let redirected = Arc::new(AtomicBool::new(false));
        let before = {
            let (target, retained, outside, redirected) = (target.clone(), retained.clone(), outside.clone(), redirected.clone());
            move || {
                fs::rename(&target, &retained).unwrap();
                junction::create(&outside, &target).unwrap();
                redirected.store(true, Ordering::SeqCst);
            }
        };
        let restore = {
            let (target, retained, redirected) = (target.clone(), retained.clone(), redirected.clone());
            move || {
                if redirected.swap(false, Ordering::SeqCst) {
                    junction::delete(&target).unwrap();
                    fs::rename(&retained, &target).unwrap();
                }
            }
        };
        let service = FileService::new(Arc::new(Events), vec![]);
        let result = service.search_text_with_hooks(&scope(&root), query(Some("target")), before, restore.clone(), |_| {}).await;
        restore();
        if let Ok(observed) = &result {
            // An explicit incomplete result also rejects a reliable absence
            // claim. The boundary is no outside data/rules and no false success.
            assert!(observed.truncated && observed.matches.is_empty());
            assert_eq!(observed.source_bytes_read, 0);
        }
        assert_eq!(fs::read(target.join("source.txt")).unwrap(), b"needle");
        assert_eq!(fs::read(outside.join("outside.txt")).unwrap(), b"outside");
        assert_eq!(service.search_text_for_agent_session(&scope(&root), query(Some("target"))).await.unwrap().matches.len(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn child_directory_replacement_cannot_supply_search_ignore_rules() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let fixture = fixture();
        let root = fixture.path().join("workspace");
        let target = root.join("target");
        let retained = fixture.path().join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(target.join("source.txt"), b"needle").unwrap();
        fs::write(outside.join(".ignore"), b"*\n").unwrap();
        fs::write(outside.join("outside.txt"), b"needle outside").unwrap();
        let canonical_target = fs::canonicalize(&target).unwrap();
        let outside_rule = fs::canonicalize(outside.join(".ignore")).unwrap();
        let redirected = Arc::new(AtomicBool::new(false));
        let before_directory = {
            let (target, retained, outside, redirected) = (target.clone(), retained.clone(), outside.clone(), redirected.clone());
            move |path: &std::path::Path| {
                if path == canonical_target && !redirected.swap(true, Ordering::SeqCst) {
                    fs::rename(&target, &retained).unwrap();
                    junction::create(&outside, &target).unwrap();
                }
            }
        };
        let restore = {
            let (target, retained, redirected) = (target.clone(), retained.clone(), redirected.clone());
            move || {
                if redirected.swap(false, Ordering::SeqCst) {
                    junction::delete(&target).unwrap();
                    fs::rename(&retained, &target).unwrap();
                }
            }
        };
        let start = IGNORE_OPENS.lock().unwrap().len();
        let service = FileService::new(Arc::new(Events), vec![]);
        let binding = scope(&root);
        let result = service.search_text_with_hooks(&binding, query(None), || {}, restore.clone(), before_directory).await.unwrap();
        let opened_outside = IGNORE_OPENS.lock().unwrap()[start..].contains(&outside_rule);
        restore();
        fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(&serde_json::json!({"opened_outside_ignore": opened_outside, "result": result})).unwrap()).unwrap();
        assert!(!opened_outside, "child rule read escaped; retained fixture: {}", fixture.keep().display());
        assert!(result.truncated);
        assert!(result.matches.is_empty());
        assert_eq!(result.source_bytes_read, 0);
        assert_eq!(fs::read(target.join("source.txt")).unwrap(), b"needle");
        assert_eq!(fs::read(outside.join("outside.txt")).unwrap(), b"needle outside");
        assert_eq!(service.search_text_for_agent_session(&binding, query(None)).await.unwrap().matches.len(), 1);
    }
}
