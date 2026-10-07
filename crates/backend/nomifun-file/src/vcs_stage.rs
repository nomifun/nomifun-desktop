use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use git2::{Index, IndexEntry, IndexTime, Repository};
use nomifun_common::AppError;
use same_file::Handle as SameFileHandle;

use crate::artifact_store::is_workspace_owner_component;

const MAX_STAGE_ENTRIES: usize = 100_000;
const VCS_STAGE_OUTCOME_UNKNOWN: &str = "vcs stage publication outcome is unknown";

pub fn vcs_stage_outcome_unknown(error: &AppError) -> bool {
    error.to_string().contains(VCS_STAGE_OUTCOME_UNKNOWN)
}

pub struct WorkspaceVcsStageOwner {
    root: PathBuf,
    root_identity: SameFileHandle,
    dir: Arc<Dir>,
}

struct GitIndexTransaction {
    index_path: PathBuf,
    lock_path: PathBuf,
    working_path: PathBuf,
    working_lock_path: PathBuf,
    index: Option<Index>,
    committed: bool,
}

impl GitIndexTransaction {
    fn begin(repository: &Repository) -> Result<Self, AppError> {
        let current = repository.index().map_err(git_error)?;
        let index_path = current
            .path()
            .map(Path::to_path_buf)
            .ok_or_else(|| AppError::Conflict("Git repository has no on-disk index".into()))?;
        drop(current);

        let lock_path = append_path_suffix(&index_path, ".lock");
        let working_path = append_path_suffix(&lock_path, ".nomifun-stage");
        let working_lock_path = append_path_suffix(&working_path, ".lock");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                AppError::Conflict(format!(
                    "Git index is already locked by another writer: {error}"
                ))
            })?;
        let result = (|| {
            lock.sync_all().map_err(io_error)?;
            drop(lock);
            let _ = std::fs::remove_file(&working_path);
            let _ = std::fs::remove_file(&working_lock_path);
            if index_path.exists() {
                std::fs::copy(&index_path, &working_path).map_err(io_error)?;
            }
            let index = Index::open(&working_path).map_err(git_error)?;
            Ok(Self {
                index_path,
                lock_path: lock_path.clone(),
                working_path: working_path.clone(),
                working_lock_path: working_lock_path.clone(),
                index: Some(index),
                committed: false,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&lock_path);
            let _ = std::fs::remove_file(&working_path);
            let _ = std::fs::remove_file(&working_lock_path);
        }
        result
    }

    fn index_mut(&mut self) -> &mut Index {
        self.index.as_mut().expect("open Git index transaction")
    }

    fn commit(mut self) -> Result<(), AppError> {
        self.index_mut().write().map_err(git_error)?;
        self.index.take();
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.working_path)
            .and_then(|file| file.sync_all())
            .map_err(io_error)?;
        replace_index_file(&self.working_path, &self.index_path).map_err(|error| {
            AppError::Internal(format!("{VCS_STAGE_OUTCOME_UNKNOWN}: {error}"))
        })?;
        std::fs::remove_file(&self.lock_path).map_err(|error| {
            AppError::Internal(format!("{VCS_STAGE_OUTCOME_UNKNOWN}: {error}"))
        })?;
        sync_parent_directory(&self.index_path).map_err(|error| {
            AppError::Internal(format!("{VCS_STAGE_OUTCOME_UNKNOWN}: {error}"))
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for GitIndexTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.lock_path);
        }
        let _ = std::fs::remove_file(&self.working_path);
        let _ = std::fs::remove_file(&self.working_lock_path);
    }
}

impl WorkspaceVcsStageOwner {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, AppError> {
        let root = std::fs::canonicalize(root.as_ref()).map_err(|error| {
            AppError::BadRequest(format!("cannot resolve VCS workspace root: {error}"))
        })?;
        let dir = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|error| {
            AppError::BadRequest(format!("cannot pin VCS workspace root: {error}"))
        })?;
        let root_identity = dir_identity(&dir)?;
        let canonical_after = std::fs::canonicalize(&root).map_err(|error| {
            AppError::Conflict(format!("VCS workspace root changed while opening: {error}"))
        })?;
        let reopened = Dir::open_ambient_dir(&root, ambient_authority()).map_err(|error| {
            AppError::Conflict(format!("VCS workspace root changed while opening: {error}"))
        })?;
        if canonical_after != root || dir_identity(&reopened)? != root_identity {
            return Err(AppError::Conflict(
                "VCS workspace root changed while opening".into(),
            ));
        }
        Ok(Self {
            root,
            root_identity,
            dir: Arc::new(dir),
        })
    }

    fn verify_root(&self) -> Result<(), AppError> {
        let reopened = Dir::open_ambient_dir(&self.root, ambient_authority())
            .map_err(|error| AppError::Conflict(format!("VCS workspace root changed: {error}")))?;
        if dir_identity(&reopened)? != self.root_identity {
            return Err(AppError::Conflict(
                "VCS workspace root identity changed during staging".into(),
            ));
        }
        Ok(())
    }

    pub fn stage_and_write(
        &self,
        repository: &Repository,
        repository_prefix: &str,
        relative: &Path,
    ) -> Result<Vec<String>, AppError> {
        self.stage_and_write_with_hook(repository, repository_prefix, relative, || {})
    }

    fn stage_and_write_with_hook<F: FnOnce()>(
        &self,
        repository: &Repository,
        repository_prefix: &str,
        relative: &Path,
        after_resolve: F,
    ) -> Result<Vec<String>, AppError> {
        let mut transaction = GitIndexTransaction::begin(repository)?;
        let staged = self.stage_with_hook(
            repository,
            transaction.index_mut(),
            repository_prefix,
            relative,
            after_resolve,
        )?;
        transaction.commit()?;
        // libgit2 caches the repository-owned Index object. The transaction
        // publishes through the standard on-disk lock protocol, so force that
        // cache to observe the committed file before this owner returns.
        repository
            .index()
            .and_then(|mut index| index.read(true))
            .map_err(|error| {
                AppError::Internal(format!(
                    "{VCS_STAGE_OUTCOME_UNKNOWN}: repository index cache refresh failed: {error}"
                ))
            })?;
        Ok(staged)
    }

    #[cfg(test)]
    fn stage(
        &self,
        repository: &Repository,
        index: &mut Index,
        repository_prefix: &str,
        relative: &Path,
    ) -> Result<Vec<String>, AppError> {
        self.stage_with_hook(repository, index, repository_prefix, relative, || {})
    }

    fn stage_with_hook<F: FnOnce()>(
        &self,
        repository: &Repository,
        index: &mut Index,
        repository_prefix: &str,
        relative: &Path,
        after_resolve: F,
    ) -> Result<Vec<String>, AppError> {
        self.verify_root()?;
        let relative = normalize_relative(relative)?;
        let repo_target = join_repo_path(repository_prefix, &portable(&relative)?);
        let resolved = resolve_nofollow(&self.dir, &relative);
        match resolved {
            Ok(Resolved::Directory(directory)) => {
                let identity = dir_identity(&directory)?;
                after_resolve();
                self.verify_root()?;
                let indexed = indexed_paths_for_target(index, &repo_target)?;
                let mut staged = indexed
                    .iter()
                    .map(|path| portable(path))
                    .collect::<Result<Vec<_>, _>>()?;
                for path in indexed {
                    index.remove_path(&path).map_err(git_error)?;
                }
                let mut added = Vec::new();
                collect_directory(repository, index, &directory, &repo_target, &mut added)?;
                staged.extend(added);
                let Resolved::Directory(reopened) = resolve_nofollow(&self.dir, &relative)
                    .map_err(io_error)?
                else {
                    return Err(AppError::Conflict(
                        "VCS stage directory identity changed during traversal".into(),
                    ));
                };
                if dir_identity(&reopened)? != identity {
                    return Err(AppError::Conflict(
                        "VCS stage directory identity changed during traversal".into(),
                    ));
                }
                self.verify_root()?;
                normalize_stage_receipt(staged, repository_prefix)
            }
            Ok(Resolved::File { parent, name }) => {
                after_resolve();
                self.verify_root()?;
                let (path, identity) =
                    stage_file(repository, index, &parent, &name, &repo_target)?;
                let Resolved::File {
                    parent: reopened_parent,
                    name: reopened_name,
                } = resolve_nofollow(&self.dir, &relative).map_err(io_error)?
                else {
                    return Err(AppError::Conflict(
                        "VCS stage file identity changed during traversal".into(),
                    ));
                };
                if file_identity(&reopened_parent, &reopened_name)? != identity {
                    return Err(AppError::Conflict(
                        "VCS stage file identity changed during traversal".into(),
                    ));
                }
                self.verify_root()?;
                normalize_stage_receipt(vec![path], repository_prefix)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                after_resolve();
                self.verify_root()?;
                let paths = indexed_paths_for_target(index, &repo_target)?;
                if paths.is_empty() {
                    return Err(AppError::NotFound("VCS stage target is not tracked".into()));
                }
                let mut removed = Vec::new();
                for path in paths {
                    index.remove_path(&path).map_err(git_error)?;
                    removed.push(portable(&path)?);
                }
                self.verify_root()?;
                normalize_stage_receipt(removed, repository_prefix)
            }
            Err(error) => Err(AppError::Forbidden(format!(
                "VCS stage path is not a pinned regular file/directory: {error}"
            ))),
        }
    }
}

enum Resolved {
    File { parent: Dir, name: OsString },
    Directory(Dir),
}

fn normalize_relative(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() {
        return Ok(PathBuf::new());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value)
                if normalized.as_os_str().is_empty()
                    && (is_workspace_owner_component(value)
                        || is_git_owner_component(value)) =>
            {
                return Err(AppError::NotFound("VCS stage target was not found".into()));
            }
            Component::Normal(value) => normalized.push(value),
            _ => return Err(AppError::BadRequest("VCS stage path must be normalized and relative".into())),
        }
    }
    Ok(normalized)
}

fn is_git_owner_component(component: &OsStr) -> bool {
    component
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(".git"))
}

fn resolve_nofollow(root: &Dir, relative: &Path) -> std::io::Result<Resolved> {
    if relative.as_os_str().is_empty() {
        return root.try_clone().map(Resolved::Directory);
    }
    let components = relative.components().map(|component| match component {
        Component::Normal(value) => Ok(value.to_os_string()),
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-normal component")),
    }).collect::<Result<Vec<_>, _>>()?;
    let mut parent = root.try_clone()?;
    for component in &components[..components.len() - 1] {
        parent = parent.open_dir_nofollow(component)?;
    }
    let name = components.last().expect("non-empty components").clone();
    let metadata = parent.symlink_metadata(&name)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "symlink/reparse point"));
    }
    if metadata.is_dir() {
        parent.open_dir_nofollow(&name).map(Resolved::Directory)
    } else if metadata.is_file() {
        Ok(Resolved::File { parent, name })
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "unsupported file type"))
    }
}

fn collect_directory(
    repository: &Repository,
    index: &mut Index,
    directory: &Dir,
    repo_path: &str,
    output: &mut Vec<String>,
) -> Result<(), AppError> {
    let mut entries = directory.entries().map_err(io_error)?.collect::<Result<Vec<_>, _>>().map_err(io_error)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if output.len() >= MAX_STAGE_ENTRIES {
            return Err(AppError::Conflict(format!("VCS stage exceeds {MAX_STAGE_ENTRIES} entries")));
        }
        let name = entry.file_name();
        if is_workspace_owner_component(&name) || is_git_owner_component(&name) {
            continue;
        }
        let metadata = directory.symlink_metadata(&name).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Forbidden("VCS stage refuses symlink/reparse entries".into()));
        }
        let child_repo_path = join_repo_path(repo_path, name.to_str().ok_or_else(|| AppError::BadRequest("VCS path is not UTF-8".into()))?);
        if metadata.is_dir() {
            let child = directory.open_dir_nofollow(&name).map_err(io_error)?;
            let identity = dir_identity(&child)?;
            collect_directory(repository, index, &child, &child_repo_path, output)?;
            let reopened = directory.open_dir_nofollow(&name).map_err(io_error)?;
            if dir_identity(&reopened)? != identity {
                return Err(AppError::Conflict(
                    "VCS stage directory identity changed during traversal".into(),
                ));
            }
        } else if metadata.is_file() {
            output.push(
                stage_file(repository, index, directory, &name, &child_repo_path)?.0,
            );
        } else {
            return Err(AppError::Forbidden("VCS stage refuses unsupported file entries".into()));
        }
    }
    Ok(())
}

fn stage_file(repository: &Repository, index: &mut Index, parent: &Dir, name: &OsStr, repo_path: &str) -> Result<(String, SameFileHandle), AppError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = parent.open_with(name, &options).map_err(io_error)?.into_std();
    let identity = SameFileHandle::from_file(file.try_clone().map_err(io_error)?).map_err(io_error)?;
    let before = file.metadata().map_err(io_error)?;
    if !before.is_file() { return Err(AppError::Forbidden("VCS stage target is not a regular file".into())); }
    let mut writer = repository.blob_writer(Some(Path::new(repo_path))).map_err(git_error)?;
    std::io::copy(&mut file, &mut writer).map_err(io_error)?;
    let oid = writer.commit().map_err(git_error)?;
    let reopened = parent.open_with(name, &options).map_err(io_error)?.into_std();
    let reopened_identity = SameFileHandle::from_file(reopened.try_clone().map_err(io_error)?).map_err(io_error)?;
    let after = file.metadata().map_err(io_error)?;
    if identity != reopened_identity || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(AppError::Conflict("VCS stage source changed while reading".into()));
    }
    let path = repo_path.as_bytes().to_vec();
    let mode = index.get_path(Path::new(repo_path), 0).map(|entry| entry.mode).unwrap_or_else(|| file_mode(&after));
    index.add(&IndexEntry {
        ctime: IndexTime::new(0, 0), mtime: IndexTime::new(0, 0), dev: 0, ino: 0, mode,
        uid: 0, gid: 0, file_size: after.len().min(u32::MAX as u64) as u32, id: oid,
        flags: 0, flags_extended: 0, path,
    }).map_err(git_error)?;
    Ok((repo_path.to_owned(), identity))
}

fn dir_identity(directory: &Dir) -> Result<SameFileHandle, AppError> {
    SameFileHandle::from_file(directory.try_clone().map_err(io_error)?.into_std_file())
        .map_err(io_error)
}

fn file_identity(parent: &Dir, name: &OsStr) -> Result<SameFileHandle, AppError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options).map_err(io_error)?.into_std();
    SameFileHandle::from_file(file).map_err(io_error)
}

#[cfg(unix)] fn file_mode(metadata: &std::fs::Metadata) -> u32 { use std::os::unix::fs::PermissionsExt; if metadata.permissions().mode() & 0o111 != 0 { 0o100755 } else { 0o100644 } }
#[cfg(not(unix))] fn file_mode(_metadata: &std::fs::Metadata) -> u32 { 0o100644 }

fn indexed_paths_for_target(index: &Index, target: &str) -> Result<Vec<PathBuf>, AppError> {
    let mut output = Vec::new();
    for entry in index.iter() {
        let candidate = std::str::from_utf8(&entry.path).map_err(|_| AppError::BadRequest("Git index path is not UTF-8".into()))?;
        if target.is_empty()
            || candidate == target
            || candidate
                .strip_prefix(target)
                .is_some_and(|suffix| suffix.starts_with('/'))
        {
            output.push(PathBuf::from(candidate));
            if output.len() > MAX_STAGE_ENTRIES {
                return Err(AppError::Conflict(format!(
                    "VCS stage exceeds {MAX_STAGE_ENTRIES} entries"
                )));
            }
        }
    }
    Ok(output)
}

fn normalize_stage_receipt(
    repository_paths: Vec<String>,
    repository_prefix: &str,
) -> Result<Vec<String>, AppError> {
    let prefix = repository_prefix.trim_matches('/');
    let mut output = repository_paths
        .into_iter()
        .map(|path| {
            if prefix.is_empty() {
                return Ok(path);
            }
            path.strip_prefix(prefix)
                .and_then(|suffix| suffix.strip_prefix('/'))
                .map(str::to_owned)
                .ok_or_else(|| {
                    AppError::Conflict(format!(
                        "Git index path {path:?} escaped workspace prefix {prefix:?}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    output.sort();
    output.dedup();
    if output.len() > MAX_STAGE_ENTRIES {
        return Err(AppError::Conflict(format!(
            "VCS stage exceeds {MAX_STAGE_ENTRIES} entries"
        )));
    }
    Ok(output)
}

fn join_repo_path(prefix: &str, path: &str) -> String { match (prefix.is_empty(), path.is_empty()) { (true, _) => path.into(), (_, true) => prefix.into(), _ => format!("{prefix}/{path}") } }
fn portable(path: &Path) -> Result<String, AppError> { path.components().map(|component| match component { Component::Normal(value) => value.to_str().map(str::to_owned).ok_or_else(|| AppError::BadRequest("VCS path is not UTF-8".into())), _ => Err(AppError::BadRequest("VCS path is not normalized".into())) }).collect::<Result<Vec<_>, _>>().map(|parts| parts.join("/")) }
fn append_path_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(not(windows))]
fn replace_index_file(source: &Path, target: &Path) -> Result<(), AppError> {
    std::fs::rename(source, target).map_err(io_error)
}

#[cfg(windows)]
fn replace_index_file(source: &Path, target: &Path) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated and retained for the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Conflict("Git index has no parent directory".into()))?;
    match File::open(parent).and_then(|directory| directory.sync_all()) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}
fn io_error(error: std::io::Error) -> AppError { AppError::Conflict(error.to_string()) }
fn git_error(error: git2::Error) -> AppError { AppError::Conflict(error.to_string()) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo(root: &Path) -> Repository { Repository::init(root).unwrap() }

    #[test]
    fn stages_bytes_from_pinned_file_handles() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/lib.rs"), "safe").unwrap();
        let repository = repo(root.path());
        let owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        owner
            .stage_and_write(&repository, "", Path::new("src"))
            .unwrap();
        let index = repository.index().unwrap();
        let entry = index.get_path(Path::new("src/lib.rs"), 0).unwrap();
        assert_eq!(repository.find_blob(entry.id).unwrap().content(), b"safe");
    }

    #[test]
    fn repository_index_lock_prevents_second_host_lost_update() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("first.txt"), "first").unwrap();
        fs::write(root.path().join("second.txt"), "second").unwrap();
        let repository = repo(root.path());
        let first_owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        let second_owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        let root_path = root.path().to_path_buf();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = std::thread::spawn(move || {
            let repository = Repository::open(root_path).unwrap();
            first_owner.stage_and_write_with_hook(
                &repository,
                "",
                Path::new("first.txt"),
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
        });
        entered_rx.recv().unwrap();
        assert!(second_owner
            .stage_and_write(&repository, "", Path::new("second.txt"))
            .is_err());
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();

        second_owner
            .stage_and_write(&repository, "", Path::new("second.txt"))
            .unwrap();
        let index = repository.index().unwrap();
        assert!(index.get_path(Path::new("first.txt"), 0).is_some());
        assert!(index.get_path(Path::new("second.txt"), 0).is_some());
    }

    #[test]
    fn staging_workspace_root_records_deletions_but_never_owner_artifacts() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("deleted.txt"), "delete me").unwrap();
        let repository = repo(root.path());
        let owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        let mut index = repository.index().unwrap();
        owner
            .stage(&repository, &mut index, "", Path::new(""))
            .unwrap();
        assert!(index.get_path(Path::new("deleted.txt"), 0).is_some());
        index.write().unwrap();

        fs::remove_file(root.path().join("deleted.txt")).unwrap();
        fs::create_dir_all(root.path().join(".nomifun/artifacts")).unwrap();
        fs::write(root.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        let receipt = owner
            .stage(&repository, &mut index, "", Path::new(""))
            .unwrap();
        assert_eq!(receipt, vec!["deleted.txt"]);
        assert!(index.get_path(Path::new("deleted.txt"), 0).is_none());
        assert!(
            index
                .iter()
                .all(|entry| !entry.path.starts_with(b".nomifun/"))
        );
    }

    #[test]
    fn nested_workspace_receipt_is_complete_sorted_and_workspace_relative() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("projects/demo");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("deleted.txt"), "delete me").unwrap();
        fs::write(workspace.join("kept.txt"), "before").unwrap();
        let repository = repo(root.path());
        let owner = WorkspaceVcsStageOwner::new(&workspace).unwrap();
        owner
            .stage_and_write(&repository, "projects/demo", Path::new(""))
            .unwrap();

        fs::remove_file(workspace.join("deleted.txt")).unwrap();
        fs::write(workspace.join("kept.txt"), "after").unwrap();
        fs::write(workspace.join("new.txt"), "new").unwrap();
        let receipt = owner
            .stage_and_write(&repository, "projects/demo", Path::new(""))
            .unwrap();
        assert_eq!(receipt, vec!["deleted.txt", "kept.txt", "new.txt"]);

        let index = repository.index().unwrap();
        assert!(
            index
                .get_path(Path::new("projects/demo/deleted.txt"), 0)
                .is_none()
        );
        assert!(
            index
                .get_path(Path::new("projects/demo/kept.txt"), 0)
                .is_some()
        );
        assert!(
            index
                .get_path(Path::new("projects/demo/new.txt"), 0)
                .is_some()
        );
    }

    #[test]
    fn receipt_limit_counts_unique_union_not_before_and_after_duplicates() {
        let unique = MAX_STAGE_ENTRIES / 2 + 1;
        let mut paths = (0..unique)
            .map(|index| format!("workspace/file-{index:06}"))
            .collect::<Vec<_>>();
        paths.extend(paths.clone());
        let receipt = normalize_stage_receipt(paths, "workspace").unwrap();
        assert_eq!(receipt.len(), unique);
        assert_eq!(receipt.first().map(String::as_str), Some("file-000000"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_final_and_ancestor_symlink_swaps() {
        let root = tempfile::tempdir().unwrap(); let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap(); fs::write(outside.path().join("secret"), "outside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("src/link")).unwrap();
        let repository=repo(root.path()); let owner=WorkspaceVcsStageOwner::new(root.path()).unwrap(); let mut index=repository.index().unwrap();
        assert!(owner.stage(&repository,&mut index,"",Path::new("src/link")).is_err());
        fs::remove_file(root.path().join("src/link")).unwrap(); fs::rename(root.path().join("src"),root.path().join("owned")).unwrap(); std::os::unix::fs::symlink(outside.path(),root.path().join("src")).unwrap();
        assert!(owner.stage(&repository,&mut index,"",Path::new("src/secret")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn pinned_traversal_never_reads_outside_during_final_or_ancestor_swap_races() {
        let final_root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(final_root.path().join("src")).unwrap();
        fs::write(final_root.path().join("src/value"), "inside").unwrap();
        fs::write(outside.path().join("value"), "outside").unwrap();
        let repository = repo(final_root.path());
        let owner = WorkspaceVcsStageOwner::new(final_root.path()).unwrap();
        let mut index = repository.index().unwrap();
        let final_result = owner.stage_with_hook(
            &repository,
            &mut index,
            "",
            Path::new("src/value"),
            || {
                fs::remove_file(final_root.path().join("src/value")).unwrap();
                std::os::unix::fs::symlink(
                    outside.path().join("value"),
                    final_root.path().join("src/value"),
                )
                .unwrap();
            },
        );
        assert!(final_result.is_err());
        assert!(index.get_path(Path::new("src/value"), 0).is_none());

        let ancestor_root = tempfile::tempdir().unwrap();
        fs::create_dir(ancestor_root.path().join("src")).unwrap();
        fs::write(ancestor_root.path().join("src/value"), "inside").unwrap();
        let repository = repo(ancestor_root.path());
        let owner = WorkspaceVcsStageOwner::new(ancestor_root.path()).unwrap();
        let mut index = repository.index().unwrap();
        let ancestor_result = owner.stage_with_hook(
            &repository,
            &mut index,
            "",
            Path::new("src"),
            || {
                fs::rename(
                    ancestor_root.path().join("src"),
                    ancestor_root.path().join("owned"),
                )
                .unwrap();
                std::os::unix::fs::symlink(outside.path(), ancestor_root.path().join("src"))
                    .unwrap();
            },
        );
        assert!(ancestor_result.is_err());
        let entry = index.get_path(Path::new("src/value"), 0).unwrap();
        assert_eq!(repository.find_blob(entry.id).unwrap().content(), b"inside");
    }

    #[cfg(unix)]
    #[test]
    fn pinned_workspace_root_swap_is_fenced_after_safe_traversal() {
        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("value"), "inside").unwrap();
        fs::write(outside.path().join("value"), "outside").unwrap();
        let repository = repo(&root);
        let owner = WorkspaceVcsStageOwner::new(&root).unwrap();
        let mut index = repository.index().unwrap();
        let result = owner.stage_with_hook(
            &repository,
            &mut index,
            "",
            Path::new(""),
            || {
                fs::rename(&root, parent.path().join("owned")).unwrap();
                std::os::unix::fs::symlink(outside.path(), &root).unwrap();
            },
        );
        assert!(result.is_err());
        assert!(index.get_path(Path::new("value"), 0).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn refuses_final_and_ancestor_junction_swaps() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(outside.path().join("secret"), "outside").unwrap();
        let repository = repo(root.path());
        let owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        let mut index = repository.index().unwrap();
        junction::create(outside.path(), root.path().join("src/link")).unwrap();
        assert!(owner.stage(&repository, &mut index, "", Path::new("src/link")).is_err());
        junction::delete(root.path().join("src/link")).unwrap();
        fs::rename(root.path().join("src"), root.path().join("owned")).unwrap();
        junction::create(outside.path(), root.path().join("src")).unwrap();
        assert!(owner.stage(&repository, &mut index, "", Path::new("src/secret")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn pinned_traversal_never_reads_outside_during_junction_swap_race() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/value"), "inside").unwrap();
        fs::write(outside.path().join("value"), "outside").unwrap();
        let repository = repo(root.path());
        let owner = WorkspaceVcsStageOwner::new(root.path()).unwrap();
        let mut index = repository.index().unwrap();
        let swapped = std::sync::atomic::AtomicBool::new(false);
        let result = owner.stage_with_hook(
            &repository,
            &mut index,
            "",
            Path::new("src"),
            || {
                if fs::rename(root.path().join("src"), root.path().join("owned")).is_ok() {
                    junction::create(outside.path(), root.path().join("src")).unwrap();
                    swapped.store(true, std::sync::atomic::Ordering::Release);
                }
            },
        );
        if swapped.load(std::sync::atomic::Ordering::Acquire) {
            assert!(result.is_err());
        } else {
            // Windows denied the replacement while a pinned directory handle
            // was live; the safe original traversal completes.
            result.unwrap();
        }
        let entry = index.get_path(Path::new("src/value"), 0).unwrap();
        assert_eq!(repository.find_blob(entry.id).unwrap().content(), b"inside");
    }
}
