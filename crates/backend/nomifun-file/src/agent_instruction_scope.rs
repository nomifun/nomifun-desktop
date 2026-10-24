//! Bounded instruction-scope discovery through the Session filesystem owner.
//! Metadata only; neither shell parsing nor a filesystem snapshot/write lease.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

use crate::{AgentSessionWorkspaceBinding, FileService, WORKSPACE_READ_OPERATION};

// The permit stays with a blocking scan even if the caller stops waiting.
static SCOPE_READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstructionScopeRequest {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub path_semantics: nomi_native_files::NativePathSemantics,
}

#[derive(Serialize)]
pub struct AgentInstructionScope {
    pub observation_kind: &'static str,
    pub is_directory_listing: bool,
    pub path: String,
    pub canonical_path: String,
    pub kind: &'static str,
    pub recursive: bool,
    #[serde(skip_serializing_if = "nomi_native_files::NativePathSemantics::is_follow")]
    pub path_semantics: nomi_native_files::NativePathSemantics,
    /// Ancestor loading is engine policy; these are directories to inspect.
    pub directories: BTreeSet<String>,
    pub complete: bool,
    pub incomplete_reasons: BTreeSet<String>,
    pub entries_scanned: usize,
    pub notice: &'static str,
}

impl FileService {
    pub async fn instruction_scope_for_agent_session(
        &self, scope: &AgentSessionWorkspaceBinding, request: AgentInstructionScopeRequest,
    ) -> Result<AgentInstructionScope, AppError> {
        self.instruction_scope_with_hooks(scope, request, |_| {}, |_| {}).await
    }

    async fn instruction_scope_with_hooks(
        &self, scope: &AgentSessionWorkspaceBinding, request: AgentInstructionScopeRequest,
        before_listing: impl FnMut(&Path) + Send + 'static,
        after_listing: impl FnMut(&Path) + Send + 'static,
    ) -> Result<AgentInstructionScope, AppError> {
        self.instruction_scope_with_all_hooks(scope, request, || {}, || {}, before_listing, after_listing).await
    }

    async fn instruction_scope_with_all_hooks(
        &self, scope: &AgentSessionWorkspaceBinding, request: AgentInstructionScopeRequest,
        before_metadata: impl FnOnce() + Send + 'static,
        after_metadata: impl FnOnce() + Send + 'static,
        before_listing: impl FnMut(&Path) + Send + 'static,
        after_listing: impl FnMut(&Path) + Send + 'static,
    ) -> Result<AgentInstructionScope, AppError> {
        scope.require_operation(WORKSPACE_READ_OPERATION)?;
        validate_agent_instruction_scope_request(&request)?;
        let target = scope.resolve_path(if request.path == "." { "" } else { &request.path })?;
        let root = scope.workspace_root().to_owned();
        let authority = scope.authority();
        let permit = SCOPE_READS.acquire().await.map_err(|_| AppError::Conflict("Instruction scope reader is closed".into()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            let access = WorkspaceDirectoryAccess { authority };
            let native = nomi_native_files::acquire_instruction_scope_with_semantics_and_access(
                &target, request.recursive, request.path_semantics, &access, before_metadata, after_metadata, before_listing, after_listing,
            ).map_err(crate::native_file_acquisition_error)?;
            project_agent_instruction_scope(request, native, &root)
        }).await.map_err(|error| AppError::Internal(format!("instruction discovery failed: {error}")))?
    }
}

pub fn validate_agent_instruction_scope_request(request: &AgentInstructionScopeRequest) -> Result<(), AppError> {
    if request.path.is_empty() || request.path.len() > 4096 || request.path.contains('\0') {
        return Err(AppError::BadRequest("invalid native instruction scope path".into()));
    }
    Ok(())
}

/// Pure owner projection for either native execution identity. The caller
/// supplies its already host-resolved project root; this function performs no
/// filesystem access and cannot adopt an external path as project ownership.
pub fn project_agent_instruction_scope(
    request: AgentInstructionScopeRequest, native: nomi_native_files::NativeInstructionScope, workspace_root: &Path,
) -> Result<AgentInstructionScope, AppError> {
    validate_agent_instruction_scope_request(&request)?;
    let invalid = || AppError::Conflict("native instruction acquisition differs from its bounded contract".into());
    let valid_path = |value: &str| {
        let path = Path::new(value);
        !value.is_empty() && value.len() <= 4096 && !value.contains('\0') && path.is_absolute()
            && !path.components().any(|part| matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir))
    };
    let canonical = Path::new(&native.canonical_path);
    let base = if native.kind == "directory" { canonical } else { canonical.parent().ok_or_else(invalid)? };
    if request.recursive != native.recursive || request.path_semantics != native.path_semantics
        || !(matches!(native.kind.as_str(), "file" | "directory" | "missing")
            || (native.kind == "link" && native.path_semantics == nomi_native_files::NativePathSemantics::Entry))
        || !valid_path(&native.canonical_path) || native.directories.is_empty() || native.directories.len() > 65
        || native.entries_scanned > 20_001 || (!native.recursive && native.entries_scanned != 0)
        || native.complete != native.incomplete_reasons.is_empty()
        || !native.directories.iter().any(|path| Path::new(path) == base)
        || native.directories.iter().any(|path| !valid_path(path)
            || (Path::new(path) != base && !(native.recursive && native.kind == "directory" && Path::new(path).starts_with(base))))
    { return Err(invalid()); }
    let kind = match native.kind.as_str() { "file" => "file", "directory" => "directory", "link" => "link", _ => "missing" };
    let result = AgentInstructionScope {
        observation_kind: "instruction_scope", is_directory_listing: false, path: request.path,
        canonical_path: crate::path_safety::session_path_label(workspace_root, canonical)?, kind,
        recursive: native.recursive, path_semantics: native.path_semantics,
        directories: native.directories.into_iter().map(|path| crate::path_safety::session_path_label(workspace_root, Path::new(&path)))
            .collect::<Result<BTreeSet<_>, _>>()?,
        complete: native.complete, incomplete_reasons: native.incomplete_reasons, entries_scanned: native.entries_scanned,
        notice: "Instruction-location metadata, not a filesystem entry listing, snapshot, permission grant or proof of shell access scope. entries_scanned counts discovery traversal; nonrecursive metadata has zero scans even for a nonempty directory. complete describes instruction discovery, not directory contents. Load ancestor AGENTS.override.md/AGENTS.md for each directory. Follow semantics resolve the final link and report descendant links as incomplete. Entry semantics address the final link itself; recursive directory-entry mutations count child links without traversing their targets. Hidden/ignored real directories remain included. Use canonical paths and refresh applicable instruction discovery when needed after effects; external concurrent renames remain possible.",
    };
    if !crate::agent_text_read::fits_text_result(&result, 0)? {
        return Err(AppError::BadRequest("Instruction scope result exceeds the bounded envelope; narrow the path".into()));
    }
    Ok(result)
}

struct WorkspaceDirectoryAccess { authority: crate::PathAuthority }
impl nomi_native_files::NativeDirectoryAccess for WorkspaceDirectoryAccess {
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, nomi_native_files::NativeFileError> {
        crate::path_safety::validate_native_read_path_authority(&path.to_string_lossy(), &self.authority)
            .map_err(crate::native_acquisition::owner_error_to_native)
    }
    fn canonicalize_entry(&self, path: &Path) -> Result<PathBuf, nomi_native_files::NativeFileError> {
        crate::path_safety::canonical_native_read_entry(path, &self.authority).map_err(crate::native_acquisition::owner_error_to_native)
    }
    fn metadata(&self, path: &Path) -> Result<Option<nomi_native_files::NativeEntryKind>, nomi_native_files::NativeFileError> {
        crate::workspace_read_dir::metadata(path, &self.authority).map_err(crate::native_acquisition::owner_error_to_native)
            .map(|metadata| metadata.map(|entry| {
                let kind = entry.metadata.file_type();
                if kind.is_symlink() { nomi_native_files::NativeEntryKind::Link }
                else if kind.is_dir() { nomi_native_files::NativeEntryKind::Directory }
                else if kind.is_file() { nomi_native_files::NativeEntryKind::File }
                else { nomi_native_files::NativeEntryKind::Other }
            }))
    }
    fn entries(&self, path: &Path) -> Result<Box<dyn Iterator<Item = Result<nomi_native_files::NativeDirectoryEntry, nomi_native_files::NativeFileError>>>, nomi_native_files::NativeFileError> {
        let entries = crate::workspace_read_dir::read_directory(path, &self.authority).map_err(crate::native_acquisition::owner_error_to_native)?;
        Ok(Box::new(entries.map(|entry| {
            let entry = entry.map_err(|error| nomi_native_files::NativeFileError::io("enumerating native instruction entry", error))?;
            let kind = entry.file_type().map_err(|error| nomi_native_files::NativeFileError::io("classifying native instruction entry",
                error.raw_os_error().map(std::io::Error::from_raw_os_error)
                    .unwrap_or_else(|| std::io::Error::new(error.kind(), error.to_string()))))?;
            Ok(nomi_native_files::NativeDirectoryEntry { path: entry.path(),
                kind: if kind.is_symlink() { nomi_native_files::NativeEntryKind::Link }
                else if kind.is_dir() { nomi_native_files::NativeEntryKind::Directory }
                else if kind.is_file() { nomi_native_files::NativeEntryKind::File }
                else { nomi_native_files::NativeEntryKind::Other } })
        })))
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
            AgentInstructionScopeRequest { path: "parent/target".into(), recursive: false, path_semantics: Default::default() }, before, after, |_| {}, |_| {}).await;
        if let Ok(observed) = &result {
            if observed.kind != "file" {
                fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(observed).unwrap()).unwrap();
                panic!("instruction kind came from outside the workspace; retained fixture: {}", fixture.keep().display());
            }
        }
        assert_eq!(fs::read(parent.join("target")).unwrap(), b"inside file");
        assert!(outside.join("target").is_dir());
        let observed = service.instruction_scope_for_agent_session(&binding,
            AgentInstructionScopeRequest { path: "parent/target".into(), recursive: false, path_semantics: Default::default() }).await.unwrap();
        assert_eq!(observed.kind, "file");
        assert_eq!(observed.directories, BTreeSet::from(["parent".into()]));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_worker_and_login_owner_share_exact_scope_projection() {
        let fixture = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        std::fs::create_dir_all(root.join("nested/.hidden/deep")).unwrap();
        std::fs::create_dir(root.join("outside")).unwrap();
        std::fs::write(root.join("nested/AGENTS.md"), b"parent rules").unwrap();
        std::fs::write(root.join("nested/.hidden/deep/AGENTS.override.md"), b"deep rules").unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("nested/alias")).unwrap();
        let scope = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner", [WORKSPACE_READ_OPERATION], &root).unwrap();
        let service = FileService::new(Arc::new(NullEvents), vec![]);
        for (path, recursive) in [("./nested", true), ("nested/AGENTS.md", false), ("future/nested/file", false), ("nested/alias", false)] {
            let request = AgentInstructionScopeRequest { path: path.into(), recursive, path_semantics: Default::default() };
            let ordinary = service.instruction_scope_for_agent_session(&scope, request.clone()).await.unwrap();
            let target = scope.resolve_path(path).unwrap();
            let native = nomi_native_files::acquire_instruction_scope(&target, recursive).unwrap();
            let projected = project_agent_instruction_scope(request, native, &root).unwrap();
            assert_eq!(serde_json::to_value(ordinary).unwrap(), serde_json::to_value(projected).unwrap(), "{path}");
        }
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
                AgentInstructionScopeRequest { path: path.into(), recursive: false, path_semantics: Default::default() }).await.unwrap();
            assert_eq!(observed.kind, kind, "{path}");
            assert_eq!(observed.directories, BTreeSet::from([directory.into()]));
            assert!(observed.complete);
            assert_eq!(observed.entries_scanned, 0);
            let metadata = serde_json::to_value(&observed).unwrap();
            assert_eq!(metadata["observation_kind"], "instruction_scope");
            assert_eq!(metadata["is_directory_listing"], false);
        }
        for path in ["source.txt/child", "bad\0path"] {
            assert!(service.instruction_scope_for_agent_session(&binding,
                AgentInstructionScopeRequest { path: path.into(), recursive: false, path_semantics: Default::default() }).await.is_err(), "{path}");
        }
        assert!(!root.path().join("future").exists());
    }

    #[tokio::test]
    async fn nested_unicode_scope_labels_round_trip_through_the_native_file_owner() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let directory = workspace.join("项目").join("层一").join("层二");
        let outside = fixture.path().join("outside").join("外部");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(directory.join("file.txt"), b"original file").unwrap();
        std::fs::write(directory.join("AGENTS.md"), "nested Unicode rules").unwrap();
        std::fs::write(outside.join("file.txt"), b"outside file").unwrap();
        let root = std::fs::canonicalize(&workspace).unwrap();
        let target = std::fs::canonicalize(directory.join("file.txt")).unwrap();
        let scope = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION], &root).unwrap();
        let files = FileService::new(Arc::new(NullEvents), vec![]);
        let portable = "项目/层一/层二/file.txt";
        let native_relative = Path::new("项目").join("层一").join("层二").join("file.txt");
        for requested in [portable, native_relative.to_str().unwrap(), target.to_str().unwrap()] {
            let observed = files.instruction_scope_for_agent_session(&scope,
                AgentInstructionScopeRequest { path: requested.into(), recursive: false, path_semantics: Default::default() }).await.unwrap();
            assert_eq!(observed.path, requested, "the accepted request spelling is preserved");
            assert_eq!(observed.canonical_path, portable);
            let label = nomifun_agent_contracts::SessionPathLabel::parse(observed.canonical_path).unwrap();
            assert_eq!(root.join(label.native_path()), target);
            let directory_label = nomifun_agent_contracts::SessionPathLabel::parse(observed.directories.iter().next().unwrap()).unwrap();
            let instruction = directory_label.native_path().join("AGENTS.md");
            let read = files.read_text_page_for_agent_session(&scope,
                serde_json::from_value(serde_json::json!({"path":instruction.to_str().unwrap()})).unwrap())
                .await.unwrap().unwrap();
            assert_eq!(read.content, "nested Unicode rules");
        }
        let outside_file = std::fs::canonicalize(outside.join("file.txt")).unwrap();
        let observed = files.instruction_scope_for_agent_session(&scope,
            AgentInstructionScopeRequest { path: outside_file.to_str().unwrap().into(), recursive: false, path_semantics: Default::default() }).await.unwrap();
        let label = nomifun_agent_contracts::SessionPathLabel::parse(observed.canonical_path).unwrap();
        assert!(label.native_path().is_absolute());
        assert_eq!(label.native_path(), outside_file);
        assert!(!label.native_path().starts_with(&root), "an external observation does not adopt project ownership");
        let denied = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [crate::WORKSPACE_WRITE_OPERATION], &root).unwrap();
        assert!(files.instruction_scope_for_agent_session(&denied,
            AgentInstructionScopeRequest { path: portable.into(), recursive: false, path_semantics: Default::default() }).await.is_err());
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
            AgentInstructionScopeRequest { path: requested.into(), recursive: true, path_semantics: Default::default() }, before, after).await;
        restore();
        if let Ok(observed) = &result {
            fs::write(fixture.path().join("observation.json"), serde_json::to_vec_pretty(observed).unwrap()).unwrap();
            panic!("instruction scope accepted outside metadata; retained fixture: {}", fixture.keep().display());
        }
        assert!(matches!(result, Err(AppError::NativeFileAcquisition {
            code: "FILE_CHANGED_DURING_READ", kind: None, raw_os_error: None, ..
        })), "instruction namespace replacement must retain the typed Changed observation");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 13);
        fs::write(target.join("AGENTS.md"), b"local instructions").unwrap();
        let observed = files.instruction_scope_for_agent_session(&scope,
            AgentInstructionScopeRequest { path: requested.into(), recursive: true, path_semantics: Default::default() }).await.unwrap();
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
            AgentInstructionScopeRequest { path: ".".into(), recursive: true, path_semantics: Default::default() }).await.unwrap();
        assert!(!observed.complete);
        assert_eq!(observed.incomplete_reasons, BTreeSet::from(["symlink_entry".into()]));
        assert_eq!(observed.directories, BTreeSet::from(["".into(), ".hidden/空 格🐱".into(), "ignored".into()]));
        assert_eq!(fs::read(outside.path().join("AGENTS.md")).unwrap(), b"outside instructions");
    }

    #[tokio::test]
    async fn root_scope_uses_the_session_default_directory() {
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
                AgentInstructionScopeRequest { path: ".".into(), recursive: true, path_semantics: Default::default() },
            )
            .await
            .unwrap();

        assert_eq!(observed.path, ".");
        assert_eq!(observed.canonical_path, "");
        assert_eq!(observed.kind, "directory");
        assert!(observed.complete);
        assert!(observed.directories.contains(""));
        assert!(scope.resolve_path(".").is_ok());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn entry_scope_and_canonical_unlink_keep_the_junction_target_intact() {
        use nomi_native_files::NativePathSemantics::{Entry, Follow};
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&workspace).unwrap(); std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("AGENTS.md"), b"outside rules").unwrap();
        std::fs::write(outside.join("value"), b"preserved").unwrap();
        let link = workspace.join("link"); junction::create(&outside, &link).unwrap();
        let binding = crate::workspace_binding(nomifun_common::generate_id(), "binding", "workspace", "owner",
            [WORKSPACE_READ_OPERATION, crate::WORKSPACE_DELETE_OPERATION, crate::WORKSPACE_WRITE_OPERATION], &workspace).unwrap();
        let files = FileService::new(Arc::new(NullEvents), vec![]);
        let request = |path: &str, recursive, path_semantics| AgentInstructionScopeRequest { path: path.into(), recursive, path_semantics };
        let entry = files.instruction_scope_for_agent_session(&binding, request("link", true, Entry)).await.unwrap();
        assert_eq!(entry.kind, "link"); assert_eq!(entry.canonical_path, "link");
        assert_eq!(entry.directories, BTreeSet::from([String::new()])); assert!(entry.complete);
        let followed = files.instruction_scope_for_agent_session(&binding, request("link", false, Follow)).await.unwrap();
        assert_eq!(followed.kind, "directory");
        assert_eq!(Path::new(&followed.canonical_path), std::fs::canonicalize(&outside).unwrap());
        let tree = files.instruction_scope_for_agent_session(&binding, request(".", true, Entry)).await.unwrap();
        assert!(tree.complete); assert_eq!(tree.directories, BTreeSet::from([String::new()]));
        let renamed = files.rename_entry_for_agent_session(&binding, "link", "renamed").await.unwrap();
        assert!(std::fs::symlink_metadata(&renamed).unwrap().file_type().is_symlink());
        files.remove_entry_for_agent_session(&binding, &renamed).await.unwrap();
        assert!(!link.exists());
        assert_eq!(std::fs::read(outside.join("value")).unwrap(), b"preserved");
        assert_eq!(std::fs::read(outside.join("AGENTS.md")).unwrap(), b"outside rules");
    }
}
