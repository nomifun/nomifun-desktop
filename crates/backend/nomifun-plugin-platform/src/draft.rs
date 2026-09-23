use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use fs2::FileExt as _;
use nomifun_agent_contracts::{PluginDraftId, PLUGIN_MANIFEST_PATH};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::ImportCancellation;

const MAX_DRAFT_FILES: usize = 4_096;
const MAX_DRAFT_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DRAFT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PluginDraftStoreError {
    #[error("unsafe Plugin Draft path {path}: {reason}")]
    UnsafePath { path: PathBuf, reason: String },
    #[error("Plugin Draft was not found")]
    NotFound,
    #[error("Plugin Draft replacement was canceled")]
    Canceled,
    #[error("Plugin Draft limit exceeded: {0}")]
    Limit(String),
    #[error("Plugin Draft filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type PluginDraftStoreResult<T> = Result<T, PluginDraftStoreError>;

#[derive(Clone, Debug)]
pub struct PluginDraftStore {
    root: PathBuf,
}

pub struct StagedPluginDraftReplacement {
    lock: Option<File>,
    owner_root: PathBuf,
    draft_path: PathBuf,
    staged_path: PathBuf,
    backup_path: PathBuf,
    published: bool,
    completed: bool,
}

impl StagedPluginDraftReplacement {
    pub fn publish(&mut self) -> PluginDraftStoreResult<()> {
        if self.published || self.completed {
            return Err(unsafe_path(
                &self.draft_path,
                "Draft replacement has already advanced",
            ));
        }
        require_directory(&self.owner_root, &self.draft_path)?;
        if self.backup_path.exists() {
            return Err(unsafe_path(
                &self.backup_path,
                "Draft replacement backup already exists",
            ));
        }
        fs::rename(&self.draft_path, &self.backup_path)
            .map_err(|source| io_error(&self.draft_path, source))?;
        if let Err(source) = fs::rename(&self.staged_path, &self.draft_path) {
            let restore = fs::rename(&self.backup_path, &self.draft_path);
            return match restore {
                Ok(()) => Err(io_error(&self.draft_path, source)),
                Err(restore_error) => Err(unsafe_path(
                    &self.draft_path,
                    &format!(
                        "staged Draft publish failed ({source}) and backup restore failed ({restore_error})"
                    ),
                )),
            };
        }
        self.published = true;
        sync_directory(&self.owner_root)?;
        Ok(())
    }

    pub fn commit(mut self) -> PluginDraftStoreResult<()> {
        if !self.published || self.completed {
            return Err(unsafe_path(
                &self.draft_path,
                "Draft replacement is not published",
            ));
        }
        self.completed = true;
        let result = validate_removal_tree(&self.backup_path)
            .and_then(|()| {
                fs::remove_dir_all(&self.backup_path)
                    .map_err(|source| io_error(&self.backup_path, source))
            })
            .and_then(|()| sync_directory(&self.owner_root));
        self.unlock();
        result
    }

    pub fn rollback(mut self) -> PluginDraftStoreResult<()> {
        self.rollback_inner()?;
        self.completed = true;
        self.unlock();
        Ok(())
    }

    fn rollback_inner(&mut self) -> PluginDraftStoreResult<()> {
        if self.completed {
            return Ok(());
        }
        if self.published {
            validate_removal_tree(&self.draft_path)?;
            fs::rename(&self.draft_path, &self.staged_path)
                .map_err(|source| io_error(&self.draft_path, source))?;
            if let Err(source) = fs::rename(&self.backup_path, &self.draft_path) {
                let restore = fs::rename(&self.staged_path, &self.draft_path);
                return match restore {
                    Ok(()) => Err(io_error(&self.draft_path, source)),
                    Err(restore_error) => Err(unsafe_path(
                        &self.draft_path,
                        &format!(
                            "Draft rollback failed ({source}) and generated-tree restore failed ({restore_error})"
                        ),
                    )),
                };
            }
            self.published = false;
            validate_removal_tree(&self.staged_path)?;
            fs::remove_dir_all(&self.staged_path)
                .map_err(|source| io_error(&self.staged_path, source))?;
        } else if self.staged_path.exists() {
            validate_removal_tree(&self.staged_path)?;
            fs::remove_dir_all(&self.staged_path)
                .map_err(|source| io_error(&self.staged_path, source))?;
        }
        sync_directory(&self.owner_root)
    }

    fn unlock(&mut self) {
        if let Some(lock) = self.lock.take() {
            let _ = lock.unlock();
        }
    }
}

impl Drop for StagedPluginDraftReplacement {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.rollback_inner();
        }
        self.unlock();
    }
}

impl PluginDraftStore {
    pub fn new(root: impl AsRef<Path>) -> PluginDraftStoreResult<Self> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(unsafe_path(root, "Draft root must be absolute"));
        }
        ensure_directory(root)?;
        Ok(Self {
            root: fs::canonicalize(root).map_err(|source| io_error(root, source))?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<PathBuf> {
        validate_uuid(owner_user_id, "owner")?;
        validate_uuid(draft_id.as_ref(), "Draft")?;
        let owner = self.root.join(owner_user_id);
        ensure_direct_child(&self.root, &owner)?;
        ensure_directory(&owner)?;
        let _lock = self.acquire_lock(owner_user_id, draft_id)?;
        let draft = owner.join(draft_id.as_ref());
        match fs::create_dir(&draft) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(PluginDraftStoreError::UnsafePath {
                    path: draft,
                    reason: "Draft already exists".into(),
                });
            }
            Err(source) => return Err(io_error(&draft, source)),
        }
        sync_directory(&owner)?;
        Ok(fs::canonicalize(&draft).map_err(|source| io_error(&draft, source))?)
    }

    pub fn open(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<PathBuf> {
        validate_uuid(owner_user_id, "owner")?;
        validate_uuid(draft_id.as_ref(), "Draft")?;
        let owner = self.root.join(owner_user_id);
        let draft = owner.join(draft_id.as_ref());
        require_directory(&self.root, &owner)?;
        require_directory(&owner, &draft)?;
        Ok(fs::canonicalize(&draft).map_err(|source| io_error(&draft, source))?)
    }

    pub fn write(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
        relative_path: &str,
        bytes: &[u8],
    ) -> PluginDraftStoreResult<()> {
        let _lock = self.acquire_lock(owner_user_id, draft_id)?;
        self.write_unlocked(owner_user_id, draft_id, relative_path, bytes)
    }

    fn write_unlocked(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
        relative_path: &str,
        bytes: &[u8],
    ) -> PluginDraftStoreResult<()> {
        if bytes.len() as u64 > MAX_DRAFT_FILE_BYTES {
            return Err(PluginDraftStoreError::Limit(format!(
                "file exceeds {MAX_DRAFT_FILE_BYTES} bytes"
            )));
        }
        let root = self.open(owner_user_id, draft_id)?;
        let normalized = normalize_package_path(relative_path)?;
        validate_package_file(&normalized)?;
        let destination = join_normalized(&root, &normalized);
        ensure_parents(&root, destination.parent().expect("package file has parent"))?;
        if let Ok(metadata) = fs::symlink_metadata(&destination)
            && (is_link(&metadata) || !metadata.is_file())
        {
            return Err(unsafe_path(&destination, "destination is not a regular file"));
        }
        let parent = destination.parent().expect("package file has parent");
        let temporary = parent.join(format!(".draft-write-{}", Uuid::now_v7()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error(&temporary, source))?;
            file.write_all(bytes)
                .map_err(|source| io_error(&temporary, source))?;
            file.sync_all()
                .map_err(|source| io_error(&temporary, source))?;
            drop(file);
            fs::rename(&temporary, &destination)
                .map_err(|source| io_error(&destination, source))?;
            sync_directory(parent)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn delete_file(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
        relative_path: &str,
    ) -> PluginDraftStoreResult<bool> {
        let _lock = self.acquire_lock(owner_user_id, draft_id)?;
        let root = self.open(owner_user_id, draft_id)?;
        let normalized = normalize_package_path(relative_path)?;
        validate_package_file(&normalized)?;
        let path = join_normalized(&root, &normalized);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(io_error(&path, source)),
        };
        if is_link(&metadata) || !metadata.is_file() {
            return Err(unsafe_path(&path, "refusing to delete a non-regular file"));
        }
        require_within(&root, &path)?;
        fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
        sync_directory(path.parent().expect("Draft file has parent"))?;
        Ok(true)
    }

    pub fn freeze(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<BTreeMap<String, Vec<u8>>> {
        let _lock = self.acquire_lock(owner_user_id, draft_id)?;
        self.freeze_unlocked(owner_user_id, draft_id)
    }

    fn freeze_unlocked(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<BTreeMap<String, Vec<u8>>> {
        let root = self.open(owner_user_id, draft_id)?;
        let mut output = BTreeMap::new();
        let mut collision_keys = HashSet::new();
        let mut total = 0u64;
        for entry in WalkDir::new(&root).follow_links(false).min_depth(1) {
            let entry = entry.map_err(|error| PluginDraftStoreError::Io {
                path: error.path().unwrap_or(&root).to_path_buf(),
                source: error
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("cannot scan Draft")),
            })?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|source| io_error(entry.path(), source))?;
            if is_link(&metadata) {
                return Err(unsafe_path(entry.path(), "Draft contains a link or reparse point"));
            }
            let relative = entry.path().strip_prefix(&root).map_err(|_| {
                unsafe_path(entry.path(), "Draft entry escaped its workspace")
            })?;
            let normalized = normalize_path(relative)?;
            if !collision_keys.insert(windows_key(&normalized)?) {
                return Err(unsafe_path(entry.path(), "Draft contains colliding paths"));
            }
            if metadata.is_dir() {
                validate_package_directory(&normalized)?;
                continue;
            }
            if !metadata.is_file() {
                return Err(unsafe_path(entry.path(), "Draft contains a special file"));
            }
            validate_package_file(&normalized)?;
            if output.len() >= MAX_DRAFT_FILES || metadata.len() > MAX_DRAFT_FILE_BYTES {
                return Err(PluginDraftStoreError::Limit("Draft file inventory exceeds limits".into()));
            }
            total = total.saturating_add(metadata.len());
            if total > MAX_DRAFT_BYTES {
                return Err(PluginDraftStoreError::Limit("Draft exceeds total byte limit".into()));
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            File::open(entry.path())
                .map_err(|source| io_error(entry.path(), source))?
                .take(MAX_DRAFT_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|source| io_error(entry.path(), source))?;
            if bytes.len() as u64 != metadata.len() {
                return Err(unsafe_path(entry.path(), "Draft file changed while freezing"));
            }
            output.insert(normalized, bytes);
        }
        if !output.contains_key(PLUGIN_MANIFEST_PATH) {
            return Err(PluginDraftStoreError::UnsafePath {
                path: root,
                reason: "nomifun.plugin.json is required".into(),
            });
        }
        Ok(output)
    }

    pub fn stage_exact_replacement(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
        files: &BTreeMap<String, Vec<u8>>,
        cancellation: &dyn ImportCancellation,
    ) -> PluginDraftStoreResult<StagedPluginDraftReplacement> {
        let lock = self.acquire_lock(owner_user_id, draft_id)?;
        let draft_path = self.open(owner_user_id, draft_id)?;
        let owner_root = draft_path
            .parent()
            .ok_or_else(|| unsafe_path(&draft_path, "Draft has no owner directory"))?
            .to_path_buf();
        if files.is_empty() || files.len() > MAX_DRAFT_FILES {
            return Err(PluginDraftStoreError::Limit(
                "replacement file inventory exceeds limits".into(),
            ));
        }
        if !files.contains_key(PLUGIN_MANIFEST_PATH) {
            return Err(unsafe_path(
                &draft_path,
                "nomifun.plugin.json is required",
            ));
        }
        let replacement_id = Uuid::now_v7();
        let staged_path = owner_root.join(format!(".draft-stage-{replacement_id}"));
        let backup_path = owner_root.join(format!(".draft-backup-{replacement_id}"));
        validate_direct_child_path(&owner_root, &staged_path)?;
        validate_direct_child_path(&owner_root, &backup_path)?;
        fs::create_dir(&staged_path).map_err(|source| io_error(&staged_path, source))?;

        let staged = (|| {
            let mut collision_keys = HashSet::new();
            let mut total = 0u64;
            for (relative_path, bytes) in files {
                if cancellation.is_cancelled() {
                    return Err(PluginDraftStoreError::Canceled);
                }
                if bytes.len() as u64 > MAX_DRAFT_FILE_BYTES {
                    return Err(PluginDraftStoreError::Limit(format!(
                        "file exceeds {MAX_DRAFT_FILE_BYTES} bytes"
                    )));
                }
                total = total.saturating_add(bytes.len() as u64);
                if total > MAX_DRAFT_BYTES {
                    return Err(PluginDraftStoreError::Limit(
                        "replacement exceeds total byte limit".into(),
                    ));
                }
                let normalized = normalize_package_path(relative_path)?;
                validate_package_file(&normalized)?;
                if !collision_keys.insert(windows_key(&normalized)?) {
                    return Err(unsafe_path(
                        Path::new(relative_path),
                        "replacement contains colliding paths",
                    ));
                }
                let destination = join_normalized(&staged_path, &normalized);
                ensure_parents(
                    &staged_path,
                    destination
                        .parent()
                        .expect("replacement file has a parent"),
                )?;
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&destination)
                    .map_err(|source| io_error(&destination, source))?;
                file.write_all(bytes)
                    .map_err(|source| io_error(&destination, source))?;
                file.sync_all()
                    .map_err(|source| io_error(&destination, source))?;
                sync_directory(destination.parent().expect("replacement file has a parent"))?;
            }
            if cancellation.is_cancelled() {
                return Err(PluginDraftStoreError::Canceled);
            }
            sync_directory(&staged_path)
        })();
        if let Err(error) = staged {
            if staged_path.exists() {
                let _ = validate_removal_tree(&staged_path)
                    .and_then(|()| fs::remove_dir_all(&staged_path).map_err(|source| io_error(&staged_path, source)));
            }
            return Err(error);
        }

        Ok(StagedPluginDraftReplacement {
            lock: Some(lock),
            owner_root,
            draft_path,
            staged_path,
            backup_path,
            published: false,
            completed: false,
        })
    }

    pub fn discard(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<bool> {
        let _lock = self.acquire_lock(owner_user_id, draft_id)?;
        let draft = match self.open(owner_user_id, draft_id) {
            Ok(draft) => draft,
            Err(PluginDraftStoreError::NotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        let owner = draft
            .parent()
            .ok_or_else(|| unsafe_path(&draft, "Draft has no owner directory"))?;
        validate_removal_tree(&draft)?;
        fs::remove_dir_all(&draft).map_err(|source| io_error(&draft, source))?;
        sync_directory(owner)?;
        Ok(true)
    }

    fn acquire_lock(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginDraftStoreResult<File> {
        validate_uuid(owner_user_id, "owner")?;
        validate_uuid(draft_id.as_ref(), "Draft")?;
        let owner = self.root.join(owner_user_id);
        require_directory(&self.root, &owner)?;
        let lock_path = owner.join(format!(".draft-{}.lock", draft_id.as_ref()));
        validate_direct_child_path(&owner, &lock_path)?;
        match fs::symlink_metadata(&lock_path) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                return Err(unsafe_path(&lock_path, "Draft lock is not a regular file"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&lock_path, source)),
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| io_error(&lock_path, source))?;
        let metadata = fs::symlink_metadata(&lock_path).map_err(|source| io_error(&lock_path, source))?;
        if is_link(&metadata) || !metadata.is_file() {
            return Err(unsafe_path(&lock_path, "Draft lock became unsafe"));
        }
        file.lock_exclusive()
            .map_err(|source| io_error(&lock_path, source))?;
        Ok(file)
    }
}

fn validate_uuid(value: &str, label: &str) -> PluginDraftStoreResult<()> {
    let parsed = Uuid::parse_str(value)
        .map_err(|_| unsafe_path(Path::new(value), &format!("{label} ID is not a UUID")))?;
    if parsed.get_version_num() != 7 || parsed.to_string() != value {
        return Err(unsafe_path(
            Path::new(value),
            &format!("{label} ID must be canonical UUIDv7"),
        ));
    }
    Ok(())
}

fn normalize_package_path(value: &str) -> PluginDraftStoreResult<String> {
    normalize_path(Path::new(value))
}

fn normalize_path(path: &Path) -> PluginDraftStoreResult<String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(unsafe_path(path, "path must be non-empty and relative"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(value) = component else {
            return Err(unsafe_path(path, "traversal and absolute paths are forbidden"));
        };
        let value = value
            .to_str()
            .ok_or_else(|| unsafe_path(path, "path must be UTF-8"))?;
        if value.is_empty() || value.contains(['/', '\\', ':']) || value.nfc().ne(value.chars()) {
            return Err(unsafe_path(path, "path component is not portable normalized text"));
        }
        parts.push(value);
    }
    let normalized = parts.join("/");
    windows_key(&normalized)?;
    Ok(normalized)
}

fn windows_key(path: &str) -> PluginDraftStoreResult<String> {
    let mut output = Vec::new();
    for component in path.split('/') {
        let trimmed = component.trim_end_matches([' ', '.']);
        let stem = trimmed.split('.').next().unwrap_or(trimmed).to_ascii_uppercase();
        if trimmed != component
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4"
                    | "COM5" | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2"
                    | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
            )
        {
            return Err(unsafe_path(Path::new(path), "path is unsafe on Windows"));
        }
        output.push(trimmed.to_ascii_lowercase());
    }
    Ok(output.join("/"))
}

fn validate_package_file(path: &str) -> PluginDraftStoreResult<()> {
    if path == PLUGIN_MANIFEST_PATH
        || path == "service/main.mjs"
        || path.starts_with("ui/")
        || (path.starts_with("migrations/") && path.ends_with(".mjs"))
        || path.starts_with("source/")
    {
        Ok(())
    } else {
        Err(unsafe_path(Path::new(path), "file is outside the Plugin Package layout"))
    }
}

fn validate_package_directory(path: &str) -> PluginDraftStoreResult<()> {
    if ["ui", "service", "migrations", "source"]
        .into_iter()
        .any(|root| path == root || path.starts_with(&format!("{root}/")))
    {
        Ok(())
    } else {
        Err(unsafe_path(Path::new(path), "directory is outside the Plugin Package layout"))
    }
}

fn join_normalized(root: &Path, path: &str) -> PathBuf {
    path.split('/').fold(root.to_path_buf(), |mut output, part| {
        output.push(part);
        output
    })
}

fn ensure_parents(root: &Path, parent: &Path) -> PluginDraftStoreResult<()> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| unsafe_path(parent, "parent escaped Draft"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(unsafe_path(parent, "parent contains traversal"));
        };
        current.push(component);
        if current.exists() {
            require_directory(root, &current)?;
        } else {
            fs::create_dir(&current).map_err(|source| io_error(&current, source))?;
        }
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> PluginDraftStoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !is_link(&metadata) && metadata.is_dir() => Ok(()),
        Ok(_) => Err(unsafe_path(path, "managed path is not a real directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
            let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
            if is_link(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(path, "managed path became unsafe"));
            }
            Ok(())
        }
        Err(source) => Err(io_error(path, source)),
    }
}

fn validate_direct_child_path(parent: &Path, child: &Path) -> PluginDraftStoreResult<()> {
    if child.parent() != Some(parent) {
        return Err(unsafe_path(child, "path is not a direct child"));
    }
    Ok(())
}

fn ensure_direct_child(parent: &Path, child: &Path) -> PluginDraftStoreResult<()> {
    validate_direct_child_path(parent, child)?;
    match fs::create_dir(child) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            require_directory(parent, child)
        }
        Err(source) => Err(io_error(child, source)),
    }
}

fn require_directory(root: &Path, path: &Path) -> PluginDraftStoreResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            PluginDraftStoreError::NotFound
        } else {
            io_error(path, source)
        }
    })?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(unsafe_path(path, "path is not a real directory"));
    }
    require_within(root, path)
}

fn require_within(root: &Path, path: &Path) -> PluginDraftStoreResult<()> {
    let root = fs::canonicalize(root).map_err(|source| io_error(root, source))?;
    let path = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    if !path.starts_with(&root) {
        return Err(unsafe_path(&path, "path escaped its managed root"));
    }
    Ok(())
}

fn validate_removal_tree(root: &Path) -> PluginDraftStoreResult<()> {
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| PluginDraftStoreError::Io {
            path: error.path().unwrap_or(root).to_path_buf(),
            source: error
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("cannot validate Draft removal")),
        })?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|source| io_error(entry.path(), source))?;
        if is_link(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(unsafe_path(entry.path(), "refusing to remove an unsafe tree"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn sync_directory(path: &Path) -> PluginDraftStoreResult<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| io_error(path, source))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn unsafe_path(path: &Path, reason: &str) -> PluginDraftStoreError {
    PluginDraftStoreError::UnsafePath {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> PluginDraftStoreError {
    PluginDraftStoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}
