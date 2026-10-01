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
    pub observation_kind: &'static str,
    pub is_directory_listing: bool,
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
        self.instruction_scope_with_hooks(scope, request, |_| {}, |_| {}).await
    }

    async fn instruction_scope_with_hooks(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        request: AgentInstructionScopeRequest,
        before_listing: impl FnMut(&Path) + Send + 'static,
        after_listing: impl FnMut(&Path) + Send + 'static,
    ) -> Result<AgentInstructionScope, AppError> {
        self.instruction_scope_with_all_hooks(scope, request, || {}, || {}, before_listing, after_listing).await
    }

    async fn instruction_scope_with_all_hooks(
        &self, scope: &AgentSessionWorkspaceBinding, request: AgentInstructionScopeRequest,
        before_metadata: impl FnOnce() + Send + 'static,
        after_metadata: impl FnOnce() + Send + 'static,
        mut before_listing: impl FnMut(&Path) + Send + 'static,
        mut after_listing: impl FnMut(&Path) + Send + 'static,
    ) -> Result<AgentInstructionScope, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        validate_relative(&request.path)?;
        // The model-facing scope protocol names the workspace root ".", while
        // ordinary file operations require a normalized relative path. Resolve
        // that one metadata-only spelling as the binding root without making
        // "." a valid write/delete target.
        let target = scope.resolve_relative_path(if request.path == "." { "" } else { &request.path })?;
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
            before_metadata();
            let metadata = crate::workspace_read_dir::metadata(&canonical, &authority);
            after_metadata();
            let (kind, directory) = match metadata? {
                Some(observed) if observed.metadata.file_type().is_symlink() =>
                    return Err(AppError::Conflict("INSTRUCTION_SCOPE_CHANGED: target became a symbolic link".into())),
                Some(observed) if observed.metadata.is_dir() => ("directory", canonical.clone()),
                Some(observed) if observed.metadata.is_file() => ("file", canonical.parent().ok_or_else(invalid)?.to_owned()),
                None => ("missing", canonical.parent().ok_or_else(invalid)?.to_owned()),
                _ => return Err(invalid()),
            };
            let mut result = AgentInstructionScope {
                observation_kind: "instruction_scope", is_directory_listing: false,
                path: request.path, canonical_path: relative(&root, &canonical)?, kind,
                recursive: request.recursive, directories: BTreeSet::from([relative(&root, &directory)?]),
                complete: true, incomplete_reasons: BTreeSet::new(), entries_scanned: 0,
                notice: "Instruction-location metadata, not a filesystem entry listing, snapshot, permission grant or proof of shell access scope. entries_scanned counts discovery traversal; nonrecursive metadata has zero scans even for a nonempty directory. complete describes instruction discovery, not directory contents. Load ancestor AGENTS.override.md/AGENTS.md for each directory. Recursive discovery includes hidden/ignored directories, never follows descendant symlinks and reports them as incomplete. Use canonical paths and refresh applicable instruction discovery when needed after effects; external concurrent renames remain possible.",
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
                    before_listing(&directory);
                    let entries = match crate::workspace_read_dir::read_directory(&directory, &authority) {
                        Ok(entries) => entries,
                        Err(error @ (AppError::Conflict(_) | AppError::Forbidden(_))) => return Err(error),
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
                    after_listing(&directory);
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct NullEvents;

    impl nomifun_realtime::UserEventSink for NullEvents {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn instruction_metadata_never_adopts_an_outside_entry_kind() {
        use std::fs;
        let fixture = match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => tempfile::Builder::new().prefix("scope-metadata-").tempdir_in(parent).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        let retained = root.join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(outside.join("target")).unwrap();
        fs::write(parent.join("target"), b"inside file").unwrap();
        let before = {
            let (parent, retained, outside) = (parent.clone(), retained.clone(), outside.clone());
            move || { fs::rename(&parent, &retained).unwrap(); junction::create(&outside, &parent).unwrap(); }
        };
        let after = {
            let (parent, retained) = (parent.clone(), retained.clone());
            move || { junction::delete(&parent).unwrap(); fs::rename(&retained, &parent).unwrap(); }
        };
        let binding = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner", [WORKSPACE_READ_OPERATION], &root).unwrap();
        let service = FileService::new(Arc::new(NullEvents), vec![]);
        let result = service.instruction_scope_with_all_hooks(&binding,
            AgentInstructionScopeRequest { path: "parent/target".into(), recursive: false }, before, after, |_| {}, |_| {}).await;
        if let Ok(observed) = &result {
            if observed.kind != "file" {
                fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(observed).unwrap()).unwrap();
                panic!("instruction kind came from outside the workspace; retained fixture: {}", fixture.keep().display());
            }
        }
        assert_eq!(fs::read(parent.join("target")).unwrap(), b"inside file");
        assert!(outside.join("target").is_dir());
        let observed = service.instruction_scope_for_agent_session(&binding,
            AgentInstructionScopeRequest { path: "parent/target".into(), recursive: false }).await.unwrap();
        assert_eq!(observed.kind, "file");
        assert_eq!(observed.directories, BTreeSet::from(["parent".into()]));
    }

    #[tokio::test]
    async fn instruction_metadata_preserves_file_directory_and_deep_missing_scope() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();
        std::fs::write(root.path().join("directory/existing.txt"), b"not an empty directory").unwrap();
        std::fs::write(root.path().join("source.txt"), b"source").unwrap();
        let binding = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner", [WORKSPACE_READ_OPERATION], root.path()).unwrap();
        let service = FileService::new(Arc::new(NullEvents), vec![]);
        for (path, kind, directory) in [(".", "directory", ""), ("directory", "directory", "directory"),
            ("source.txt", "file", ""), ("future/nested/new.txt", "missing", "future/nested")] {
            let observed = service.instruction_scope_for_agent_session(&binding,
                AgentInstructionScopeRequest { path: path.into(), recursive: false }).await.unwrap();
            assert_eq!(observed.kind, kind, "{path}");
            assert_eq!(observed.directories, BTreeSet::from([directory.into()]));
            assert!(observed.complete);
            assert_eq!(observed.entries_scanned, 0);
            let metadata = serde_json::to_value(&observed).unwrap();
            assert_eq!(metadata["observation_kind"], "instruction_scope");
            assert_eq!(metadata["is_directory_listing"], false);
        }
        for path in ["source.txt/child", "../outside", ".nomifun/internal"] {
            assert!(service.instruction_scope_for_agent_session(&binding,
                AgentInstructionScopeRequest { path: path.into(), recursive: false }).await.is_err(), "{path}");
        }
        assert!(!root.path().join("future").exists());
    }

    #[cfg(windows)]
    async fn transient_directory_escape(replace_root: bool) {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::fs;
        let fixture = match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => tempfile::Builder::new().prefix("instruction-race-").tempdir_in(parent).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let root = fixture.path().join("workspace");
        let target = if replace_root { root.clone() } else { root.join("parent") };
        let retained = fixture.path().join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir(&outside).unwrap();
        for index in 0..13 { fs::write(outside.join(format!("outside-{index}")), b"outside").unwrap(); }
        let canonical = fs::canonicalize(&target).unwrap();
        let scope = crate::resource::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], &root).unwrap();
        let files = FileService::new(Arc::new(NullEvents), vec![]);
        let redirected = Arc::new(AtomicBool::new(false));
        let before = {
            let (target, retained, outside, canonical, redirected) = (target.clone(), retained.clone(), outside.clone(), canonical.clone(), redirected.clone());
            move |directory: &Path| {
                assert_eq!(directory, canonical);
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
        let after = { let restore = restore.clone(); move |_: &Path| restore() };
        let requested = if replace_root { "." } else { "parent" };
        let result = files.instruction_scope_with_hooks(&scope,
            AgentInstructionScopeRequest { path: requested.into(), recursive: true }, before, after).await;
        restore();
        if let Ok(observed) = &result {
            fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(observed).unwrap()).unwrap();
            panic!("instruction scope accepted outside metadata; retained fixture: {}", fixture.keep().display());
        }
        assert!(matches!(result, Err(AppError::Conflict(_) | AppError::Forbidden(_))));
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 13);
        fs::write(target.join("AGENTS.md"), b"local instructions").unwrap();
        let observed = files.instruction_scope_for_agent_session(&scope,
            AgentInstructionScopeRequest { path: requested.into(), recursive: true }).await.unwrap();
        assert!(observed.complete);
        assert_eq!(observed.entries_scanned, 1);
        assert_eq!(observed.directories, BTreeSet::from([if replace_root { "".into() } else { "parent".into() }]));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn transient_instruction_directory_junction_never_supplies_outside_entries() {
        transient_directory_escape(false).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn transient_instruction_root_junction_never_supplies_outside_entries() {
        transient_directory_escape(true).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn recursive_scope_keeps_hidden_ignored_instructions_and_reports_links_incomplete() {
        use std::fs;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), b"ignored/\n").unwrap();
        for (directory, filename) in [(".hidden/空 格🐱", "AGENTS.override.md"), ("ignored", "agents.md")] {
            fs::create_dir_all(root.path().join(directory)).unwrap();
            fs::write(root.path().join(directory).join(filename), b"local instructions").unwrap();
        }
        fs::write(outside.path().join("AGENTS.md"), b"outside instructions").unwrap();
        junction::create(outside.path(), root.path().join("escape")).unwrap();
        let scope = crate::resource::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], root.path()).unwrap();
        let files = FileService::new(Arc::new(NullEvents), vec![]);
        let observed = files.instruction_scope_for_agent_session(&scope,
            AgentInstructionScopeRequest { path: ".".into(), recursive: true }).await.unwrap();
        assert!(!observed.complete);
        assert_eq!(observed.incomplete_reasons, BTreeSet::from(["symlink_entry".into()]));
        assert_eq!(observed.directories, BTreeSet::from(["".into(), ".hidden/空 格🐱".into(), "ignored".into()]));
        assert_eq!(fs::read(outside.path().join("AGENTS.md")).unwrap(), b"outside instructions");
    }

    #[tokio::test]
    async fn root_scope_uses_the_workspace_binding_without_granting_dot_file_operations() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("AGENTS.md"), "root rules").unwrap();
        let scope = crate::resource::workspace_binding(
            nomifun_common::generate_id(),
            "binding",
            "workspace",
            "owner",
            [WORKSPACE_READ_OPERATION],
            workspace.path(),
        )
        .unwrap();
        let files = FileService::new(Arc::new(NullEvents), vec![]);

        let observed = files
            .instruction_scope_for_agent_session(
                &scope,
                AgentInstructionScopeRequest { path: ".".into(), recursive: true },
            )
            .await
            .unwrap();

        assert_eq!(observed.path, ".");
        assert_eq!(observed.canonical_path, "");
        assert_eq!(observed.kind, "directory");
        assert!(observed.complete);
        assert!(observed.directories.contains(""));
        assert!(scope.resolve_relative_path(".").is_err());
    }
}
