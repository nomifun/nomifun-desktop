use std::ffi::{OsStr, OsString};
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

pub struct WorkspaceVcsStageOwner {
    root: PathBuf,
    root_identity: SameFileHandle,
    dir: Arc<Dir>,
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

    pub fn stage(
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
                for path in indexed_paths_for_target(index, &repo_target)? {
                    index.remove_path(&path).map_err(git_error)?;
                }
                let mut staged = Vec::new();
                collect_directory(repository, index, &directory, &repo_target, &mut staged)?;
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
                Ok(staged)
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
                Ok(vec![path])
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
                Ok(removed)
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
        }
    }
    Ok(output)
}

fn join_repo_path(prefix: &str, path: &str) -> String { match (prefix.is_empty(), path.is_empty()) { (true, _) => path.into(), (_, true) => prefix.into(), _ => format!("{prefix}/{path}") } }
fn portable(path: &Path) -> Result<String, AppError> { path.components().map(|component| match component { Component::Normal(value) => value.to_str().map(str::to_owned).ok_or_else(|| AppError::BadRequest("VCS path is not UTF-8".into())), _ => Err(AppError::BadRequest("VCS path is not normalized".into())) }).collect::<Result<Vec<_>, _>>().map(|parts| parts.join("/")) }
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
        let mut index = repository.index().unwrap();
        owner.stage(&repository, &mut index, "", Path::new("src")).unwrap();
        let entry = index.get_path(Path::new("src/lib.rs"), 0).unwrap();
        assert_eq!(repository.find_blob(entry.id).unwrap().content(), b"safe");
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
        owner
            .stage(&repository, &mut index, "", Path::new(""))
            .unwrap();
        assert!(index.get_path(Path::new("deleted.txt"), 0).is_none());
        assert!(
            index
                .iter()
                .all(|entry| !entry.path.starts_with(b".nomifun/"))
        );
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
