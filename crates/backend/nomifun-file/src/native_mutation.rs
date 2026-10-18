//! A Windows file action captures its native footprint before Effect admission
//! and retains the parent/Entry objects that own execution until I/O finishes.
use std::path::{Path, PathBuf};

use nomifun_agent_contracts::{NativeMutationNamespace, NativeMutationPlatform,
    NativeMutationSemantics, NativeMutationTarget, NativeMutationTargets};
use nomifun_common::AppError;
use crate::{AgentSessionWorkspaceBinding, FileService};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeMutationRequest {
    Write { path: String },
    Patch { paths: Vec<String> },
    Delete { path: String },
    Rename { path: String, destination: String },
}

pub struct PreparedNativeMutation {
    scope: AgentSessionWorkspaceBinding,
    request: NativeMutationRequest,
    targets: NativeMutationTargets,
    pub(crate) pins: Vec<NativeMutationPin>,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

pub(crate) struct NativeMutationPin {
    pub(crate) path: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) parent: cap_std::fs::Dir,
    pub(crate) object: Option<std::fs::File>,
    _ancestors: Vec<std::fs::File>,
}

impl PreparedNativeMutation {
    pub fn targets(&self) -> &NativeMutationTargets { &self.targets }
    pub(crate) fn require(&self, scope: &AgentSessionWorkspaceBinding, request: &NativeMutationRequest) -> Result<(), AppError> {
        if &self.scope == scope && &self.request == request { Ok(()) }
        else { Err(AppError::BadRequest("prepared native mutation does not match this file action".into())) }
    }
}

impl NativeMutationPin {
    pub(crate) fn prepare_parent(&self) -> Result<crate::workspace_write::PreparedParent, AppError> {
        crate::workspace_write::prepare_pinned_parent(self.path.clone(), self.root.clone(), self.parent.try_clone()
            .map_err(|error| crate::path_safety::native_io_error("retaining admitted native parent", error))?)
    }
}

impl FileService {
    /// This acquisition creates no files or directories and holds no Store
    /// transaction. Admission persists `targets()` before the pins execute.
    pub async fn prepare_native_mutation_for_agent_session(
        &self, scope: &AgentSessionWorkspaceBinding, request: NativeMutationRequest,
    ) -> Result<PreparedNativeMutation, AppError> {
        scope.require_operation(match &request {
            NativeMutationRequest::Delete { .. } => crate::WORKSPACE_DELETE_OPERATION,
            _ => crate::WORKSPACE_WRITE_OPERATION,
        })?;
        let guard = crate::service::NATIVE_FILE_MUTATIONS.lock().await;
        let entries: Vec<(String, NativeMutationSemantics)> = match &request {
            NativeMutationRequest::Write { path } => vec![(path.clone(), NativeMutationSemantics::Follow)],
            NativeMutationRequest::Patch { paths } => paths.iter().map(|path| (path.clone(), NativeMutationSemantics::Follow)).collect(),
            NativeMutationRequest::Delete { path } => vec![(path.clone(), NativeMutationSemantics::Entry)],
            NativeMutationRequest::Rename { path, destination } => vec![(path.clone(), NativeMutationSemantics::Entry), (destination.clone(), NativeMutationSemantics::Entry)],
        };
        if entries.is_empty() || entries.len() > crate::service::MAX_AGENT_PATCH_FILES {
            return Err(AppError::BadRequest("native mutation target count is outside its bounded contract".into()));
        }
        let entries = entries.into_iter().map(|(path, semantics)| {
            if path.is_empty() { return Err(AppError::BadRequest("native mutation requires an explicit target".into())); }
            Ok((scope.resolve_path(&path)?, semantics))
        }).collect::<Result<Vec<_>, AppError>>()?;
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut pins: Vec<NativeMutationPin> = Vec::with_capacity(entries.len());
            let mut targets: Vec<NativeMutationTarget> = Vec::with_capacity(entries.len());
            for (index, (path, semantics)) in entries.into_iter().enumerate() {
                let entry_required = semantics == NativeMutationSemantics::Entry && index == 0;
                let rename_destination = index == 1 && matches!(request, NativeMutationRequest::Rename { .. });
                let directory_destination = rename_destination && pins.first().is_some_and(|pin|
                    pin.object.as_ref().and_then(|file| file.metadata().ok()).is_some_and(|metadata| metadata.is_dir()));
                let (pin, mut target) = capture(&path, semantics, entry_required, directory_destination)?;
                if rename_destination && targets.first().is_some_and(|target| target.subtree) { target.subtree = true; }
                // Publication produces a new object identity. Until settlement
                // we cannot prove a newly created hardlink on this volume is
                // disjoint without inventing a post-hoc historical identity.
                target.volume_wide = target.volume_wide || target.subtree || matches!(request,
                    NativeMutationRequest::Write { .. } | NativeMutationRequest::Patch { .. });
                pins.push(pin); targets.push(target);
            }
            if matches!(request, NativeMutationRequest::Write { .. } | NativeMutationRequest::Patch { .. }) {
                // One full-volume claim covers the publication range among
                // cooperating admitted Windows explicit-file actions. User
                // and other-program concurrent editing is externally coordinated.
                // Retain every real pin while avoiding repeated ancestry in
                // the original bounded Effect payload for a large patch.
                let mut volumes: Vec<NativeMutationTarget> = Vec::new();
                for mut target in targets {
                    if volumes.iter().any(|prior| prior.platform == target.platform && prior.volume_id == target.volume_id) { continue; }
                    target.ancestor_ids = vec![target.namespace.parent_id.clone()];
                    volumes.push(target);
                }
                targets = volumes;
            }
            Ok(PreparedNativeMutation { scope, request, targets: NativeMutationTargets { version: 1, targets }, pins, _guard: guard })
        }).await.map_err(|error| AppError::Internal(format!("native mutation acquisition worker stopped: {error}")))?
    }
}

#[cfg(windows)]
fn capture(path: &Path, semantics: NativeMutationSemantics, entry_required: bool, directory_destination: bool) -> Result<(NativeMutationPin, NativeMutationTarget), AppError> {
    use std::os::windows::{ffi::OsStrExt as _, fs::{MetadataExt as _, OpenOptionsExt as _}};
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE,
        FILE_ATTRIBUTE_REPARSE_POINT, SYNCHRONIZE};
    let io = |error| crate::path_safety::native_io_error("capturing native mutation target", error);
    let normalized = nomi_native_files::windows::normalize_path(path).map_err(io)?;
    let path = if semantics == NativeMutationSemantics::Entry && std::fs::symlink_metadata(&normalized).is_ok() {
        crate::path_safety::canonical_entry(&normalized)?
    } else { crate::workspace_write::validate_global_target(&normalized)? };
    let target_parent = path.parent().ok_or_else(|| AppError::BadRequest("native target has no parent".into()))?;
    let mut root = target_parent;
    loop {
        match std::fs::symlink_metadata(root) {
            Ok(metadata) if metadata.is_dir() => break,
            Ok(_) => return Err(AppError::BadRequest("native mutation parent is not a directory".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                root = root.parent().ok_or_else(|| io(error))?;
            }
            Err(error) => return Err(io(error)),
        }
    }
    // Entry operations retain only parent identity, then use their original
    // DELETE handle and relative destination. Publication acquires the actual
    // add right it needs; metadata-only identity pins are not name locks.
    let directory = if entry_required { nomi_native_files::windows::open_namespace_pin(None, root) }
        else { nomi_native_files::windows::open_rename_parent(root, root != target_parent || directory_destination) }.map_err(io)?;
    if !crate::workspace_write::windows_parent_path_matches(&nomi_native_files::windows::final_path(&directory).map_err(io)?, root)
        || directory.metadata().map_err(io)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(AppError::Conflict("native mutation parent changed while being pinned".into()));
    }
    let parent_id = crate::windows_cleanup::durable_identity_token(&directory).map_err(io)?;
    let mut ancestors = Vec::new(); let mut ancestor_ids = Vec::new();
    for ancestor in root.ancestors() {
        let file = nomi_native_files::windows::open_namespace_pin(None, ancestor).map_err(io)?;
        ancestor_ids.push(crate::windows_cleanup::durable_identity_token(&file).map_err(io)?);
        ancestors.push(file);
    }
    let object = if entry_required {
        Some(nomi_native_files::windows::open_delete(None, &path).map_err(io)?)
    } else {
        match std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(&path) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io(error)),
        }
    };
    if let Some(file) = &object {
        if nomi_native_files::windows::final_path(file).map_err(io)? != path {
            return Err(AppError::Conflict("native mutation target changed while being pinned".into()));
        }
    }
    let subtree = object.as_ref().map(|file| file.metadata()).transpose().map_err(io)?
        .is_some_and(|metadata| metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0);
    let mut object_id = object.as_ref().map(crate::windows_cleanup::durable_identity_token).transpose().map_err(io)?;
    let mut components = Vec::new();
    for component in path.strip_prefix(root).map_err(|_| AppError::Conflict("native target left its captured parent".into()))?.components() {
        let std::path::Component::Normal(name) = component else { return Err(AppError::BadRequest("invalid native target component".into())); };
        // Base and ADS share a namespace claim, including an absent stream.
        let value: Vec<u16> = name.encode_wide().take_while(|unit| *unit != 58).collect();
        // Freeze one conservative NT case key even in case-sensitive folders.
        // A later directory case-mode change cannot evade an unknown claim.
        components.push(value.into_iter().map(|unit| {
            // SAFETY: the OS maps one UTF-16 code unit without pointers.
            unsafe { windows_sys::Wdk::System::SystemServices::RtlUpcaseUnicodeChar(unit) }
        }).collect());
    }
    if object_id.is_none() && let Some(name) = path.file_name().and_then(|name| name.to_str())
        && let Some((base, _)) = name.split_once(':') {
        let base = nomi_native_files::windows::join_component(target_parent, std::ffi::OsStr::new(base)).map_err(io)?;
        match nomi_native_files::windows::open_metadata(None, &base) {
            Ok(file) => { object_id = Some(crate::windows_cleanup::durable_identity_token(&file).map_err(io)?); }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(io(error)),
        }
    }
    // Follow publication has an explicit full-volume claim. Its observed old
    // file ID is useful evidence, but keeping another handle to that old inode
    // would obstruct ReplaceFile backup cleanup on basic-disposition systems.
    // The actual I/O keeps the admitted writable parent and publication guard;
    // only Entry operations consume the original target DELETE handle.
    let object = if semantics == NativeMutationSemantics::Follow { None } else { object };
    let root = root.to_path_buf();
    let pin = NativeMutationPin { path, root, parent: cap_std::fs::Dir::from_std_file(directory), object, _ancestors: ancestors };
    let volume_id = parent_id.split('-').next().expect("native identity includes a volume").to_owned();
    Ok((pin, NativeMutationTarget { platform: NativeMutationPlatform::Windows, object_id, volume_id, volume_wide: subtree,
        namespace: NativeMutationNamespace { parent_id, components }, ancestor_ids, subtree, semantics }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{fs, sync::Arc};
    struct Events;
    impl nomifun_realtime::UserEventSink for Events {
        fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }
    fn service() -> FileService { FileService::new(Arc::new(Events), vec![]) }
    fn scope(root: &Path) -> AgentSessionWorkspaceBinding {
        crate::workspace_binding(nomifun_common::generate_id(), "native-pins", "native-workspace", "owner", [
            crate::WORKSPACE_READ_OPERATION, crate::WORKSPACE_WRITE_OPERATION, crate::WORKSPACE_DELETE_OPERATION,
        ], root).unwrap()
    }

    #[tokio::test]
    async fn capture_missing_target_has_no_filesystem_side_effect_and_freezes_native_parent() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let binding = scope(&root); let service = service();
        let prepared = service.prepare_native_mutation_for_agent_session(&binding,
            NativeMutationRequest::Write { path: "missing/nested/new.txt".into() }).await.unwrap();
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let target = &prepared.targets().targets[0];
        assert!(target.object_id.is_none()); assert!(target.volume_wide); assert!(!target.subtree);
        assert_eq!(target.namespace.components.len(), 3);
        assert!(target.ancestor_ids.contains(&target.namespace.parent_id));
        let moved = root.with_extension("moved");
        let error = fs::rename(&root, &moved).unwrap_err();
        assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
        service.write_file_with_native_mutation(&binding, "missing/nested/new.txt", b"pinned", prepared).await.unwrap();
        assert_eq!(fs::read(root.join("missing/nested/new.txt")).unwrap(), b"pinned");
    }

    #[tokio::test]
    async fn admitted_follow_executes_frozen_object_after_junction_route_changes() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(fixture.path()).unwrap();
        let first = root.join("first"); let second = root.join("second");
        fs::create_dir(&first).unwrap(); fs::create_dir(&second).unwrap();
        fs::write(first.join("file.txt"), b"first").unwrap(); fs::write(second.join("file.txt"), b"second").unwrap();
        let route = root.join("route"); junction::create(&first, &route).unwrap();
        let binding = scope(&root); let service = service();
        let prepared = service.prepare_native_mutation_for_agent_session(&binding,
            NativeMutationRequest::Write { path: "route/file.txt".into() }).await.unwrap();
        assert_eq!(prepared.pins[0].path, first.join("file.txt"));
        junction::delete(&route).unwrap(); fs::remove_dir(&route).unwrap(); junction::create(&second, &route).unwrap();
        service.write_file_with_native_mutation(&binding, "route/file.txt", b"written", prepared).await.unwrap();
        assert_eq!(fs::read(first.join("file.txt")).unwrap(), b"written");
        assert_eq!(fs::read(second.join("file.txt")).unwrap(), b"second");
        junction::delete(&route).unwrap();
    }

    #[tokio::test]
    async fn admitted_parent_pin_prevents_ancestor_move_before_publication() {
        let fixture = tempfile::tempdir().unwrap(); let root = fs::canonicalize(fixture.path()).unwrap();
        let ancestor = root.join("ancestor"); let parent = ancestor.join("parent");
        fs::create_dir_all(&parent).unwrap();
        let binding = scope(&root); let service = service();
        let prepared = service.prepare_native_mutation_for_agent_session(&binding,
            NativeMutationRequest::Write { path: "ancestor/parent/new.txt".into() }).await.unwrap();
        let moved = root.join("moved");
        for result in [fs::rename(&ancestor, &moved), crate::windows_test_support::rename_with_posix_semantics(&ancestor, &moved, false)] {
            let error = result.unwrap_err(); assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
        }
        service.write_file_with_native_mutation(&binding, "ancestor/parent/new.txt", b"pinned", prepared).await.unwrap();
        assert_eq!(fs::read(parent.join("new.txt")).unwrap(), b"pinned");
    }

    #[test]
    fn pinned_facts_converge_hardlinks_native_spelling_and_stream_base() {
        let fixture = tempfile::tempdir().unwrap(); let root = fs::canonicalize(fixture.path()).unwrap();
        let first = root.join("Mixed.txt"); let alias = root.join("hard.txt");
        fs::write(&first, b"base").unwrap(); fs::hard_link(&first, &alias).unwrap();
        let (_, target) = capture(&first, NativeMutationSemantics::Follow, false, false).unwrap();
        let (_, hardlink) = capture(&alias, NativeMutationSemantics::Follow, false, false).unwrap();
        let (_, case_alias) = capture(&root.join("MIXED.TXT"), NativeMutationSemantics::Follow, false, false).unwrap();
        let stream = nomi_native_files::windows::join_component(&root, std::ffi::OsStr::new("Mixed.txt:metadata")).unwrap();
        let (_, missing_stream) = capture(&stream, NativeMutationSemantics::Follow, false, false).unwrap();
        assert_eq!(target.object_id, hardlink.object_id); assert_eq!(target.volume_id, hardlink.volume_id);
        assert_eq!(target, case_alias);
        assert_eq!(target.object_id, missing_stream.object_id);
        assert_eq!(target.namespace, missing_stream.namespace);
        assert_ne!(target.namespace, hardlink.namespace);
    }

    #[tokio::test]
    async fn rename_uses_source_and_destination_pins_and_keeps_entry_link_semantics() {
        let fixture = tempfile::tempdir().unwrap(); let root = fs::canonicalize(fixture.path()).unwrap();
        let target = root.join("target"); fs::create_dir(&target).unwrap(); fs::write(target.join("sentinel"), b"outside").unwrap();
        let link = root.join("link"); junction::create(&target, &link).unwrap();
        let binding = scope(&root); let service = service();
        let prepared = service.prepare_native_mutation_for_agent_session(&binding,
            NativeMutationRequest::Rename { path: "link".into(), destination: "moved".into() }).await.unwrap();
        assert_eq!(prepared.targets().targets.len(), 2);
        assert!(!prepared.targets().targets[0].subtree);
        service.rename_entry_with_native_mutation(&binding, "link", "moved", prepared).await.unwrap();
        service.remove_entry_with_receipt_for_agent_session(&binding, "moved").await.unwrap();
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"outside");
    }

    #[tokio::test]
    async fn prepared_request_cannot_be_consumed_by_another_session_binding() {
        let fixture = tempfile::tempdir().unwrap(); let binding = scope(fixture.path());
        let other = scope(fixture.path()); let service = service();
        let prepared = service.prepare_native_mutation_for_agent_session(&binding,
            NativeMutationRequest::Write { path: "new.txt".into() }).await.unwrap();
        assert!(service.write_file_with_native_mutation(&other, "new.txt", b"wrong", prepared).await.is_err());
        assert!(!fixture.path().join("new.txt").exists());
    }
}
