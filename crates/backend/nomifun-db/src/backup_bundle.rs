//! Offline backup-bundle and object-graph import primitives.
//!
//! This module deliberately does not replace a live database.  It provides the
//! small, deterministic core needed by a future offline command:
//! - a WAL-safe SQLite snapshot plus a checksummed manifest;
//! - restore/merge semantics that preserve IDs and reject divergent content;
//! - clone semantics that mint new IDs and rewrite every declared reference
//!   through one explicit old-to-new map.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};

use nomifun_common::{
    TimestampMs, generate_prefixed_id, now_ms, validate_id_prefix, validate_uuidv7,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Database, DbError, database::validate_sqlite_snapshot};

pub const BACKUP_FORMAT: &str = "nomifun-backup";
pub const BACKUP_FORMAT_VERSION: u32 = 1;
pub const BACKUP_SCHEMA: &str = "id-contract-v2";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const DATABASE_FILE: &str = "database.sqlite3";
pub const ENCRYPTION_KEY_FILE: &str = "encryption_key";
pub const COMPANION_DIR: &str = "companion";
pub const MANAGED_WORKSPACES_DIR: &str = "conversations";

/// Bundle paths are deliberately independent of the source data/work roots.
/// Restore always materializes them below the destination data directory.
pub const BUNDLE_DATA_DIR: &str = "data";
pub const BUNDLE_WORK_DIR: &str = "work";

/// Defensive limits for an offline, user-controlled bundle. They are high
/// enough for ordinary agent workspaces while preventing a crafted manifest
/// from making verification/restore consume unbounded disk space.
pub const MAX_BACKUP_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_BACKUP_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_BACKUP_FILES: usize = 200_000;
pub const MAX_BACKUP_DIRECTORIES: usize = 200_000;
pub const MAX_BACKUP_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSource<'a> {
    pub data_dir: &'a Path,
    pub work_dir: &'a Path,
}

impl<'a> BackupSource<'a> {
    pub fn new(data_dir: &'a Path, work_dir: &'a Path) -> Self {
        Self { data_dir, work_dir }
    }
}

/// Validate source roots before a caller opens any source file. The data root
/// is required; a not-yet-created work root is allowed and simply contributes
/// no managed workspaces.
pub fn validate_backup_source_roots(source: BackupSource<'_>) -> Result<(), BackupError> {
    ensure_existing_directory_is_safe(source.data_dir, "backup data root")
        .map_err(|error| BackupError::UnsafeSource(error.to_string()))?;
    match fs::symlink_metadata(source.work_dir) {
        Ok(_) => ensure_existing_directory_is_safe(source.work_dir, "backup work root")
            .map_err(|error| BackupError::UnsafeSource(error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupLayout {
    /// Managed workspaces are restored below
    /// `<destination_data_dir>/conversations`, regardless of the source
    /// installation's custom work-dir setting.
    pub managed_workspaces: String,
    /// Custom workspaces outside the managed `conversations/` tree are user
    /// paths and are never captured by this bundle.
    pub custom_external_workspaces_included: bool,
}

impl BackupLayout {
    fn full_offline_bundle() -> Self {
        Self {
            managed_workspaces: MANAGED_WORKSPACES_DIR.to_owned(),
            custom_external_workspaces_included: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupObjectGraph {
    /// Root entity IDs included in the logical backup selection.  A full
    /// dataset backup uses an empty list to mean "the complete database".
    pub roots: Vec<String>,
    /// Entity kinds represented in the snapshot or logical graph.
    pub entity_kinds: Vec<String>,
}

impl BackupObjectGraph {
    pub fn full_database() -> Self {
        Self {
            roots: Vec::new(),
            entity_kinds: vec!["database".to_owned()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupFileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub format_version: u32,
    pub schema: String,
    pub source_storage_generation: String,
    pub created_at: TimestampMs,
    pub object_graph: BackupObjectGraph,
    pub layout: BackupLayout,
    /// Explicit directory entries preserve empty companion/workspace
    /// directories and let verification reject undeclared directory trees.
    pub directories: Vec<String>,
    pub files: Vec<BackupFileEntry>,
}

impl BackupManifest {
    pub fn validate(&self) -> Result<(), BackupError> {
        if self.format != BACKUP_FORMAT {
            return Err(BackupError::InvalidManifest(format!(
                "unsupported backup format: {}",
                self.format
            )));
        }
        if self.format_version != BACKUP_FORMAT_VERSION {
            return Err(BackupError::InvalidManifest(format!(
                "unsupported backup format version: {}",
                self.format_version
            )));
        }
        if self.schema != BACKUP_SCHEMA {
            return Err(BackupError::InvalidManifest(format!(
                "unsupported backup schema: {}",
                self.schema
            )));
        }
        validate_uuidv7(&self.source_storage_generation).map_err(|_| {
            BackupError::InvalidManifest(
                "source storage generation must be canonical lowercase UUIDv7".into(),
            )
        })?;
        if self.created_at < 0 {
            return Err(BackupError::InvalidManifest(
                "created_at must be an epoch-millisecond timestamp".into(),
            ));
        }
        if self.object_graph.entity_kinds.is_empty() {
            return Err(BackupError::InvalidManifest(
                "object graph must declare at least one entity kind".into(),
            ));
        }
        if self.layout.managed_workspaces != MANAGED_WORKSPACES_DIR {
            return Err(BackupError::InvalidManifest(format!(
                "unsupported managed-workspace restore path: {}",
                self.layout.managed_workspaces
            )));
        }
        if self.layout.custom_external_workspaces_included {
            return Err(BackupError::InvalidManifest(
                "custom external workspaces must not be embedded in an offline bundle".into(),
            ));
        }
        for root in &self.object_graph.roots {
            validate_runtime_prefixed_id(root, None)?;
        }
        if self.files.is_empty() {
            return Err(BackupError::InvalidManifest(
                "backup manifest contains no files".into(),
            ));
        }
        if self.files.len() > MAX_BACKUP_FILES {
            return Err(BackupError::InvalidManifest(format!(
                "backup contains too many files: {} > {MAX_BACKUP_FILES}",
                self.files.len()
            )));
        }
        if self.directories.len() > MAX_BACKUP_DIRECTORIES {
            return Err(BackupError::InvalidManifest(format!(
                "backup contains too many directories: {} > {MAX_BACKUP_DIRECTORIES}",
                self.directories.len()
            )));
        }
        let mut directories = BTreeSet::new();
        for directory in &self.directories {
            validate_relative_bundle_path(directory)?;
            if !directories.insert(directory.clone()) {
                return Err(BackupError::InvalidManifest(format!(
                    "duplicate backup directory entry: {directory}"
                )));
            }
            if directory != &format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}")
                && !directory.starts_with(&format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}/"))
                && directory != &format!("{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}")
                && !directory.starts_with(&format!(
                    "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/"
                ))
            {
                return Err(BackupError::InvalidManifest(format!(
                    "unsupported backup directory path: {directory}"
                )));
            }
        }
        let mut paths = BTreeSet::new();
        let mut total_bytes = 0_u64;
        for file in &self.files {
            validate_relative_bundle_path(&file.path)?;
            if !paths.insert(file.path.clone()) {
                return Err(BackupError::InvalidManifest(format!(
                    "duplicate backup file entry: {}",
                    file.path
                )));
            }
            if file.bytes > MAX_BACKUP_FILE_BYTES {
                return Err(BackupError::InvalidManifest(format!(
                    "backup file is too large: {} has {} bytes (limit {MAX_BACKUP_FILE_BYTES})",
                    file.path, file.bytes
                )));
            }
            total_bytes = total_bytes.checked_add(file.bytes).ok_or_else(|| {
                BackupError::InvalidManifest("backup byte total overflowed".into())
            })?;
            if total_bytes > MAX_BACKUP_TOTAL_BYTES {
                return Err(BackupError::InvalidManifest(format!(
                    "backup is too large: {total_bytes} bytes (limit {MAX_BACKUP_TOTAL_BYTES})"
                )));
            }
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(BackupError::InvalidManifest(format!(
                    "invalid SHA-256 digest for {}",
                    file.path
                )));
            }
        }
        if !paths.contains(DATABASE_FILE) {
            return Err(BackupError::InvalidManifest(format!(
                "backup manifest must contain {DATABASE_FILE}"
            )));
        }
        for path in &paths {
            if path != DATABASE_FILE
                && path != &format!("{BUNDLE_DATA_DIR}/{ENCRYPTION_KEY_FILE}")
                && !path.starts_with(&format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}/"))
                && !path.starts_with(&format!(
                    "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/"
                ))
            {
                return Err(BackupError::InvalidManifest(format!(
                    "unsupported backup payload path: {path}"
                )));
            }
        }
        if let Some(collision) = paths.iter().find(|path| directories.contains(*path)) {
            return Err(BackupError::InvalidManifest(format!(
                "backup path is declared as both a file and directory: {collision}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("backup I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("backup checksum mismatch for {path}")]
    ChecksumMismatch { path: String },
    #[error("backup source is unsafe: {0}")]
    UnsafeSource(String),
    #[error("backup import conflict for {entity_type} {id}")]
    Conflict { entity_type: String, id: String },
    #[error("backup object graph is invalid: {0}")]
    InvalidGraph(String),
}

/// Create a database-only offline directory bundle.
///
/// The database is snapshotted through SQLite so committed WAL pages are
/// included. Call [`create_backup_bundle_with_sources`] for the complete CLI
/// bundle that also carries the persistent key and portable file domains.
pub async fn create_backup_bundle(
    database: &Database,
    destination: &Path,
    source_storage_generation: &str,
    object_graph: BackupObjectGraph,
) -> Result<BackupManifest, BackupError> {
    create_backup_bundle_impl(
        database,
        destination,
        source_storage_generation,
        object_graph,
        None,
    )
    .await
}

pub async fn create_backup_bundle_with_sources(
    database: &Database,
    destination: &Path,
    source_storage_generation: &str,
    object_graph: BackupObjectGraph,
    source: BackupSource<'_>,
) -> Result<BackupManifest, BackupError> {
    create_backup_bundle_impl(
        database,
        destination,
        source_storage_generation,
        object_graph,
        Some(source),
    )
    .await
}

async fn create_backup_bundle_impl(
    database: &Database,
    destination: &Path,
    source_storage_generation: &str,
    object_graph: BackupObjectGraph,
    source: Option<BackupSource<'_>>,
) -> Result<BackupManifest, BackupError> {
    if path_exists_no_follow(destination)? {
        return Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("backup destination already exists: {}", destination.display()),
        )));
    }
    validate_uuidv7(source_storage_generation).map_err(|_| {
        BackupError::InvalidManifest(
            "source storage generation must be canonical lowercase UUIDv7".into(),
        )
    })?;

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    create_directory_tree_safe(parent, "backup destination parent")?;
    if let Some(source) = source {
        reject_backup_output_overlap(destination, source)?;
    }
    let staging = create_sibling_staging(destination)?;

    let result = async {
        let database_path = staging.join(DATABASE_FILE);
        database.snapshot_into(&database_path).await?;
        set_private_file_permissions(&database_path)?;
        let mut files = vec![file_entry(&database_path, DATABASE_FILE)?];
        let mut directories = Vec::new();

        if let Some(source) = source {
            copy_optional_regular_file(
                &source.data_dir.join(ENCRYPTION_KEY_FILE),
                &staging.join(BUNDLE_DATA_DIR).join(ENCRYPTION_KEY_FILE),
                &format!("{BUNDLE_DATA_DIR}/{ENCRYPTION_KEY_FILE}"),
                &mut files,
            )?;
            copy_optional_tree(
                &source.data_dir.join(COMPANION_DIR),
                &staging.join(BUNDLE_DATA_DIR).join(COMPANION_DIR),
                &format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}"),
                &mut directories,
                &mut files,
            )?;
            copy_optional_tree(
                &source.work_dir.join(MANAGED_WORKSPACES_DIR),
                &staging
                    .join(BUNDLE_WORK_DIR)
                    .join(MANAGED_WORKSPACES_DIR),
                &format!("{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}"),
                &mut directories,
                &mut files,
            )?;
        }
        directories.sort();
        files.sort_by(|left, right| left.path.cmp(&right.path));

        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_owned(),
            format_version: BACKUP_FORMAT_VERSION,
            schema: BACKUP_SCHEMA.to_owned(),
            source_storage_generation: source_storage_generation.to_owned(),
            created_at: now_ms(),
            object_graph,
            layout: BackupLayout::full_offline_bundle(),
            directories,
            files,
        };
        manifest.validate()?;
        write_json_atomic(&staging.join(MANIFEST_FILE), &manifest)?;
        verify_backup_bundle(&staging)?;
        sync_directory_best_effort(&staging);
        Ok::<_, BackupError>(manifest)
    }
    .await;

    match result {
        Ok(manifest) => {
            if let Err(error) = install_staging_directory(&staging, destination) {
                let _ = remove_staging_dir(&staging);
                return Err(error);
            }
            sync_directory_best_effort(parent);
            Ok(manifest)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// Validate the manifest and every declared file checksum.
pub fn verify_backup_bundle(bundle_dir: &Path) -> Result<BackupManifest, BackupError> {
    let bundle_metadata = fs::symlink_metadata(bundle_dir)?;
    if bundle_metadata.file_type().is_symlink() || !bundle_metadata.is_dir() {
        return Err(BackupError::InvalidManifest(format!(
            "backup bundle root is not a regular directory: {}",
            bundle_dir.display()
        )));
    }
    ensure_existing_directory_is_safe(bundle_dir, "backup bundle root")?;
    let manifest_path = bundle_dir.join(MANIFEST_FILE);
    let manifest_metadata = fs::symlink_metadata(&manifest_path)?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(BackupError::InvalidManifest(
            "backup manifest is not a regular file".into(),
        ));
    }
    let mut manifest_bytes = Vec::new();
    open_regular_file_no_follow(&manifest_path)?
        .take(MAX_BACKUP_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)?;
    if manifest_bytes.len() as u64 > MAX_BACKUP_MANIFEST_BYTES {
        return Err(BackupError::InvalidManifest(format!(
            "backup manifest exceeds {MAX_BACKUP_MANIFEST_BYTES} bytes"
        )));
    }
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| BackupError::InvalidManifest(error.to_string()))?;
    manifest.validate()?;
    let manifest_paths: BTreeSet<&str> =
        manifest.files.iter().map(|entry| entry.path.as_str()).collect();
    let manifest_directories: BTreeSet<&str> = manifest
        .directories
        .iter()
        .map(String::as_str)
        .collect();
    verify_bundle_tree(bundle_dir, &manifest_paths, &manifest_directories)?;

    let mut verified_total = 0_u64;
    for file in &manifest.files {
        let path = resolve_bundle_file(bundle_dir, &file.path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(BackupError::InvalidManifest(format!(
                "backup entry is not a regular file: {}",
                file.path
            )));
        }
        verified_total = verified_total.checked_add(metadata.len()).ok_or_else(|| {
            BackupError::InvalidManifest("verified backup byte total overflowed".into())
        })?;
        if verified_total > MAX_BACKUP_TOTAL_BYTES {
            return Err(BackupError::InvalidManifest(format!(
                "verified backup exceeds {MAX_BACKUP_TOTAL_BYTES} bytes"
            )));
        }
        if metadata.len() != file.bytes || sha256_file(&path)? != file.sha256 {
            return Err(BackupError::ChecksumMismatch {
                path: file.path.clone(),
            });
        }
    }
    Ok(manifest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub manifest: BackupManifest,
    /// Restore creates a fresh dataset namespace. The source generation remains
    /// provenance in the manifest; reusing it would let browser-local state
    /// from a different point in the source timeline leak into this restore.
    pub destination_storage_generation: String,
}

/// Materialize a verified backup into a new offline data directory.
///
/// The destination must be absent. Every payload is first copied into a
/// sibling staging directory, the SQLite snapshot is validated there, and the
/// complete directory is installed with one rename. Entity IDs are preserved,
/// while storage-generation is deliberately rotated so browser-side caches
/// cannot bind stale state to the restored graph.
pub async fn restore_backup_bundle(
    bundle_dir: &Path,
    destination_database: &Path,
    destination_generation_file: &Path,
) -> Result<RestoreOutcome, BackupError> {
    if path_exists_no_follow(destination_database)?
        || path_exists_no_follow(destination_generation_file)?
    {
        return Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "restore database/generation destination already exists",
        )));
    }
    let parent = destination_database
        .parent()
        .ok_or_else(|| BackupError::InvalidManifest("database has no parent directory".into()))?;
    if destination_generation_file.parent() != Some(parent) {
        return Err(BackupError::InvalidManifest(
            "database and generation file must share one destination directory".into(),
        ));
    }
    if destination_database.file_name().and_then(|name| name.to_str())
        != Some("nomifun-backend.db")
        || destination_generation_file
            .file_name()
            .and_then(|name| name.to_str())
            != Some("storage-generation")
    {
        return Err(BackupError::InvalidManifest(
            "complete restore requires canonical nomifun-backend.db and storage-generation paths"
                .into(),
        ));
    }
    restore_backup_data_dir(bundle_dir, parent).await
}

pub async fn restore_backup_data_dir(
    bundle_dir: &Path,
    destination_data_dir: &Path,
) -> Result<RestoreOutcome, BackupError> {
    let manifest = verify_backup_bundle(bundle_dir)?;
    if path_exists_no_follow(destination_data_dir)? {
        return Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "restore destination already exists: {}",
                destination_data_dir.display()
            ),
        )));
    }
    let parent = destination_data_dir
        .parent()
        .unwrap_or_else(|| Path::new("."));
    create_directory_tree_safe(parent, "restore destination parent")?;

    let staging = create_sibling_staging(destination_data_dir)?;
    let destination_storage_generation = uuid::Uuid::now_v7().to_string();
    let result = async {
        let mut directories: Vec<_> = manifest.directories.iter().collect();
        directories.sort_by(|left, right| {
            Path::new(left)
                .components()
                .count()
                .cmp(&Path::new(right).components().count())
                .then_with(|| left.cmp(right))
        });
        for directory in directories {
            let relative_destination = restore_relative_directory(directory)?;
            fs::create_dir(staging.join(relative_destination))?;
        }
        for entry in &manifest.files {
            let source = resolve_bundle_file(bundle_dir, &entry.path)?;
            let relative_destination = restore_relative_path(&entry.path)?;
            let destination = staging.join(relative_destination);
            copy_regular_file_bounded(&source, &destination, entry.bytes)?;
            if sha256_file(&destination)? != entry.sha256 {
                return Err(BackupError::ChecksumMismatch {
                    path: entry.path.clone(),
                });
            }
        }
        let restored_database = staging.join("nomifun-backend.db");
        validate_sqlite_snapshot(&restored_database).await?;
        write_synced_file(
            &staging.join("storage-generation"),
            destination_storage_generation.as_bytes(),
        )?;
        sync_directory_best_effort(&staging);
        install_staging_directory(&staging, destination_data_dir)?;
        sync_directory_best_effort(parent);
        Ok::<(), BackupError>(())
    }
    .await;
    if let Err(error) = result {
        let _ = remove_staging_dir(&staging);
        return Err(error);
    }

    Ok(RestoreOutcome {
        manifest,
        destination_storage_generation,
    })
}

/// One portable logical entity record. `payload` contains the entity's own
/// fields and `references` declares every durable entity-ID edge by JSON
/// Pointer.
///
/// A reference value may be either:
/// - a single canonical entity-ID string; or
/// - an array/object containing canonical entity-ID strings at any depth.
///
/// The legacy single-string representation remains wire-compatible. During a
/// clone both the declared reference value and the value at the same pointer
/// in `payload` are recursively rewritten through the one old-to-new map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableEntity {
    pub entity_type: String,
    pub id_prefix: String,
    pub id: String,
    pub payload: Value,
    #[serde(default)]
    pub references: BTreeMap<String, Value>,
}

impl PortableEntity {
    fn validate(&self) -> Result<(), BackupError> {
        validate_runtime_prefixed_id(&self.id, Some(&self.id_prefix))?;
        if !self.payload.is_object() {
            return Err(BackupError::InvalidGraph(format!(
                "{} {} payload must be a JSON object",
                self.entity_type, self.id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PortableGraph {
    pub entities: Vec<PortableEntity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Restore,
    Merge,
    Clone,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportReport {
    pub inserted: usize,
    pub skipped_identical: usize,
    pub remap: BTreeMap<String, String>,
}

/// In-memory catalog used by the offline import planner and its tests.
///
/// A future command can persist the planned records in one database
/// transaction; the collision and graph-rewrite rules do not depend on a
/// particular repository implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PortableCatalog {
    entities: BTreeMap<String, PortableEntity>,
}

impl PortableCatalog {
    pub fn get(&self, id: &str) -> Option<&PortableEntity> {
        self.entities.get(id)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn import(
        &mut self,
        graph: &PortableGraph,
        mode: ImportMode,
    ) -> Result<ImportReport, BackupError> {
        validate_graph(graph)?;
        match mode {
            ImportMode::Restore | ImportMode::Merge => self.preserve_ids(graph),
            ImportMode::Clone => self.clone_graph(graph),
        }
    }

    fn preserve_ids(&mut self, graph: &PortableGraph) -> Result<ImportReport, BackupError> {
        // Plan first so a late conflict cannot partially mutate the catalog.
        for entity in &graph.entities {
            if let Some(existing) = self.entities.get(&entity.id)
                && existing != entity
            {
                return Err(BackupError::Conflict {
                    entity_type: entity.entity_type.clone(),
                    id: entity.id.clone(),
                });
            }
        }
        let mut report = ImportReport::default();
        for entity in &graph.entities {
            if self.entities.contains_key(&entity.id) {
                report.skipped_identical += 1;
            } else {
                self.entities.insert(entity.id.clone(), entity.clone());
                report.inserted += 1;
            }
        }
        Ok(report)
    }

    fn clone_graph(&mut self, graph: &PortableGraph) -> Result<ImportReport, BackupError> {
        let source_ids: BTreeSet<&str> =
            graph.entities.iter().map(|entity| entity.id.as_str()).collect();
        let mut remap = BTreeMap::new();
        for entity in &graph.entities {
            let new_id = loop {
                let candidate = generate_prefixed_id(&entity.id_prefix);
                if !self.entities.contains_key(&candidate)
                    && !remap.values().any(|value| value == &candidate)
                {
                    break candidate;
                }
            };
            remap.insert(entity.id.clone(), new_id);
        }

        let mut cloned = Vec::with_capacity(graph.entities.len());
        for entity in &graph.entities {
            let mut entity = entity.clone();
            entity.id = remap[&entity.id].clone();
            for (pointer, reference) in &mut entity.references {
                remap_reference_value(reference, &source_ids, &remap, pointer)?;
                let payload_reference = entity.payload.pointer_mut(pointer).ok_or_else(|| {
                    BackupError::InvalidGraph(format!(
                        "{} {} declares reference pointer {pointer}, but payload has no such value",
                        entity.entity_type, entity.id
                    ))
                })?;
                remap_reference_value(payload_reference, &source_ids, &remap, pointer)?;
                if payload_reference != reference {
                    return Err(BackupError::InvalidGraph(format!(
                        "{} {} reference {pointer} disagrees with its payload value after remap",
                        entity.entity_type, entity.id
                    )));
                }
            }
            cloned.push(entity);
        }
        for entity in cloned {
            self.entities.insert(entity.id.clone(), entity);
        }
        Ok(ImportReport {
            inserted: graph.entities.len(),
            skipped_identical: 0,
            remap,
        })
    }
}

fn validate_graph(graph: &PortableGraph) -> Result<(), BackupError> {
    let mut ids = BTreeSet::new();
    for entity in &graph.entities {
        entity.validate()?;
        if !ids.insert(entity.id.as_str()) {
            return Err(BackupError::InvalidGraph(format!(
                "duplicate entity id {}",
                entity.id
            )));
        }
    }
    for entity in &graph.entities {
        for (pointer, reference) in &entity.references {
            if !pointer.starts_with('/') {
                return Err(BackupError::InvalidGraph(format!(
                    "{} {} reference key {pointer:?} must be an RFC 6901 JSON Pointer",
                    entity.entity_type, entity.id,
                )));
            }
            validate_reference_value(entity, pointer, reference)?;
            let payload_reference = entity.payload.pointer(pointer).ok_or_else(|| {
                BackupError::InvalidGraph(format!(
                    "{} {} declares reference pointer {pointer}, but payload has no such value",
                    entity.entity_type, entity.id
                ))
            })?;
            if payload_reference != reference {
                return Err(BackupError::InvalidGraph(format!(
                    "{} {} reference {pointer} disagrees with its payload value",
                    entity.entity_type, entity.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_reference_value(
    entity: &PortableEntity,
    pointer: &str,
    value: &Value,
) -> Result<(), BackupError> {
    let mut leaf_count = 0usize;
    visit_reference_strings(value, &mut |target| {
        leaf_count += 1;
        validate_runtime_prefixed_id(target, None).map_err(|_| {
            BackupError::InvalidGraph(format!(
                "{} {} has invalid entity reference {target:?} at {pointer}",
                entity.entity_type, entity.id
            ))
        })
    })?;
    if leaf_count == 0 {
        return Err(BackupError::InvalidGraph(format!(
            "{} {} reference {pointer} contains no entity IDs",
            entity.entity_type, entity.id
        )));
    }
    Ok(())
}

fn visit_reference_strings(
    value: &Value,
    visitor: &mut impl FnMut(&str) -> Result<(), BackupError>,
) -> Result<(), BackupError> {
    match value {
        Value::String(target) => visitor(target),
        Value::Array(values) => {
            for value in values {
                visit_reference_strings(value, visitor)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for value in values.values() {
                visit_reference_strings(value, visitor)?;
            }
            Ok(())
        }
        _ => Err(BackupError::InvalidGraph(
            "declared references may contain only strings, arrays, and objects".into(),
        )),
    }
}

fn remap_reference_value(
    value: &mut Value,
    source_ids: &BTreeSet<&str>,
    remap: &BTreeMap<String, String>,
    pointer: &str,
) -> Result<(), BackupError> {
    match value {
        Value::String(target) => {
            if source_ids.contains(target.as_str()) {
                *target = remap.get(target).cloned().ok_or_else(|| {
                    BackupError::InvalidGraph(format!(
                        "reference {pointer} to {target} has no clone remap"
                    ))
                })?;
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                remap_reference_value(value, source_ids, remap, pointer)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                remap_reference_value(value, source_ids, remap, pointer)?;
            }
            Ok(())
        }
        _ => Err(BackupError::InvalidGraph(format!(
            "reference {pointer} contains a non-string scalar"
        ))),
    }
}

fn sibling_staging_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup");
    destination.with_file_name(format!(".{name}.staging-{}", uuid::Uuid::now_v7()))
}

fn create_sibling_staging(destination: &Path) -> Result<PathBuf, BackupError> {
    for _ in 0..16 {
        let staging = sibling_staging_path(destination);
        match create_private_directory(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(BackupError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique sibling staging directory",
    )))
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::create_dir(path)
}

fn path_exists_no_follow(path: &Path) -> Result<bool, BackupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn install_staging_directory(staging: &Path, destination: &Path) -> Result<(), BackupError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let staging = CString::new(staging.as_os_str().as_bytes()).map_err(|_| {
            BackupError::InvalidManifest("staging path contains a NUL byte".into())
        })?;
        let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            BackupError::InvalidManifest("destination path contains a NUL byte".into())
        })?;
        // RENAME_NOREPLACE is the atomic directory install primitive needed
        // here: a concurrent creator must never be overwritten.
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                staging.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(());
        }
        return Err(std::io::Error::last_os_error().into());
    }

    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let staging = CString::new(staging.as_os_str().as_bytes()).map_err(|_| {
            BackupError::InvalidManifest("staging path contains a NUL byte".into())
        })?;
        let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            BackupError::InvalidManifest("destination path contains a NUL byte".into())
        })?;
        let result = unsafe {
            libc::renamex_np(
                staging.as_ptr(),
                destination.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if result == 0 {
            return Ok(());
        }
        return Err(std::io::Error::last_os_error().into());
    }

    #[cfg(windows)]
    {
        // MoveFile semantics used by std::fs::rename are no-replace.
        fs::rename(staging, destination)?;
        Ok(())
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        windows
    )))]
    {
        let _ = (staging, destination);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this platform has no configured atomic no-replace directory install primitive",
        )))
    }
}

fn validate_relative_bundle_path(path: &str) -> Result<(), BackupError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackupError::InvalidManifest(format!(
            "invalid bundle path: {}",
            path.display()
        )));
    }
    if path
        .components()
        .any(|component| component.as_os_str().to_str().is_none())
    {
        return Err(BackupError::InvalidManifest(
            "bundle paths must be valid UTF-8".into(),
        ));
    }
    Ok(())
}

fn normalize_bundle_path(path: &Path) -> Result<String, BackupError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackupError::UnsafeSource(format!(
            "non-portable relative path: {}",
            path.display()
        )));
    }
    let components: Result<Vec<_>, _> = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| {
                    BackupError::UnsafeSource(format!(
                        "backup path is not valid UTF-8: {}",
                        path.display()
                    ))
                })
        })
        .collect();
    Ok(components?.join("/"))
}

fn copy_optional_regular_file(
    source: &Path,
    destination: &Path,
    bundle_path: &str,
    files: &mut Vec<BackupFileEntry>,
) -> Result<(), BackupError> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    reject_special_file(source, &metadata)?;
    if !metadata.is_file() {
        return Err(BackupError::UnsafeSource(format!(
            "expected a regular file at {}",
            source.display()
        )));
    }
    if metadata.len() > MAX_BACKUP_FILE_BYTES {
        return Err(BackupError::UnsafeSource(format!(
            "{} is too large to back up ({} bytes)",
            source.display(),
            metadata.len()
        )));
    }
    copy_regular_file_bounded(source, destination, metadata.len())?;
    files.push(file_entry(destination, bundle_path)?);
    enforce_file_collection_limits(files)
}

fn reject_backup_output_overlap(
    destination: &Path,
    source: BackupSource<'_>,
) -> Result<(), BackupError> {
    validate_backup_source_roots(source)?;
    let destination_parent = destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    for root in [source.data_dir, source.work_dir] {
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let canonical = root.canonicalize()?;
        if destination_parent.starts_with(&canonical) {
            return Err(BackupError::UnsafeSource(format!(
                "backup output must not overlap source root: {}",
                root.display()
            )));
        }
    }
    Ok(())
}

fn copy_optional_tree(
    source_root: &Path,
    destination_root: &Path,
    bundle_root: &str,
    directories: &mut Vec<String>,
    files: &mut Vec<BackupFileEntry>,
) -> Result<(), BackupError> {
    let metadata = match fs::symlink_metadata(source_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    reject_special_file(source_root, &metadata)?;
    if !metadata.is_dir() {
        return Err(BackupError::UnsafeSource(format!(
            "expected a regular directory at {}",
            source_root.display()
        )));
    }
    if let Some(parent) = destination_root.parent() {
        fs::create_dir_all(parent)?;
    }
    create_private_directory(destination_root)?;
    directories.push(bundle_root.to_owned());
    enforce_directory_collection_limits(directories)?;

    let mut pending = vec![(source_root.to_path_buf(), PathBuf::new())];
    while let Some((directory, relative_directory)) = pending.pop() {
        let mut entries: Vec<_> = fs::read_dir(&directory)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let source_path = entry.path();
            let relative_path = relative_directory.join(entry.file_name());
            let metadata = fs::symlink_metadata(&source_path)?;
            reject_special_file(&source_path, &metadata)?;
            if metadata.is_dir() {
                let relative_portable = normalize_bundle_path(&relative_path)?;
                let bundle_path = format!("{bundle_root}/{relative_portable}");
                validate_relative_bundle_path(&bundle_path)?;
                create_private_directory(&destination_root.join(&relative_path))?;
                directories.push(bundle_path);
                enforce_directory_collection_limits(directories)?;
                pending.push((source_path, relative_path));
                continue;
            }
            if !metadata.is_file() {
                return Err(BackupError::UnsafeSource(format!(
                    "backup tree contains a non-regular file: {}",
                    source_path.display()
                )));
            }
            if metadata.len() > MAX_BACKUP_FILE_BYTES {
                return Err(BackupError::UnsafeSource(format!(
                    "{} is too large to back up ({} bytes)",
                    source_path.display(),
                    metadata.len()
                )));
            }
            let relative_portable = normalize_bundle_path(&relative_path)?;
            let bundle_path = format!("{bundle_root}/{relative_portable}");
            validate_relative_bundle_path(&bundle_path)?;
            let destination = destination_root.join(&relative_path);
            copy_regular_file_bounded(&source_path, &destination, metadata.len())?;
            files.push(file_entry(&destination, &bundle_path)?);
            enforce_file_collection_limits(files)?;
        }
    }
    Ok(())
}

fn reject_special_file(path: &Path, metadata: &fs::Metadata) -> Result<(), BackupError> {
    if metadata.file_type().is_symlink() || metadata_has_reparse_point(metadata) {
        return Err(BackupError::UnsafeSource(format!(
            "symlinks/reparse points are not allowed in backup sources: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_has_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_has_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn enforce_file_collection_limits(files: &[BackupFileEntry]) -> Result<(), BackupError> {
    if files.len() > MAX_BACKUP_FILES {
        return Err(BackupError::UnsafeSource(format!(
            "backup source contains more than {MAX_BACKUP_FILES} files"
        )));
    }
    let total = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.bytes)
            .ok_or_else(|| BackupError::UnsafeSource("backup byte total overflowed".into()))
    })?;
    if total > MAX_BACKUP_TOTAL_BYTES {
        return Err(BackupError::UnsafeSource(format!(
            "backup source exceeds {MAX_BACKUP_TOTAL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn enforce_directory_collection_limits(directories: &[String]) -> Result<(), BackupError> {
    if directories.len() > MAX_BACKUP_DIRECTORIES {
        return Err(BackupError::UnsafeSource(format!(
            "backup source contains more than {MAX_BACKUP_DIRECTORIES} directories"
        )));
    }
    Ok(())
}

fn copy_regular_file_bounded(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
) -> Result<(), BackupError> {
    if expected_bytes > MAX_BACKUP_FILE_BYTES {
        return Err(BackupError::InvalidManifest(format!(
            "refusing to copy an oversized backup file: {} bytes",
            expected_bytes
        )));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let source_metadata = fs::symlink_metadata(source)?;
    reject_special_file(source, &source_metadata)?;
    if !source_metadata.is_file() || source_metadata.len() != expected_bytes {
        return Err(BackupError::UnsafeSource(format!(
            "backup file changed type or size while being copied: {}",
            source.display()
        )));
    }
    let mut reader = open_regular_file_no_follow(source)?;
    let opened_metadata = reader.metadata()?;
    reject_special_file(source, &opened_metadata)?;
    if !opened_metadata.is_file() || opened_metadata.len() != expected_bytes {
        return Err(BackupError::UnsafeSource(format!(
            "backup file changed type or size while being opened: {}",
            source.display()
        )));
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut writer = options.open(destination)?;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut copied = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied.checked_add(read as u64).ok_or_else(|| {
            BackupError::InvalidManifest("copied byte count overflowed".into())
        })?;
        if copied > expected_bytes || copied > MAX_BACKUP_FILE_BYTES {
            return Err(BackupError::UnsafeSource(format!(
                "backup file grew while being copied: {}",
                source.display()
            )));
        }
        writer.write_all(&buffer[..read])?;
    }
    if copied != expected_bytes {
        return Err(BackupError::UnsafeSource(format!(
            "backup file changed size while being copied: {}",
            source.display()
        )));
    }
    writer.sync_all()?;
    Ok(())
}

fn resolve_bundle_file(bundle_dir: &Path, relative_path: &str) -> Result<PathBuf, BackupError> {
    validate_relative_bundle_path(relative_path)?;
    let mut current = bundle_dir.to_path_buf();
    for component in Path::new(relative_path).components() {
        let Component::Normal(component) = component else {
            return Err(BackupError::InvalidManifest(format!(
                "invalid bundle path: {relative_path}"
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() || metadata_has_reparse_point(&metadata) {
            return Err(BackupError::InvalidManifest(format!(
                "bundle path crosses a symlink/reparse point: {relative_path}"
            )));
        }
    }
    Ok(current)
}

fn verify_bundle_tree(
    bundle_dir: &Path,
    manifest_paths: &BTreeSet<&str>,
    manifest_directories: &BTreeSet<&str>,
) -> Result<(), BackupError> {
    let mut pending = vec![(bundle_dir.to_path_buf(), PathBuf::new())];
    let mut regular_files = 0_usize;
    let mut seen_directories = BTreeSet::new();
    while let Some((directory, relative_directory)) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative_path = relative_directory.join(entry.file_name());
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || metadata_has_reparse_point(&metadata) {
                return Err(BackupError::InvalidManifest(format!(
                    "bundle contains a symlink/reparse point: {}",
                    relative_path.display()
                )));
            }
            if metadata.is_dir() {
                let portable = normalize_bundle_path(&relative_path).map_err(|error| {
                    BackupError::InvalidManifest(format!(
                        "invalid bundle directory path: {error}"
                    ))
                })?;
                let structural_container = portable == BUNDLE_DATA_DIR || portable == BUNDLE_WORK_DIR;
                if !structural_container && !manifest_directories.contains(portable.as_str()) {
                    return Err(BackupError::InvalidManifest(format!(
                        "bundle contains undeclared directory: {portable}"
                    )));
                }
                if !structural_container {
                    seen_directories.insert(portable);
                }
                pending.push((path, relative_path));
                continue;
            }
            if !metadata.is_file() {
                return Err(BackupError::InvalidManifest(format!(
                    "bundle contains a non-regular file: {}",
                    relative_path.display()
                )));
            }
            regular_files += 1;
            if regular_files > MAX_BACKUP_FILES + 1 {
                return Err(BackupError::InvalidManifest(
                    "bundle contains too many files".into(),
                ));
            }
            let portable = normalize_bundle_path(&relative_path).map_err(|error| {
                BackupError::InvalidManifest(format!("invalid bundle file path: {error}"))
            })?;
            if portable != MANIFEST_FILE && !manifest_paths.contains(portable.as_str()) {
                return Err(BackupError::InvalidManifest(format!(
                    "bundle contains undeclared file: {portable}"
                )));
            }
        }
    }
    if seen_directories.len() != manifest_directories.len() {
        let missing = manifest_directories
            .iter()
            .find(|directory| !seen_directories.contains(**directory))
            .copied()
            .unwrap_or("<unknown>");
        return Err(BackupError::InvalidManifest(format!(
            "manifest declares a missing directory: {missing}"
        )));
    }
    Ok(())
}

fn ensure_existing_directory_is_safe(path: &Path, label: &str) -> Result<(), BackupError> {
    walk_directory_components(path, label, false)
}

fn create_directory_tree_safe(path: &Path, label: &str) -> Result<(), BackupError> {
    walk_directory_components(path, label, true)
}

fn walk_directory_components(
    path: &Path,
    label: &str,
    create_missing: bool,
) -> Result<(), BackupError> {
    let absolute = if path.as_os_str().is_empty() {
        std::env::current_dir()?
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                current.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                current.pop();
            }
            Component::Normal(value) => {
                current.push(value);
                let metadata = match fs::symlink_metadata(&current) {
                    Ok(metadata) => metadata,
                    Err(error)
                        if create_missing && error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        match create_private_directory(&current) {
                            Ok(()) => fs::symlink_metadata(&current)?,
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                                fs::symlink_metadata(&current)?
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    Err(error) => return Err(error.into()),
                };
                if metadata.file_type().is_symlink()
                    || metadata_has_reparse_point(&metadata)
                    || !metadata.is_dir()
                {
                    return Err(BackupError::InvalidManifest(format!(
                        "{label} crosses a symlink/reparse point or non-directory at {}",
                        current.display()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn restore_relative_directory(bundle_path: &str) -> Result<PathBuf, BackupError> {
    let companion_root = format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}");
    let workspace_root = format!("{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}");
    if bundle_path == companion_root || bundle_path.starts_with(&format!("{companion_root}/")) {
        let suffix = bundle_path
            .strip_prefix(&format!("{BUNDLE_DATA_DIR}/"))
            .expect("guarded prefix");
        validate_relative_bundle_path(suffix)?;
        return Ok(PathBuf::from(suffix));
    }
    if bundle_path == workspace_root || bundle_path.starts_with(&format!("{workspace_root}/")) {
        let suffix = bundle_path
            .strip_prefix(&format!("{BUNDLE_WORK_DIR}/"))
            .expect("guarded prefix");
        validate_relative_bundle_path(suffix)?;
        return Ok(PathBuf::from(suffix));
    }
    Err(BackupError::InvalidManifest(format!(
        "unsupported restore directory path: {bundle_path}"
    )))
}

fn restore_relative_path(bundle_path: &str) -> Result<PathBuf, BackupError> {
    match bundle_path {
        DATABASE_FILE => Ok(PathBuf::from("nomifun-backend.db")),
        path if path == format!("{BUNDLE_DATA_DIR}/{ENCRYPTION_KEY_FILE}") => {
            Ok(PathBuf::from(ENCRYPTION_KEY_FILE))
        }
        path if path.starts_with(&format!("{BUNDLE_DATA_DIR}/{COMPANION_DIR}/")) => {
            let suffix = path
                .strip_prefix(&format!("{BUNDLE_DATA_DIR}/"))
                .expect("guarded prefix");
            validate_relative_bundle_path(suffix)?;
            Ok(PathBuf::from(suffix))
        }
        path if path.starts_with(&format!(
            "{BUNDLE_WORK_DIR}/{MANAGED_WORKSPACES_DIR}/"
        )) =>
        {
            let suffix = path
                .strip_prefix(&format!("{BUNDLE_WORK_DIR}/"))
                .expect("guarded prefix");
            validate_relative_bundle_path(suffix)?;
            Ok(PathBuf::from(suffix))
        }
        _ => Err(BackupError::InvalidManifest(format!(
            "unsupported restore payload path: {bundle_path}"
        ))),
    }
}

fn write_synced_file(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory_best_effort(path: &Path) {
    if let Ok(directory) = fs::File::open(path) {
        let _ = directory.sync_all();
    }
}

fn remove_staging_dir(path: &Path) -> Result<(), BackupError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink()
        || metadata_has_reparse_point(&metadata)
        || !metadata.is_dir()
    {
        return Err(BackupError::InvalidManifest(format!(
            "refusing to remove unsafe staging path: {}",
            path.display()
        )));
    }
    fs::remove_dir_all(path)?;
    Ok(())
}

fn validate_runtime_prefixed_id(
    value: &str,
    expected_prefix: Option<&str>,
) -> Result<(), BackupError> {
    let Some((prefix, uuid_body)) = value.split_once('_') else {
        return Err(BackupError::InvalidGraph(format!(
            "entity ID has no prefix separator: {value}"
        )));
    };
    validate_id_prefix(prefix).map_err(|error| BackupError::InvalidGraph(error.to_string()))?;
    if expected_prefix.is_some_and(|expected| prefix != expected) {
        return Err(BackupError::InvalidGraph(format!(
            "entity ID {value} has the wrong prefix"
        )));
    }
    let uuid = uuid::Uuid::parse_str(uuid_body)
        .map_err(|_| BackupError::InvalidGraph(format!("invalid entity ID {value}")))?;
    if uuid.get_version() != Some(uuid::Version::SortRand)
        || uuid.get_variant() != uuid::Variant::RFC4122
        || uuid.hyphenated().to_string() != uuid_body
    {
        return Err(BackupError::InvalidGraph(format!(
            "non-canonical UUIDv7 entity ID {value}"
        )));
    }
    Ok(())
}

fn file_entry(path: &Path, relative_path: &str) -> Result<BackupFileEntry, BackupError> {
    Ok(BackupFileEntry {
        path: relative_path.to_owned(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn open_regular_file_no_follow(path: &Path) -> Result<fs::File, BackupError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    reject_special_file(path, &metadata)?;
    if !metadata.is_file() {
        return Err(BackupError::UnsafeSource(format!(
            "expected a regular file at {}",
            path.display()
        )));
    }
    Ok(file)
}

fn sha256_file(path: &Path) -> Result<String, BackupError> {
    let mut reader = BufReader::new(open_regular_file_no_follow(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), BackupError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| BackupError::InvalidManifest(error.to_string()))?;
    let tmp = path.with_extension("tmp");
    write_synced_file(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<(), BackupError> {
    Ok(())
}
