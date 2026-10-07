//! Generation-based storage owned by one Unified Plugin instance.
//!
//! The core database stores only the name of the authoritative generation.
//! This module materializes complete generations before that pointer is
//! changed; it never decides which generation is active. Callers must hold the
//! Plugin mutation mutex while cloning, publishing, or deleting generations.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nomifun_agent_contracts::{DigestHex, PluginId};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Value as SqliteValue, ValueRef};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde_json::Value as JsonValue;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

const GENERATIONS_DIRECTORY: &str = "generations";
const STAGING_DIRECTORY: &str = "staging";
const DATA_FILE: &str = "data.sqlite";
const FILES_DIRECTORY: &str = "files";
const KV_TABLE: &str = "_nomifun_kv";
const MIGRATIONS_TABLE: &str = "_nomifun_migrations";
const RESERVED_PREFIX: &str = "_nomifun_";
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_RELATIVE_PATH_BYTES: usize = 1_024;
const MAX_FILE_READ_BYTES: u64 = 64 * 1024 * 1024;
const MAX_KV_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_DATABASE_BATCH: usize = 64;
const MAX_DATABASE_ROWS: usize = 10_000;
const MAX_DATABASE_RESULT_BYTES: usize = 4 * 1024 * 1024;

pub type PluginDataRootResult<T> = Result<T, PluginDataRootError>;

#[derive(Debug, Error)]
pub enum PluginDataRootError {
    #[error("invalid {label}: {value}")]
    InvalidIdentity { label: &'static str, value: String },
    #[error("Plugin DataRoot path is unsafe: {path} ({reason})")]
    UnsafePath { path: PathBuf, reason: String },
    #[error("Plugin DataRoot already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("Plugin DataRoot was not found: {0}")]
    NotFound(PathBuf),
    #[error("Plugin DataRoot handle has the wrong kind")]
    WrongRootKind,
    #[error("invalid Plugin storage request: {0}")]
    InvalidRequest(String),
    #[error("Plugin storage limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("Plugin DataRoot I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Plugin SQLite operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Plugin JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataGeneration(String);

impl DataGeneration {
    pub fn new(value: impl Into<String>) -> PluginDataRootResult<Self> {
        let value = value.into();
        validate_component(&value, "data generation")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DataGeneration {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataRootKind {
    Generation,
    Staging,
    Preview,
}

#[derive(Clone, Debug)]
pub struct PluginDataRootHandle {
    plugin_id: PluginId,
    generation: DataGeneration,
    path: PathBuf,
    managed_root: Arc<PathBuf>,
    kind: DataRootKind,
    cache: MemoryPluginCache,
    cache_scope: String,
}

impl PluginDataRootHandle {
    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn generation(&self) -> &DataGeneration {
        &self.generation
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> DataRootKind {
        self.kind
    }

    pub fn storage(&self) -> PluginStorage {
        PluginStorage { root: self.clone() }
    }

    pub fn cache(&self) -> ScopedPluginCache {
        ScopedPluginCache {
            cache: self.cache.clone(),
            scope: self.cache_scope.clone(),
        }
    }

    fn database_path(&self) -> PathBuf {
        self.path.join(DATA_FILE)
    }

    fn files_path(&self) -> PathBuf {
        self.path.join(FILES_DIRECTORY)
    }

    fn verify(&self) -> PluginDataRootResult<()> {
        require_real_directory(&self.path)?;
        let canonical = canonicalize(&self.path)?;
        if canonical != self.path || !canonical.starts_with(self.managed_root.as_path()) {
            return Err(unsafe_path(
                &self.path,
                "the generation root changed or escaped its managed root",
            ));
        }
        require_regular_file(&self.database_path())?;
        require_real_directory(&self.files_path())?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct PluginDataRootManager {
    root: Arc<PathBuf>,
    cache: MemoryPluginCache,
}

impl PluginDataRootManager {
    /// `root` is the canonical `plugin-data` directory, not a specific Plugin.
    pub fn new(root: impl AsRef<Path>) -> PluginDataRootResult<Self> {
        let requested = root.as_ref();
        if !requested.is_absolute() {
            return Err(unsafe_path(requested, "managed root must be absolute"));
        }
        ensure_real_directory(requested)?;
        let root = canonicalize(requested)?;
        Ok(Self {
            root: Arc::new(root),
            cache: MemoryPluginCache::default(),
        })
    }

    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    pub fn stage_empty(
        &self,
        plugin_id: PluginId,
        generation: DataGeneration,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        let layout = self.ensure_layout(&plugin_id)?;
        let path = layout.staging.join(generation.as_str());
        create_exact_directory(&layout.staging, &path)?;
        let result = (|| {
            ensure_real_directory(&path.join(FILES_DIRECTORY))?;
            initialize_database(&path.join(DATA_FILE))?;
            sync_tree(&path)?;
            self.handle(plugin_id, generation, path.clone(), DataRootKind::Staging)
        })();
        if result.is_err() {
            let _ = remove_owned_tree(&layout.staging, &path);
        }
        result
    }

    pub fn stage_clone(
        &self,
        source: &PluginDataRootHandle,
        generation: DataGeneration,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        if source.kind != DataRootKind::Generation {
            return Err(PluginDataRootError::WrongRootKind);
        }
        source.verify()?;
        let layout = self.ensure_layout(&source.plugin_id)?;
        let path = layout.staging.join(generation.as_str());
        create_exact_directory(&layout.staging, &path)?;
        let result = (|| {
            ensure_real_directory(&path.join(FILES_DIRECTORY))?;
            snapshot_database(&source.database_path(), &path.join(DATA_FILE))?;
            copy_files_tree(&source.files_path(), &path.join(FILES_DIRECTORY))?;
            initialize_database(&path.join(DATA_FILE))?;
            sync_tree(&path)?;
            self.handle(
                source.plugin_id.clone(),
                generation,
                path.clone(),
                DataRootKind::Staging,
            )
        })();
        if result.is_err() {
            let _ = remove_owned_tree(&layout.staging, &path);
        }
        result
    }

    /// Stage a verified Backup payload through the same DataRoot layout used
    /// by installs and migrations. The caller still owns publication; a
    /// failed validation can discard this handle without changing any active
    /// generation.
    pub fn stage_import(
        &self,
        plugin_id: PluginId,
        generation: DataGeneration,
        data_sqlite: &[u8],
        files: &BTreeMap<String, Vec<u8>>,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        if data_sqlite.is_empty() {
            return Err(PluginDataRootError::InvalidRequest(
                "Backup data.sqlite is empty".into(),
            ));
        }
        let staged = self.stage_empty(plugin_id, generation)?;
        let result = (|| {
            staged.verify()?;
            let database = staged.database_path();
            require_regular_file(&database)?;
            let mut output = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&database)
                .map_err(|source| io_error(&database, source))?;
            output
                .write_all(data_sqlite)
                .map_err(|source| io_error(&database, source))?;
            output
                .sync_all()
                .map_err(|source| io_error(&database, source))?;
            initialize_database(&database)?;
            let storage = staged.storage();
            for (path, bytes) in files {
                storage.file_write(path, bytes)?;
            }
            sync_tree(staged.path())?;
            staged.verify()?;
            Ok(staged.clone())
        })();
        if result.is_err() {
            let _ = self.discard_staging(staged);
        }
        result
    }

    pub fn publish(
        &self,
        staged: PluginDataRootHandle,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        if staged.kind != DataRootKind::Staging {
            return Err(PluginDataRootError::WrongRootKind);
        }
        staged.verify()?;
        let layout = self.ensure_layout(&staged.plugin_id)?;
        if staged.path.parent() != Some(layout.staging.as_path()) {
            return Err(unsafe_path(&staged.path, "staging handle is not an exact child"));
        }
        let destination = layout.generations.join(staged.generation.as_str());
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(PluginDataRootError::AlreadyExists(destination));
        }
        sync_tree(&staged.path)?;
        fs::rename(&staged.path, &destination).map_err(|source| io_error(&destination, source))?;
        sync_directory(&layout.generations)?;
        let canonical = canonicalize(&destination)?;
        self.cache.clear_scope(&staged.cache_scope);
        self.handle(
            staged.plugin_id,
            staged.generation,
            canonical,
            DataRootKind::Generation,
        )
    }

    pub fn create_empty_generation(
        &self,
        plugin_id: PluginId,
        generation: DataGeneration,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        self.publish(self.stage_empty(plugin_id, generation)?)
    }

    pub fn open_generation(
        &self,
        plugin_id: PluginId,
        generation: DataGeneration,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        let layout = self.layout(&plugin_id, false)?;
        let path = layout.generations.join(generation.as_str());
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PluginDataRootError::NotFound(path.clone())
            } else {
                io_error(&path, source)
            }
        })?;
        if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(unsafe_path(&path, "generation is not a real directory"));
        }
        let path = canonicalize(&path)?;
        self.handle(plugin_id, generation, path, DataRootKind::Generation)
    }

    pub fn discard_staging(&self, staged: PluginDataRootHandle) -> PluginDataRootResult<bool> {
        if staged.kind != DataRootKind::Staging {
            return Err(PluginDataRootError::WrongRootKind);
        }
        let layout = self.layout(&staged.plugin_id, false)?;
        if staged.path.parent() != Some(layout.staging.as_path()) {
            return Err(unsafe_path(&staged.path, "staging handle is not an exact child"));
        }
        let removed = remove_owned_tree(&layout.staging, &staged.path)?;
        self.cache.clear_scope(&staged.cache_scope);
        Ok(removed)
    }

    /// Remove one exact unpublished generation left by an interrupted
    /// install. This never scans outside the named Plugin's `staging`
    /// directory and is therefore safe to replay during journal recovery.
    pub fn delete_staging_exact(
        &self,
        plugin_id: &PluginId,
        generation: &DataGeneration,
    ) -> PluginDataRootResult<bool> {
        let layout = match self.layout(plugin_id, false) {
            Ok(layout) => layout,
            Err(PluginDataRootError::NotFound(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let target = layout.staging.join(generation.as_str());
        let removed = remove_owned_tree(&layout.staging, &target)?;
        self.cache.clear_scope(&format!(
            "staging:{}/{}",
            plugin_id.as_ref(),
            generation.as_str()
        ));
        Ok(removed)
    }

    pub fn delete_generation_exact(
        &self,
        plugin_id: &PluginId,
        generation: &DataGeneration,
    ) -> PluginDataRootResult<bool> {
        let layout = match self.layout(plugin_id, false) {
            Ok(layout) => layout,
            Err(PluginDataRootError::NotFound(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let target = layout.generations.join(generation.as_str());
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(io_error(&target, source)),
        };
        if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(unsafe_path(&target, "generation is not a real directory"));
        }
        validate_removal_tree(&target)?;
        let quarantine = layout
            .staging
            .join(format!(".delete-{}", Uuid::now_v7()));
        fs::rename(&target, &quarantine).map_err(|source| io_error(&target, source))?;
        sync_directory(&layout.generations)?;
        let removal = remove_owned_tree(&layout.staging, &quarantine);
        if removal.is_ok() {
            self.cache
                .clear_scope(&production_cache_scope(plugin_id));
        }
        removal
    }

    /// Remove every complete generation not referenced by the current and
    /// single Previous pointers. The exact Plugin directory is validated
    /// before enumeration; unexpected files or links fail closed.
    pub fn prune_generations(
        &self,
        plugin_id: &PluginId,
        keep: &BTreeSet<DataGeneration>,
    ) -> PluginDataRootResult<usize> {
        let layout = self.layout(plugin_id, false)?;
        let mut entries = fs::read_dir(&layout.generations)
            .map_err(|source| io_error(&layout.generations, source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| io_error(&layout.generations, source))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut removed = 0usize;
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(
                    &path,
                    "generation inventory contains a non-directory entry",
                ));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| unsafe_path(&path, "generation name is not UTF-8"))?;
            let generation = DataGeneration::new(name)?;
            if !keep.contains(&generation)
                && self.delete_generation_exact(plugin_id, &generation)?
            {
                removed = removed.checked_add(1).ok_or_else(|| {
                    PluginDataRootError::InvalidRequest(
                        "generation cleanup count overflow".into(),
                    )
                })?;
            }
        }
        Ok(removed)
    }

    /// Remove only orphaned Preview roots after a process restart. Preview
    /// sessions are never persisted, so every exact `preview-<uuidv7>` child
    /// under a managed staging directory is unreachable at boot. Install
    /// staging generations and current/Previous generations are untouched.
    pub fn cleanup_preview_roots(&self) -> PluginDataRootResult<usize> {
        require_real_directory(self.root())?;
        let mut plugins = fs::read_dir(self.root())
            .map_err(|source| io_error(self.root(), source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| io_error(self.root(), source))?;
        plugins.sort_by_key(std::fs::DirEntry::file_name);
        let mut removed = 0usize;
        for plugin in plugins {
            let plugin_path = plugin.path();
            let metadata = fs::symlink_metadata(&plugin_path)
                .map_err(|source| io_error(&plugin_path, source))?;
            if metadata_is_link_or_reparse(&metadata) {
                return Err(unsafe_path(
                    &plugin_path,
                    "plugin-data contains a link or reparse point",
                ));
            }
            if !metadata.is_dir() {
                continue;
            }
            let plugin_name = plugin
                .file_name()
                .to_str()
                .ok_or_else(|| unsafe_path(&plugin_path, "Plugin directory is not UTF-8"))?
                .to_owned();
            if nomifun_common::validate_uuidv7(&plugin_name).is_err() {
                continue;
            }
            let staging = plugin_path.join(STAGING_DIRECTORY);
            let staging_metadata = match fs::symlink_metadata(&staging) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(io_error(&staging, source)),
            };
            if metadata_is_link_or_reparse(&staging_metadata) || !staging_metadata.is_dir() {
                return Err(unsafe_path(&staging, "staging root is not a real directory"));
            }
            let mut entries = fs::read_dir(&staging)
                .map_err(|source| io_error(&staging, source))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| io_error(&staging, source))?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let name = entry
                    .file_name()
                    .to_str()
                    .ok_or_else(|| unsafe_path(&entry.path(), "staging entry is not UTF-8"))?
                    .to_owned();
                let Some(session_id) = name.strip_prefix("preview-") else {
                    continue;
                };
                nomifun_common::validate_uuidv7(session_id)
                    .map_err(|_| unsafe_path(&entry.path(), "Preview directory is not UUIDv7"))?;
                if remove_owned_tree(&staging, &entry.path())? {
                    removed = removed.checked_add(1).ok_or_else(|| {
                        PluginDataRootError::LimitExceeded(
                            "Preview cleanup count overflow".into(),
                        )
                    })?;
                }
            }
        }
        Ok(removed)
    }

    /// Permanently remove the complete DataRoot owned by one exact Plugin.
    /// The Plugin id is validated as a single path component and the entire
    /// tree is rejected if it contains a link, reparse point, or special file.
    /// No parent or sibling path can be selected by this operation.
    pub fn delete_plugin_exact(&self, plugin_id: &PluginId) -> PluginDataRootResult<bool> {
        validate_component(plugin_id.as_ref(), "plugin_id")?;
        require_real_directory(self.root())?;
        let target = self.root.join(plugin_id.as_ref());
        let removed = remove_owned_tree(self.root(), &target)?;
        self.cache.clear_plugin(plugin_id);
        Ok(removed)
    }

    pub fn create_empty_preview(
        &self,
        plugin_id: PluginId,
        session_id: &str,
    ) -> PluginDataRootResult<PreviewDataRoot> {
        self.create_preview_inner(plugin_id, session_id, None)
    }

    pub fn clone_preview(
        &self,
        source: &PluginDataRootHandle,
        session_id: &str,
    ) -> PluginDataRootResult<PreviewDataRoot> {
        if source.kind != DataRootKind::Generation {
            return Err(PluginDataRootError::WrongRootKind);
        }
        source.verify()?;
        self.create_preview_inner(source.plugin_id.clone(), session_id, Some(source))
    }

    /// Clone production data into a Draft-owned Preview identity. This keeps a
    /// Preview Service process separate from the installed Plugin process while
    /// both use the same storage adapter and copied bytes.
    pub fn clone_preview_for(
        &self,
        source: &PluginDataRootHandle,
        preview_plugin_id: PluginId,
        session_id: &str,
    ) -> PluginDataRootResult<PreviewDataRoot> {
        if source.kind != DataRootKind::Generation {
            return Err(PluginDataRootError::WrongRootKind);
        }
        source.verify()?;
        self.create_preview_inner(preview_plugin_id, session_id, Some(source))
    }

    fn create_preview_inner(
        &self,
        plugin_id: PluginId,
        session_id: &str,
        source: Option<&PluginDataRootHandle>,
    ) -> PluginDataRootResult<PreviewDataRoot> {
        validate_component(session_id, "preview session")?;
        let layout = self.ensure_layout(&plugin_id)?;
        let name = format!("preview-{session_id}");
        let generation = DataGeneration::new(name.clone())?;
        let path = layout.staging.join(&name);
        create_exact_directory(&layout.staging, &path)?;
        let result = (|| {
            ensure_real_directory(&path.join(FILES_DIRECTORY))?;
            if let Some(source) = source {
                snapshot_database(&source.database_path(), &path.join(DATA_FILE))?;
                copy_files_tree(&source.files_path(), &path.join(FILES_DIRECTORY))?;
            }
            initialize_database(&path.join(DATA_FILE))?;
            sync_tree(&path)?;
            let handle = self.handle(
                plugin_id,
                generation,
                path.clone(),
                DataRootKind::Preview,
            )?;
            Ok(PreviewDataRoot {
                handle: Some(handle),
                staging_parent: layout.staging.clone(),
            })
        })();
        if result.is_err() {
            let _ = remove_owned_tree(&layout.staging, &path);
        }
        result
    }

    fn handle(
        &self,
        plugin_id: PluginId,
        generation: DataGeneration,
        path: PathBuf,
        kind: DataRootKind,
    ) -> PluginDataRootResult<PluginDataRootHandle> {
        let cache_scope = match kind {
            DataRootKind::Generation => production_cache_scope(&plugin_id),
            DataRootKind::Staging => {
                format!("staging:{}/{}", plugin_id.as_ref(), generation.as_str())
            }
            DataRootKind::Preview => {
                format!("preview:{}/{}", plugin_id.as_ref(), generation.as_str())
            }
        };
        let handle = PluginDataRootHandle {
            plugin_id,
            generation,
            path,
            managed_root: Arc::clone(&self.root),
            kind,
            cache: self.cache.clone(),
            cache_scope,
        };
        handle.verify()?;
        Ok(handle)
    }

    fn ensure_layout(&self, plugin_id: &PluginId) -> PluginDataRootResult<PluginLayout> {
        self.layout(plugin_id, true)
    }

    fn layout(&self, plugin_id: &PluginId, create: bool) -> PluginDataRootResult<PluginLayout> {
        validate_component(plugin_id.as_ref(), "plugin_id")?;
        require_real_directory(self.root())?;
        let plugin = self.root.join(plugin_id.as_ref());
        if create {
            ensure_direct_child_directory(self.root(), &plugin)?;
        } else {
            require_real_directory(&plugin)?;
        }
        let plugin = canonicalize(&plugin)?;
        if plugin.parent() != Some(self.root.as_path()) {
            return Err(unsafe_path(&plugin, "Plugin root escaped plugin-data"));
        }
        let generations = plugin.join(GENERATIONS_DIRECTORY);
        let staging = plugin.join(STAGING_DIRECTORY);
        if create {
            ensure_direct_child_directory(&plugin, &generations)?;
            ensure_direct_child_directory(&plugin, &staging)?;
        } else {
            require_real_directory(&generations)?;
            require_real_directory(&staging)?;
        }
        Ok(PluginLayout {
            generations: canonicalize(&generations)?,
            staging: canonicalize(&staging)?,
        })
    }
}

struct PluginLayout {
    generations: PathBuf,
    staging: PathBuf,
}

pub struct PreviewDataRoot {
    handle: Option<PluginDataRootHandle>,
    staging_parent: PathBuf,
}

impl PreviewDataRoot {
    pub fn handle(&self) -> &PluginDataRootHandle {
        self.handle.as_ref().expect("live Preview DataRoot")
    }

    pub fn storage(&self) -> PluginStorage {
        self.handle().storage()
    }

    pub fn cache(&self) -> ScopedPluginCache {
        self.handle().cache()
    }

    pub fn destroy(mut self) -> PluginDataRootResult<()> {
        self.remove()
    }

    fn remove(&mut self) -> PluginDataRootResult<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        if handle.path.parent() != Some(self.staging_parent.as_path()) {
            return Err(unsafe_path(&handle.path, "Preview is not an exact staging child"));
        }
        remove_owned_tree(&self.staging_parent, &handle.path)?;
        handle.cache.clear_scope(&handle.cache_scope);
        Ok(())
    }
}

impl Drop for PreviewDataRoot {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryPluginCache {
    entries: Arc<Mutex<HashMap<(String, String), CacheEntry>>>,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    value: JsonValue,
    expires_at: Option<Instant>,
}

impl MemoryPluginCache {
    fn get(&self, scope: &str, key: &str) -> PluginDataRootResult<Option<JsonValue>> {
        validate_cache_key(key)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let identity = (scope.to_owned(), key.to_owned());
        if entries
            .get(&identity)
            .and_then(|entry| entry.expires_at)
            .is_some_and(|expires| expires <= Instant::now())
        {
            entries.remove(&identity);
            return Ok(None);
        }
        Ok(entries.get(&identity).map(|entry| entry.value.clone()))
    }

    fn set(
        &self,
        scope: &str,
        key: &str,
        value: JsonValue,
        ttl: Option<Duration>,
    ) -> PluginDataRootResult<()> {
        validate_cache_key(key)?;
        let expires_at = ttl.and_then(|ttl| Instant::now().checked_add(ttl));
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                (scope.to_owned(), key.to_owned()),
                CacheEntry { value, expires_at },
            );
        Ok(())
    }

    fn delete(&self, scope: &str, key: &str) -> PluginDataRootResult<bool> {
        validate_cache_key(key)?;
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(scope.to_owned(), key.to_owned()))
            .is_some())
    }

    fn clear_scope(&self, scope: &str) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(candidate, _), _| candidate != scope);
    }

    fn clear_plugin(&self, plugin_id: &PluginId) {
        let production = production_cache_scope(plugin_id);
        let staging = format!("staging:{}/", plugin_id.as_ref());
        let preview = format!("preview:{}/", plugin_id.as_ref());
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(scope, _), _| {
                scope != &production
                    && !scope.starts_with(&staging)
                    && !scope.starts_with(&preview)
            });
    }
}

#[derive(Clone, Debug)]
pub struct ScopedPluginCache {
    cache: MemoryPluginCache,
    scope: String,
}

impl ScopedPluginCache {
    pub fn get(&self, key: &str) -> PluginDataRootResult<Option<JsonValue>> {
        self.cache.get(&self.scope, key)
    }

    pub fn set(
        &self,
        key: &str,
        value: JsonValue,
        ttl: Option<Duration>,
    ) -> PluginDataRootResult<()> {
        self.cache.set(&self.scope, key, value, ttl)
    }

    pub fn delete(&self, key: &str) -> PluginDataRootResult<bool> {
        self.cache.delete(&self.scope, key)
    }

    pub fn clear(&self) {
        self.cache.clear_scope(&self.scope);
    }
}

#[derive(Clone, Debug)]
pub struct PluginStorage {
    root: PluginDataRootHandle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KvRead {
    pub value: Option<JsonValue>,
    /// Tombstones retain their revision. `None` means the key never existed.
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvCompareAndSwap {
    pub applied: bool,
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginSqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginSqlStatement {
    pub sql: String,
    pub parameters: Vec<PluginSqlValue>,
}

impl PluginSqlStatement {
    pub fn new(sql: impl Into<String>, parameters: Vec<PluginSqlValue>) -> Self {
        Self {
            sql: sql.into(),
            parameters,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<PluginSqlValue>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginExecuteResult {
    pub affected_rows: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginMigrationRecord {
    pub migration_id: String,
    pub migration_digest: DigestHex,
    pub from_version: u32,
    pub to_version: u32,
    pub applied_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFileEntry {
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: u64,
}

impl PluginStorage {
    pub fn root(&self) -> &PluginDataRootHandle {
        &self.root
    }

    pub fn kv_get(&self, key: &str) -> PluginDataRootResult<KvRead> {
        validate_kv_key(key)?;
        let connection = self.open_internal_database()?;
        read_kv(&connection, key)
    }

    pub fn kv_set(&self, key: &str, value: &JsonValue) -> PluginDataRootResult<u64> {
        validate_kv_key(key)?;
        let value_json = checked_json(value)?;
        let mut connection = self.open_internal_database()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_kv(&transaction, key)?;
        let revision = next_revision(current.revision)?;
        transaction.execute(
            &format!(
                "INSERT INTO {KV_TABLE} (key, value_json, revision, is_deleted)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json,
                     revision = excluded.revision, is_deleted = 0"
            ),
            rusqlite::params![key, value_json, revision_as_i64(revision)?],
        )?;
        transaction.commit()?;
        Ok(revision)
    }

    pub fn kv_delete(&self, key: &str) -> PluginDataRootResult<bool> {
        validate_kv_key(key)?;
        let mut connection = self.open_internal_database()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_kv_row(&transaction, key)?;
        let Some(current) = current else {
            transaction.commit()?;
            return Ok(false);
        };
        if current.deleted {
            transaction.commit()?;
            return Ok(false);
        }
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| PluginDataRootError::LimitExceeded("KV revision overflow".into()))?;
        transaction.execute(
            &format!(
                "UPDATE {KV_TABLE} SET value_json = NULL, revision = ?2, is_deleted = 1
                 WHERE key = ?1"
            ),
            rusqlite::params![key, revision_as_i64(revision)?],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// `expected_revision = None` matches only a key that has never existed.
    /// `value = None` performs a conditional delete.
    pub fn kv_compare_and_swap(
        &self,
        key: &str,
        expected_revision: Option<u64>,
        value: Option<&JsonValue>,
    ) -> PluginDataRootResult<KvCompareAndSwap> {
        validate_kv_key(key)?;
        let encoded = value.map(checked_json).transpose()?;
        let mut connection = self.open_internal_database()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read_kv_row(&transaction, key)?;
        let observed = current.as_ref().map(|row| row.revision);
        if observed != expected_revision {
            transaction.commit()?;
            return Ok(KvCompareAndSwap {
                applied: false,
                revision: observed,
            });
        }

        let revision = match (current.as_ref(), encoded) {
            (None, None) => None,
            (Some(row), None) if row.deleted => Some(row.revision),
            (Some(row), None) => {
                let next = row.revision.checked_add(1).ok_or_else(|| {
                    PluginDataRootError::LimitExceeded("KV revision overflow".into())
                })?;
                transaction.execute(
                    &format!(
                        "UPDATE {KV_TABLE} SET value_json = NULL, revision = ?2, is_deleted = 1
                         WHERE key = ?1"
                    ),
                    rusqlite::params![key, revision_as_i64(next)?],
                )?;
                Some(next)
            }
            (row, Some(value_json)) => {
                let next = next_revision(row.map(|row| row.revision))?;
                transaction.execute(
                    &format!(
                        "INSERT INTO {KV_TABLE} (key, value_json, revision, is_deleted)
                         VALUES (?1, ?2, ?3, 0)
                         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json,
                             revision = excluded.revision, is_deleted = 0"
                    ),
                    rusqlite::params![key, value_json, revision_as_i64(next)?],
                )?;
                Some(next)
            }
        };
        transaction.commit()?;
        Ok(KvCompareAndSwap {
            applied: true,
            revision,
        })
    }

    pub fn db_query(
        &self,
        statement: &PluginSqlStatement,
    ) -> PluginDataRootResult<PluginQueryResult> {
        validate_user_statement(statement)?;
        let connection = self.open_user_database()?;
        let mut prepared = connection.prepare(&statement.sql)?;
        if !prepared.readonly() {
            return Err(PluginDataRootError::InvalidRequest(
                "db.query requires a read-only statement".into(),
            ));
        }
        bind_parameters(&mut prepared, &statement.parameters)?;
        let columns = prepared
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let column_count = columns.len();
        let mut rows = prepared.raw_query();
        let mut output = Vec::new();
        let mut size = columns.iter().map(String::len).sum::<usize>();
        while let Some(row) = rows.next()? {
            if output.len() >= MAX_DATABASE_ROWS {
                return Err(PluginDataRootError::LimitExceeded(format!(
                    "query returned more than {MAX_DATABASE_ROWS} rows"
                )));
            }
            let mut values = Vec::with_capacity(column_count);
            for index in 0..column_count {
                let value = sql_value(row.get_ref(index)?);
                size = size.saturating_add(sql_value_size(&value));
                if size > MAX_DATABASE_RESULT_BYTES {
                    return Err(PluginDataRootError::LimitExceeded(format!(
                        "query result exceeded {MAX_DATABASE_RESULT_BYTES} bytes"
                    )));
                }
                values.push(value);
            }
            output.push(values);
        }
        Ok(PluginQueryResult {
            columns,
            rows: output,
        })
    }

    pub fn db_execute(
        &self,
        statement: &PluginSqlStatement,
    ) -> PluginDataRootResult<PluginExecuteResult> {
        validate_user_statement(statement)?;
        let connection = self.open_user_database()?;
        execute_user_statement(&connection, statement)
    }

    pub fn db_batch(
        &self,
        statements: &[PluginSqlStatement],
    ) -> PluginDataRootResult<Vec<PluginExecuteResult>> {
        if statements.is_empty() || statements.len() > MAX_DATABASE_BATCH {
            return Err(PluginDataRootError::InvalidRequest(format!(
                "db.batch requires 1..={MAX_DATABASE_BATCH} statements"
            )));
        }
        for statement in statements {
            validate_user_statement(statement)?;
        }
        let mut connection = self.open_user_database()?;
        let authorization = install_user_authorizer(&connection);
        authorization.allow_transactions.store(true, Ordering::Release);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = (|| {
            let mut output = Vec::with_capacity(statements.len());
            for statement in statements {
                output.push(execute_user_statement(&transaction, statement)?);
            }
            Ok(output)
        })();
        match result {
            Ok(output) => {
                transaction.commit()?;
                authorization.allow_transactions.store(false, Ordering::Release);
                Ok(output)
            }
            Err(error) => {
                drop(transaction);
                authorization.allow_transactions.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    pub fn migrations(&self) -> PluginDataRootResult<Vec<PluginMigrationRecord>> {
        let connection = self.open_internal_database()?;
        let mut statement = connection.prepare(&format!(
            "SELECT migration_id, migration_digest, from_version, to_version, applied_at_ms
             FROM {MIGRATIONS_TABLE} ORDER BY ordinal"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok(PluginMigrationRecord {
                migration_id: row.get(0)?,
                migration_digest: DigestHex::from(row.get::<_, String>(1)?),
                from_version: row.get(2)?,
                to_version: row.get(3)?,
                applied_at_ms: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Record a successfully executed migration. Reusing an ID with another
    /// digest or version edge is rejected.
    pub fn record_migration(&self, record: &PluginMigrationRecord) -> PluginDataRootResult<()> {
        validate_migration_record(record)?;
        let mut connection = self.open_internal_database()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                &format!(
                    "SELECT migration_digest, from_version, to_version FROM {MIGRATIONS_TABLE}
                     WHERE migration_id = ?1"
                ),
                [&record.migration_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((digest, from, to)) = existing {
            if digest != record.migration_digest.as_ref()
                || from != record.from_version
                || to != record.to_version
            {
                return Err(PluginDataRootError::InvalidRequest(format!(
                    "migration {} was already recorded with different content",
                    record.migration_id
                )));
            }
            transaction.commit()?;
            return Ok(());
        }
        let expected_from: u32 = transaction.query_row(
            &format!("SELECT COALESCE(MAX(to_version), 0) FROM {MIGRATIONS_TABLE}"),
            [],
            |row| row.get(0),
        )?;
        if record.from_version != expected_from || record.to_version != expected_from + 1 {
            return Err(PluginDataRootError::InvalidRequest(format!(
                "migration {} does not continue dataVersion {}",
                record.migration_id, expected_from
            )));
        }
        transaction.execute(
            &format!(
                "INSERT INTO {MIGRATIONS_TABLE}
                 (migration_id, migration_digest, from_version, to_version, applied_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            rusqlite::params![
                record.migration_id,
                record.migration_digest.as_ref(),
                record.from_version,
                record.to_version,
                record.applied_at_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn file_read(&self, relative_path: &str) -> PluginDataRootResult<Vec<u8>> {
        self.root.verify()?;
        let root = self.root.files_path();
        let path = resolve_existing_path(&root, relative_path)?;
        require_regular_file(&path)?;
        let metadata = fs::metadata(&path).map_err(|source| io_error(&path, source))?;
        if metadata.len() > MAX_FILE_READ_BYTES {
            return Err(PluginDataRootError::LimitExceeded(format!(
                "file exceeds {MAX_FILE_READ_BYTES} bytes"
            )));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(&path)
            .map_err(|source| io_error(&path, source))?
            .take(MAX_FILE_READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| io_error(&path, source))?;
        if bytes.len() as u64 > MAX_FILE_READ_BYTES {
            return Err(PluginDataRootError::LimitExceeded(format!(
                "file exceeds {MAX_FILE_READ_BYTES} bytes"
            )));
        }
        Ok(bytes)
    }

    pub fn file_write(&self, relative_path: &str, bytes: &[u8]) -> PluginDataRootResult<()> {
        self.root.verify()?;
        let root = self.root.files_path();
        let destination = prepare_destination_path(&root, relative_path)?;
        if let Ok(metadata) = fs::symlink_metadata(&destination) {
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
                return Err(unsafe_path(
                    &destination,
                    "destination is not a regular file",
                ));
            }
        }
        let parent = destination
            .parent()
            .ok_or_else(|| unsafe_path(&destination, "destination has no parent"))?;
        let temporary = parent.join(format!(".nomifun-write-{}", Uuid::now_v7()));
        let result = (|| {
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error(&temporary, source))?;
            output
                .write_all(bytes)
                .map_err(|source| io_error(&temporary, source))?;
            output
                .sync_all()
                .map_err(|source| io_error(&temporary, source))?;
            drop(output);
            fs::rename(&temporary, &destination)
                .map_err(|source| io_error(&destination, source))?;
            sync_directory(parent)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn file_list(&self, relative_path: Option<&str>) -> PluginDataRootResult<Vec<PluginFileEntry>> {
        self.root.verify()?;
        let files_root = self.root.files_path();
        let start = match relative_path {
            Some(path) => resolve_existing_path(&files_root, path)?,
            None => files_root.clone(),
        };
        let prefix = relative_path.map(normalize_relative_path).transpose()?;
        let mut entries = Vec::new();
        collect_file_entries(&files_root, &start, prefix.as_deref(), &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    pub fn file_delete(&self, relative_path: &str) -> PluginDataRootResult<bool> {
        self.root.verify()?;
        let root = self.root.files_path();
        let normalized = normalize_relative_path(relative_path)?;
        let path = normalized_to_path(&root, &normalized);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(io_error(&path, source)),
        };
        if metadata_is_link_or_reparse(&metadata) {
            return Err(unsafe_path(&path, "refusing to delete a link or reparse point"));
        }
        resolve_existing_path(&root, relative_path)?;
        if metadata.is_file() {
            fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
        } else if metadata.is_dir() {
            validate_removal_tree(&path)?;
            fs::remove_dir_all(&path).map_err(|source| io_error(&path, source))?;
        } else {
            return Err(unsafe_path(&path, "refusing to delete a special file"));
        }
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        Ok(true)
    }

    fn open_internal_database(&self) -> PluginDataRootResult<Connection> {
        self.root.verify()?;
        open_database(&self.root.database_path(), false)
    }

    fn open_user_database(&self) -> PluginDataRootResult<Connection> {
        let connection = self.open_internal_database()?;
        install_user_authorizer(&connection);
        Ok(connection)
    }
}

use rusqlite::OptionalExtension as _;

#[derive(Clone, Debug)]
struct KvRow {
    value: Option<JsonValue>,
    revision: u64,
    deleted: bool,
}

fn read_kv(connection: &Connection, key: &str) -> PluginDataRootResult<KvRead> {
    let row = read_kv_row(connection, key)?;
    Ok(match row {
        Some(row) => KvRead {
            value: (!row.deleted).then_some(row.value).flatten(),
            revision: Some(row.revision),
        },
        None => KvRead {
            value: None,
            revision: None,
        },
    })
}

fn read_kv_row(connection: &Connection, key: &str) -> PluginDataRootResult<Option<KvRow>> {
    let row = connection
        .query_row(
            &format!(
                "SELECT value_json, revision, is_deleted FROM {KV_TABLE} WHERE key = ?1"
            ),
            [key],
            |row| {
                let value = row.get::<_, Option<String>>(0)?;
                let revision = row.get::<_, i64>(1)?;
                let deleted = row.get::<_, bool>(2)?;
                Ok((value, revision, deleted))
            },
        )
        .optional()?;
    row.map(|(value, revision, deleted)| {
        let revision = u64::try_from(revision).map_err(|_| {
            PluginDataRootError::InvalidRequest("persisted KV revision is invalid".into())
        })?;
        let value = value.map(|value| serde_json::from_str(&value)).transpose()?;
        if revision == 0 || (deleted && value.is_some()) || (!deleted && value.is_none()) {
            return Err(PluginDataRootError::InvalidRequest(
                "persisted KV row is invalid".into(),
            ));
        }
        Ok(KvRow {
            value,
            revision,
            deleted,
        })
    })
    .transpose()
}

fn initialize_database(path: &Path) -> PluginDataRootResult<()> {
    let connection = open_database(path, true)?;
    connection.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {KV_TABLE} (
             key TEXT PRIMARY KEY NOT NULL,
             value_json TEXT,
             revision INTEGER NOT NULL CHECK (revision >= 1),
             is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
             CHECK ((is_deleted = 1 AND value_json IS NULL)
                 OR (is_deleted = 0 AND value_json IS NOT NULL AND json_valid(value_json)))
         );
         CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (
             ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
             migration_id TEXT NOT NULL UNIQUE,
             migration_digest TEXT NOT NULL CHECK (length(migration_digest) = 64
                 AND lower(migration_digest) = migration_digest
                 AND migration_digest NOT GLOB '*[^0-9a-f]*'),
             from_version INTEGER NOT NULL CHECK (from_version >= 0),
             to_version INTEGER NOT NULL CHECK (to_version = from_version + 1),
             applied_at_ms INTEGER NOT NULL CHECK (applied_at_ms > 0)
         );"
    ))?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}

fn open_database(path: &Path, create: bool) -> PluginDataRootResult<Connection> {
    validate_database_files(path, create)?;
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    validate_database_files(path, false)?;
    Ok(connection)
}

fn validate_database_files(path: &Path, allow_missing_main: bool) -> PluginDataRootResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| unsafe_path(path, "data.sqlite has no parent"))?;
    require_real_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
                return Err(unsafe_path(path, "data.sqlite is not a regular file"));
            }
        }
        Err(error) if allow_missing_main && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PluginDataRootError::NotFound(path.to_path_buf()));
        }
        Err(source) => return Err(io_error(path, source)),
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| unsafe_path(path, "data.sqlite filename must be UTF-8"))?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = parent.join(format!("{file_name}{suffix}"));
        match fs::symlink_metadata(&sidecar) {
            Ok(metadata) => {
                if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
                    return Err(unsafe_path(
                        &sidecar,
                        "SQLite sidecar is not a regular file",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&sidecar, source)),
        }
    }
    Ok(())
}

fn snapshot_database(source: &Path, destination: &Path) -> PluginDataRootResult<()> {
    require_regular_file(source)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err(PluginDataRootError::AlreadyExists(destination.to_path_buf()));
    }
    let connection = open_database(source, false)?;
    let destination_text = destination.to_str().ok_or_else(|| {
        unsafe_path(destination, "SQLite snapshot path must be valid UTF-8")
    })?;
    connection.execute("VACUUM main INTO ?1", [destination_text])?;
    require_regular_file(destination)
}

#[derive(Debug)]
struct DatabaseAuthorizationState {
    allow_transactions: AtomicBool,
}

fn install_user_authorizer(connection: &Connection) -> Arc<DatabaseAuthorizationState> {
    let state = Arc::new(DatabaseAuthorizationState {
        allow_transactions: AtomicBool::new(false),
    });
    let callback_state = Arc::clone(&state);
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        if context
            .database_name
            .is_some_and(|database| database != "main")
        {
            return Authorization::Deny;
        }
        let reserved = |name: &str| name.to_ascii_lowercase().starts_with(RESERVED_PREFIX);
        match context.action {
            AuthAction::Attach { .. }
            | AuthAction::Detach { .. }
            | AuthAction::Pragma { .. }
            | AuthAction::CreateTempIndex { .. }
            | AuthAction::CreateTempTable { .. }
            | AuthAction::CreateTempTrigger { .. }
            | AuthAction::CreateTempView { .. }
            | AuthAction::DropTempIndex { .. }
            | AuthAction::DropTempTable { .. }
            | AuthAction::DropTempTrigger { .. }
            | AuthAction::DropTempView { .. }
            | AuthAction::CreateVtable { .. }
            | AuthAction::DropVtable { .. }
            | AuthAction::Unknown { .. } => Authorization::Deny,
            AuthAction::Function { function_name }
                if function_name.eq_ignore_ascii_case("load_extension")
                    || function_name.eq_ignore_ascii_case("fts3_tokenizer") =>
            {
                Authorization::Deny
            }
            AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                if callback_state.allow_transactions.load(Ordering::Acquire) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::CreateTable { table_name }
            | AuthAction::DropTable { table_name }
            | AuthAction::Delete { table_name }
            | AuthAction::Insert { table_name }
            | AuthAction::Update { table_name, .. }
                if reserved(table_name) =>
            {
                Authorization::Deny
            }
            AuthAction::CreateIndex {
                index_name,
                table_name,
            }
            | AuthAction::DropIndex {
                index_name,
                table_name,
            } if reserved(index_name) || reserved(table_name) => Authorization::Deny,
            AuthAction::CreateTrigger {
                trigger_name,
                table_name,
            }
            | AuthAction::DropTrigger {
                trigger_name,
                table_name,
            } if reserved(trigger_name) || reserved(table_name) => Authorization::Deny,
            AuthAction::CreateView { view_name } | AuthAction::DropView { view_name }
                if reserved(view_name) =>
            {
                Authorization::Deny
            }
            AuthAction::AlterTable { table_name, .. } if reserved(table_name) => {
                Authorization::Deny
            }
            AuthAction::Reindex { .. } | AuthAction::Analyze { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }
    }));
    state
}

fn validate_user_statement(statement: &PluginSqlStatement) -> PluginDataRootResult<()> {
    let sql = statement.sql.trim();
    if sql.is_empty() || sql.contains('\0') || has_statement_separator(sql) {
        return Err(PluginDataRootError::InvalidRequest(
            "SQL must contain exactly one statement without NUL".into(),
        ));
    }
    if statement.parameters.len() > 1_024 {
        return Err(PluginDataRootError::LimitExceeded(
            "SQL parameter count exceeds 1024".into(),
        ));
    }
    Ok(())
}

fn execute_user_statement(
    connection: &Connection,
    statement: &PluginSqlStatement,
) -> PluginDataRootResult<PluginExecuteResult> {
    if contains_reserved_identifier(&statement.sql) {
        return Err(PluginDataRootError::InvalidRequest(
            "Plugin SQL cannot write Host-reserved _nomifun_* objects".into(),
        ));
    }
    let mut prepared = connection.prepare(&statement.sql)?;
    if prepared.readonly() {
        return Err(PluginDataRootError::InvalidRequest(
            "db.execute requires a write or schema statement".into(),
        ));
    }
    bind_parameters(&mut prepared, &statement.parameters)?;
    let affected_rows = prepared.raw_execute()?;
    Ok(PluginExecuteResult {
        affected_rows: affected_rows as u64,
    })
}

fn bind_parameters(
    statement: &mut rusqlite::Statement<'_>,
    parameters: &[PluginSqlValue],
) -> PluginDataRootResult<()> {
    if statement.parameter_count() != parameters.len() {
        return Err(PluginDataRootError::InvalidRequest(format!(
            "SQL expects {} parameters but received {}",
            statement.parameter_count(),
            parameters.len()
        )));
    }
    for (index, value) in parameters.iter().enumerate() {
        statement.raw_bind_parameter(index + 1, sqlite_value(value))?;
    }
    Ok(())
}

fn sqlite_value(value: &PluginSqlValue) -> SqliteValue {
    match value {
        PluginSqlValue::Null => SqliteValue::Null,
        PluginSqlValue::Integer(value) => SqliteValue::Integer(*value),
        PluginSqlValue::Real(value) => SqliteValue::Real(*value),
        PluginSqlValue::Text(value) => SqliteValue::Text(value.clone()),
        PluginSqlValue::Blob(value) => SqliteValue::Blob(value.clone()),
    }
}

fn sql_value(value: ValueRef<'_>) -> PluginSqlValue {
    match value {
        ValueRef::Null => PluginSqlValue::Null,
        ValueRef::Integer(value) => PluginSqlValue::Integer(value),
        ValueRef::Real(value) => PluginSqlValue::Real(value),
        ValueRef::Text(value) => {
            PluginSqlValue::Text(String::from_utf8_lossy(value).into_owned())
        }
        ValueRef::Blob(value) => PluginSqlValue::Blob(value.to_vec()),
    }
}

fn sql_value_size(value: &PluginSqlValue) -> usize {
    match value {
        PluginSqlValue::Null => 0,
        PluginSqlValue::Integer(_) | PluginSqlValue::Real(_) => 8,
        PluginSqlValue::Text(value) => value.len(),
        PluginSqlValue::Blob(value) => value.len(),
    }
}

fn has_statement_separator(sql: &str) -> bool {
    #[derive(Clone, Copy)]
    enum State {
        Plain,
        Single,
        Double,
        Backtick,
        Bracket,
        LineComment,
        BlockComment,
    }
    let bytes = sql.as_bytes();
    let mut index = 0usize;
    let mut state = State::Plain;
    while index < bytes.len() {
        let current = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Plain => match (current, next) {
                (b'\'', _) => state = State::Single,
                (b'"', _) => state = State::Double,
                (b'`', _) => state = State::Backtick,
                (b'[', _) => state = State::Bracket,
                (b'-', Some(b'-')) => {
                    state = State::LineComment;
                    index += 1;
                }
                (b'/', Some(b'*')) => {
                    state = State::BlockComment;
                    index += 1;
                }
                (b';', _) => return true,
                _ => {}
            },
            State::Single if current == b'\'' => {
                if next == Some(b'\'') {
                    index += 1;
                } else {
                    state = State::Plain;
                }
            }
            State::Double if current == b'"' => {
                if next == Some(b'"') {
                    index += 1;
                } else {
                    state = State::Plain;
                }
            }
            State::Backtick if current == b'`' => state = State::Plain,
            State::Bracket if current == b']' => state = State::Plain,
            State::LineComment if matches!(current, b'\n' | b'\r') => state = State::Plain,
            State::BlockComment if current == b'*' && next == Some(b'/') => {
                state = State::Plain;
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    false
}

fn contains_reserved_identifier(sql: &str) -> bool {
    #[derive(Clone, Copy)]
    enum State {
        Plain,
        Single,
        LineComment,
        BlockComment,
    }

    fn flush(token: &mut String) -> bool {
        let reserved = token
            .to_ascii_lowercase()
            .starts_with(RESERVED_PREFIX);
        token.clear();
        reserved
    }

    let bytes = sql.as_bytes();
    let mut state = State::Plain;
    let mut token = String::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let current = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            State::Plain => match (current, next) {
                (b'\'', _) => {
                    if flush(&mut token) {
                        return true;
                    }
                    state = State::Single;
                }
                (b'-', Some(b'-')) => {
                    if flush(&mut token) {
                        return true;
                    }
                    state = State::LineComment;
                    index += 1;
                }
                (b'/', Some(b'*')) => {
                    if flush(&mut token) {
                        return true;
                    }
                    state = State::BlockComment;
                    index += 1;
                }
                _ if current.is_ascii_alphanumeric() || current == b'_' => {
                    token.push(current as char);
                }
                _ => {
                    if flush(&mut token) {
                        return true;
                    }
                }
            },
            State::Single if current == b'\'' => {
                if next == Some(b'\'') {
                    index += 1;
                } else {
                    state = State::Plain;
                }
            }
            State::LineComment if matches!(current, b'\n' | b'\r') => state = State::Plain,
            State::BlockComment if current == b'*' && next == Some(b'/') => {
                state = State::Plain;
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    flush(&mut token)
}

fn checked_json(value: &JsonValue) -> PluginDataRootResult<String> {
    let encoded = serde_json::to_string(value)?;
    if encoded.len() > MAX_KV_JSON_BYTES {
        return Err(PluginDataRootError::LimitExceeded(format!(
            "KV value exceeds {MAX_KV_JSON_BYTES} bytes"
        )));
    }
    Ok(encoded)
}

fn validate_kv_key(key: &str) -> PluginDataRootResult<()> {
    validate_visible_key(key, "KV key", 256)
}

fn validate_cache_key(key: &str) -> PluginDataRootResult<()> {
    validate_visible_key(key, "cache key", 256)
}

fn validate_visible_key(
    value: &str,
    label: &'static str,
    maximum: usize,
) -> PluginDataRootResult<()> {
    if value.is_empty()
        || value.len() > maximum
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(PluginDataRootError::InvalidIdentity {
            label,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_migration_record(record: &PluginMigrationRecord) -> PluginDataRootResult<()> {
    validate_visible_key(&record.migration_id, "migration id", 128)?;
    let digest = record.migration_digest.as_ref();
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        || record.to_version != record.from_version.saturating_add(1)
        || record.applied_at_ms <= 0
    {
        return Err(PluginDataRootError::InvalidRequest(
            "migration record is invalid".into(),
        ));
    }
    Ok(())
}

fn next_revision(current: Option<u64>) -> PluginDataRootResult<u64> {
    current
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PluginDataRootError::LimitExceeded("KV revision overflow".into()))
}

fn revision_as_i64(revision: u64) -> PluginDataRootResult<i64> {
    i64::try_from(revision)
        .map_err(|_| PluginDataRootError::LimitExceeded("KV revision overflow".into()))
}

fn production_cache_scope(plugin_id: &PluginId) -> String {
    format!("plugin:{}", plugin_id.as_ref())
}

fn validate_component(value: &str, label: &'static str) -> PluginDataRootResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value.nfc().collect::<String>() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
        })
        && !value.ends_with([' ', '.'])
        && !is_windows_reserved_name(value);
    if !valid {
        return Err(PluginDataRootError::InvalidIdentity {
            label,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn normalize_relative_path(value: &str) -> PluginDataRootResult<String> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_PATH_BYTES
        || value.contains(['\0', '\\', ':'])
        || Path::new(value).is_absolute()
    {
        return Err(PluginDataRootError::InvalidRequest(
            "file path must be a safe portable relative path".into(),
        ));
    }
    let mut components = Vec::new();
    for component in Path::new(value).components() {
        match component {
            Component::Normal(component) => {
                let component = component.to_str().ok_or_else(|| {
                    PluginDataRootError::InvalidRequest("file path must be UTF-8".into())
                })?;
                if component.is_empty()
                    || component.len() > MAX_COMPONENT_BYTES
                    || component.nfc().collect::<String>() != component
                    || component.ends_with([' ', '.'])
                    || is_windows_reserved_name(component)
                {
                    return Err(PluginDataRootError::InvalidRequest(
                        "file path is not portable".into(),
                    ));
                }
                components.push(component);
            }
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginDataRootError::InvalidRequest(
                    "file path traversal is forbidden".into(),
                ));
            }
        }
    }
    let normalized = components.join("/");
    if normalized != value || normalized.is_empty() {
        return Err(PluginDataRootError::InvalidRequest(
            "file path must already be normalized".into(),
        ));
    }
    Ok(normalized)
}

fn normalized_to_path(root: &Path, normalized: &str) -> PathBuf {
    normalized
        .split('/')
        .fold(root.to_path_buf(), |mut path, component| {
            path.push(component);
            path
        })
}

fn resolve_existing_path(root: &Path, value: &str) -> PluginDataRootResult<PathBuf> {
    let normalized = normalize_relative_path(value)?;
    require_real_directory(root)?;
    let canonical_root = canonicalize(root)?;
    let mut path = canonical_root.clone();
    for component in normalized.split('/') {
        path.push(component);
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PluginDataRootError::NotFound(path.clone())
            } else {
                io_error(&path, source)
            }
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(unsafe_path(&path, "file path contains a link or reparse point"));
        }
    }
    let canonical = canonicalize(&path)?;
    if !canonical.starts_with(&canonical_root) || canonical == canonical_root {
        return Err(unsafe_path(&path, "file path escaped its DataRoot"));
    }
    Ok(canonical)
}

fn prepare_destination_path(root: &Path, value: &str) -> PluginDataRootResult<PathBuf> {
    let normalized = normalize_relative_path(value)?;
    require_real_directory(root)?;
    let canonical_root = canonicalize(root)?;
    let components = normalized.split('/').collect::<Vec<_>>();
    let mut parent = canonical_root.clone();
    for component in &components[..components.len().saturating_sub(1)] {
        let next = parent.join(component);
        match fs::symlink_metadata(&next) {
            Ok(metadata) => {
                if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                    return Err(unsafe_path(&next, "file parent is not a real directory"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&next).map_err(|source| io_error(&next, source))?;
                sync_directory(&parent)?;
            }
            Err(source) => return Err(io_error(&next, source)),
        }
        parent = canonicalize(&next)?;
        if !parent.starts_with(&canonical_root) {
            return Err(unsafe_path(&parent, "file parent escaped its DataRoot"));
        }
    }
    let destination = parent.join(components.last().expect("normalized path component"));
    if !destination.starts_with(&canonical_root) {
        return Err(unsafe_path(&destination, "file destination escaped its DataRoot"));
    }
    Ok(destination)
}

fn collect_file_entries(
    root: &Path,
    current: &Path,
    requested: Option<&str>,
    output: &mut Vec<PluginFileEntry>,
) -> PluginDataRootResult<()> {
    let metadata = fs::symlink_metadata(current).map_err(|source| io_error(current, source))?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(unsafe_path(current, "Files tree contains a link or reparse point"));
    }
    if metadata.is_file() {
        let relative = current.strip_prefix(root).map_err(|_| {
            unsafe_path(current, "Files entry escaped its root")
        })?;
        output.push(PluginFileEntry {
            path: relative_path_string(relative)?,
            is_directory: false,
            size_bytes: metadata.len(),
        });
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(current, "Files tree contains a special entry"));
    }
    if current != root && requested.is_none_or(|requested| {
        current
            .strip_prefix(root)
            .ok()
            .and_then(|path| relative_path_string(path).ok())
            .as_deref()
            != Some(requested)
    }) {
        let relative = current.strip_prefix(root).map_err(|_| {
            unsafe_path(current, "Files entry escaped its root")
        })?;
        output.push(PluginFileEntry {
            path: relative_path_string(relative)?,
            is_directory: true,
            size_bytes: 0,
        });
    }
    let mut children = fs::read_dir(current)
        .map_err(|source| io_error(current, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(current, source))?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        collect_file_entries(root, &child.path(), requested, output)?;
    }
    Ok(())
}

fn relative_path_string(path: &Path) -> PluginDataRootResult<String> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_owned).ok_or_else(|| {
                PluginDataRootError::InvalidRequest("file path must be UTF-8".into())
            }),
            _ => Err(PluginDataRootError::InvalidRequest(
                "file path must be normalized".into(),
            )),
        })
        .collect::<PluginDataRootResult<Vec<_>>>()?;
    Ok(components.join("/"))
}

fn copy_files_tree(source: &Path, destination: &Path) -> PluginDataRootResult<()> {
    require_real_directory(source)?;
    require_real_directory(destination)?;
    let mut children = fs::read_dir(source)
        .map_err(|error| io_error(source, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error(source, error))?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        let source_path = child.path();
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| io_error(&source_path, error))?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(unsafe_path(&source_path, "Files clone contains a link"));
        }
        let destination_path = destination.join(child.file_name());
        if metadata.is_dir() {
            fs::create_dir(&destination_path)
                .map_err(|error| io_error(&destination_path, error))?;
            copy_files_tree(&source_path, &destination_path)?;
            sync_directory(&destination_path)?;
        } else if metadata.is_file() {
            let mut input = File::open(&source_path)
                .map_err(|error| io_error(&source_path, error))?;
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination_path)
                .map_err(|error| io_error(&destination_path, error))?;
            std::io::copy(&mut input, &mut output)
                .map_err(|error| io_error(&destination_path, error))?;
            output
                .sync_all()
                .map_err(|error| io_error(&destination_path, error))?;
        } else {
            return Err(unsafe_path(&source_path, "Files clone contains a special entry"));
        }
    }
    Ok(())
}

fn create_exact_directory(parent: &Path, path: &Path) -> PluginDataRootResult<()> {
    require_real_directory(parent)?;
    if path.parent() != Some(parent) {
        return Err(unsafe_path(path, "directory is not an exact child"));
    }
    match fs::create_dir(path) {
        Ok(()) => {
            sync_directory(parent)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(PluginDataRootError::AlreadyExists(path.to_path_buf()))
        }
        Err(source) => Err(io_error(path, source)),
    }
}

fn ensure_real_directory(path: &Path) -> PluginDataRootResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(path, "expected a real directory"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
            let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
            if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
                return Err(unsafe_path(path, "created path is not a real directory"));
            }
        }
        Err(source) => return Err(io_error(path, source)),
    }
    Ok(())
}

fn ensure_direct_child_directory(parent: &Path, child: &Path) -> PluginDataRootResult<()> {
    require_real_directory(parent)?;
    if child.parent() != Some(parent) {
        return Err(unsafe_path(child, "managed directory is not a direct child"));
    }
    ensure_real_directory(child)?;
    let canonical_parent = canonicalize(parent)?;
    let canonical_child = canonicalize(child)?;
    if canonical_child.parent() != Some(canonical_parent.as_path()) {
        return Err(unsafe_path(child, "managed directory escaped its parent"));
    }
    Ok(())
}

fn require_real_directory(path: &Path) -> PluginDataRootResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            PluginDataRootError::NotFound(path.to_path_buf())
        } else {
            io_error(path, source)
        }
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(unsafe_path(path, "expected a real directory"));
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> PluginDataRootResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            PluginDataRootError::NotFound(path.to_path_buf())
        } else {
            io_error(path, source)
        }
    })?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(unsafe_path(path, "expected a regular file"));
    }
    Ok(())
}

fn remove_owned_tree(parent: &Path, target: &Path) -> PluginDataRootResult<bool> {
    require_real_directory(parent)?;
    if target.parent() != Some(parent) {
        return Err(unsafe_path(target, "removal target is not an exact child"));
    }
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io_error(target, source)),
    };
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(unsafe_path(target, "removal target is not a real directory"));
    }
    validate_removal_tree(target)?;
    fs::remove_dir_all(target).map_err(|source| io_error(target, source))?;
    sync_directory(parent)?;
    Ok(true)
}

fn validate_removal_tree(path: &Path) -> PluginDataRootResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(unsafe_path(path, "tree contains a link or reparse point"));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(path, "tree contains a special entry"));
    }
    for entry in fs::read_dir(path).map_err(|source| io_error(path, source))? {
        let entry = entry.map_err(|source| io_error(path, source))?;
        validate_removal_tree(&entry.path())?;
    }
    Ok(())
}

fn sync_tree(path: &Path) -> PluginDataRootResult<()> {
    validate_removal_tree(path)?;
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|source| io_error(path, source))? {
            let entry = entry.map_err(|source| io_error(path, source))?;
            if entry
                .file_type()
                .map_err(|source| io_error(&entry.path(), source))?
                .is_dir()
            {
                sync_tree(&entry.path())?;
            } else {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(entry.path())
                    .and_then(|file| file.sync_all())
                    .map_err(|source| io_error(&entry.path(), source))?;
            }
        }
        sync_directory(path)?;
    }
    Ok(())
}

fn canonicalize(path: &Path) -> PluginDataRootResult<PathBuf> {
    fs::canonicalize(path).map_err(|source| io_error(path, source))
}

fn is_windows_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> PluginDataRootResult<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| io_error(path, source))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> PluginDataRootResult<()> {
    match File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> PluginDataRootResult<()> {
    Ok(())
}

fn unsafe_path(path: &Path, reason: impl Into<String>) -> PluginDataRootError {
    PluginDataRootError::UnsafePath {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> PluginDataRootError {
    PluginDataRootError::Io {
        path: path.to_path_buf(),
        source,
    }
}
