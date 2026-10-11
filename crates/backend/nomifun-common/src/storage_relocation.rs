//! Relocate one current dataset without changing its identity or resetting data.
//!
//! The fixed control directory owns the only durable plan and commit pointer.
//! The application holds both source work-root locks, closes every writer, then
//! calls prepare. Its returned guard retains the control and destination locks
//! while the application validates/rebinds the copied database. Only the main
//! database may change at that boundary; every other copied payload is checked
//! again before publication. No source directory is deleted, and external
//! project directories are never traversed (only work/conversations is owned).

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic_file::{rename_noreplace, replace_file, sync_directory, write_new_and_publish};
use crate::dataset_roots::{WORK_ROOT_BINDING_FILE, WORK_ROOT_OWNER_FILE};
use crate::factory_reset::{self, DatasetReceipt};
use crate::{AppError, paths, validate_uuidv7};

const VERSION: u32 = 1;
const PLAN_FILE: &str = "storage-relocation.pending.json";
const LAST_FILE: &str = "storage-relocation.last.json";
const LOCATOR_FILE: &str = "storage-location.json";
const LOCATION_MARKER: &str = ".nomifun-storage-location.json";
const SEALED_MARKER: &str = ".nomifun-storage-relocated.json";
const STAGING_MARKER: &str = ".nomifun-storage-staging.json";
const CONTROL_LOCK: &str = ".storage-relocation.lock";
const MAX_CONTROL_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageLocation {
    pub data_dir: PathBuf,
    pub work_dir: PathBuf,
    pub generation: String,
    /// First storage root: fixed control scope/native UI profile identity only.
    /// Never used as an active filesystem location or an authorization scope.
    pub origin_data_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelocationPhase {
    Prepared,
    AwaitingRestart,
    Copying,
    Verifying,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelocationPlan {
    version: u32,
    pub operation_id: String,
    pub source: StorageLocation,
    pub destination: StorageLocation,
    pub control_dir: PathBuf,
    pub staging_data_dir: PathBuf,
    pub file_count: u64,
    pub directory_count: u64,
    pub total_bytes: u64,
    pub phase: RelocationPhase,
    pub cancellable: bool,
    pub error: Option<String>,
    copied: Option<Vec<CopyEntry>>,
    installed: Option<Vec<CopyEntry>>,
}

/// Keep this value alive until commit completes. The application must validate
/// staging_data_dir's SQLite schema, canonical bindings and domain references
/// before calling commit; storage relocation never grants startup-reset rights.
#[derive(Debug)]
pub struct PreparedRelocation {
    pub plan: RelocationPlan,
    pub source: StorageLocation,
    pub destination: StorageLocation,
    pub staging_data_dir: PathBuf,
    /// Final inventory was already committed before directory publication.
    /// Recovery must skip database mutation and finish the same pointer commit.
    pub preparation_complete: bool,
    _control_lock: File,
    _target_lock: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CopyBase { Data, Work }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyEntry {
    base: CopyBase,
    relative: PathBuf,
    directory: bool,
    bytes: u64,
    digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Locator { version: u32, operation_id: String, location: StorageLocation }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootDescriptor {
    version: u32,
    operation_id: String,
    generation: String,
    control_dir: PathBuf,
    data_dir: PathBuf,
    destination: PathBuf,
}

fn io_error(action: &str, path: &Path, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("storage relocation: {action} {}: {error}", path.display()))
}

fn validate_absolute(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute() || path.as_os_str().is_empty()
        || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || crate::workspace_path_has_edge_whitespace_segment(path)
    {
        return Err(AppError::BadRequest(format!("storage location must be a safe absolute path: {}", path.display())));
    }
    Ok(())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

fn require_no_links(path: &Path) -> Result<(), AppError> {
    validate_absolute(path)?;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if is_link(&metadata) => return Err(AppError::BadRequest(format!("storage location cannot traverse a symbolic link or reparse point: {}", ancestor.display()))),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("inspect path", ancestor, error)),
        }
    }
    Ok(())
}

fn real_directory(path: &Path, create: bool) -> Result<PathBuf, AppError> {
    require_no_links(path)?;
    if create { fs::create_dir_all(path).map_err(|e| io_error("create directory", path, e))?; }
    require_no_links(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|e| io_error("inspect directory", path, e))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(AppError::BadRequest(format!("storage location is not a real directory: {}", path.display())));
    }
    paths::canonicalize_simplified(path).map_err(|e| io_error("canonicalize directory", path, e))
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    child.ancestors().any(|ancestor| paths::paths_equivalent(parent, ancestor))
        || {
            #[cfg(windows)]
            { let parent = parent.to_string_lossy().replace('/', "\\").to_lowercase();
              let child = child.to_string_lossy().replace('/', "\\").to_lowercase();
              child == parent || child.starts_with(&format!("{}\\", parent.trim_end_matches('\\'))) }
            #[cfg(not(windows))]
            { child.starts_with(parent) }
        }
}

fn require_disjoint(a: &Path, b: &Path) -> Result<(), AppError> {
    if path_contains(a, b) || path_contains(b, a) {
        return Err(AppError::BadRequest(format!("storage relocation paths must not contain one another: {} and {}", a.display(), b.display())));
    }
    Ok(())
}

fn read_regular(path: &Path, maximum: u64) -> Result<Vec<u8>, AppError> {
    let mut options = OpenOptions::new(); options.read(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    #[cfg(windows)]
    { use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x00200000); }
    let mut file = options.open(path).map_err(|e| io_error("open control file", path, e))?;
    let metadata = file.metadata().map_err(|e| io_error("inspect control file", path, e))?;
    if !metadata.is_file() || is_link(&metadata) || metadata.len() > maximum {
        return Err(AppError::Internal(format!("storage relocation control is not a bounded regular file: {}", path.display())));
    }
    let mut bytes = Vec::new(); Read::by_ref(&mut file).take(maximum + 1).read_to_end(&mut bytes).map_err(|e| io_error("read control file", path, e))?;
    if bytes.len() as u64 > maximum { return Err(io_error("read control file", path, "file grew beyond limit")); }
    Ok(bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, AppError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error("inspect control file", path, e)),
        Ok(_) => {}
    }
    serde_json::from_slice(&read_regular(path, MAX_CONTROL_BYTES)?).map(Some).map_err(|e| io_error("parse control file", path, e))
}

fn write_json(path: &Path, value: &impl Serialize, replace: bool) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| io_error("serialize control file", path, e))?;
    if bytes.len() as u64 > MAX_CONTROL_BYTES { return Err(io_error("write control file", path, "manifest exceeds size limit")); }
    let temp = path.with_file_name(format!(".storage-write-{}.tmp", Uuid::now_v7()));
    write_new_and_publish(&temp, path, &bytes, if replace { replace_file } else { rename_noreplace })
        .map_err(|e| io_error("publish control file", path, e))
}

fn lock_file(path: &Path) -> Result<File, AppError> {
    require_no_links(path)?;
    let mut options = OpenOptions::new(); options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x00200000); }
    let file = options.open(path).map_err(|e| io_error("open lifecycle lock", path, e))?;
    let metadata = file.metadata().map_err(|e| io_error("inspect opened lifecycle lock", path, e))?;
    if !metadata.is_file() || is_link(&metadata) { return Err(io_error("validate lifecycle lock", path, "not a regular file")); }
    file.try_lock_exclusive().map_err(|e| AppError::Conflict(format!("storage relocation is already in progress for {}: {e}", path.display())))?;
    Ok(file)
}

fn control_lock(control: &Path) -> Result<(PathBuf, File), AppError> {
    let control = real_directory(control, true)?;
    let lock = lock_file(&control.join(CONTROL_LOCK))?;
    Ok((control, lock))
}

fn same_location(a: &StorageLocation, b: &StorageLocation) -> bool {
    a.generation == b.generation && paths::paths_equivalent(&a.data_dir, &b.data_dir)
        && paths::paths_equivalent(&a.work_dir, &b.work_dir)
}

fn source_location(data: &Path, work: &Path) -> Result<StorageLocation, AppError> {
    let data_dir = real_directory(data, false)?;
    let work_dir = real_directory(work, false)?;
    require_storage_root_not_sealed(&data_dir)?;
    factory_reset::require_current_v3_dataset_for_work_dir(&data_dir, &work_dir)?;
    let receipt: DatasetReceipt = read_json(&data_dir.join(factory_reset::V3_DATASET_RECEIPT_FILE))?.ok_or_else(|| io_error("read source receipt", &data_dir, "missing"))?;
    Ok(StorageLocation { origin_data_dir: data_dir.clone(), data_dir, work_dir, generation: receipt.generation })
}

fn target_path(target: &Path) -> Result<PathBuf, AppError> {
    require_no_links(target)?;
    target.file_name().ok_or_else(|| AppError::BadRequest("storage target cannot be a filesystem root".into()))?;
    canonical_prospective_path(target)
}

// Resolve existing ancestors without creating directories. In particular a
// rejected overlapping target must not create folders inside the source.
fn canonical_prospective_path(path: &Path) -> Result<PathBuf, AppError> {
    require_no_links(path)?;
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = real_directory(ancestor, false)?;
                for name in missing.into_iter().rev() { resolved.push(name); }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(ancestor.file_name().ok_or_else(|| AppError::BadRequest("storage location has no existing filesystem ancestor".into()))?.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| AppError::BadRequest("storage location has no existing filesystem ancestor".into()))?;
            }
            Err(error) => return Err(io_error("inspect prospective location", ancestor, error)),
        }
    }
}

fn require_disk_space(target: &Path, bytes: u64) -> Result<(), AppError> {
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("storage target has no parent".into()))?;
    let available = fs2::available_space(parent).map_err(|e| io_error("inspect target free space", parent, e))?;
    if available < bytes {
        return Err(AppError::BadRequest(format!("storage target has insufficient free space: needs at least {bytes} bytes, available {available} bytes")));
    }
    Ok(())
}

fn require_empty_target(target: &Path) -> Result<(), AppError> {
    require_no_links(target)?;
    match fs::symlink_metadata(target) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_error("inspect target", target, e)),
        Ok(m) if !m.is_dir() || is_link(&m) => return Err(AppError::BadRequest(format!("storage target is not a real directory: {}", target.display()))),
        Ok(_) => {}
    }
    for marker in [WORK_ROOT_OWNER_FILE, WORK_ROOT_BINDING_FILE, factory_reset::V3_DATASET_RECEIPT_FILE, LOCATION_MARKER, SEALED_MARKER] {
        if fs::symlink_metadata(target.join(marker)).is_ok() {
            return Err(AppError::Conflict(format!("storage target {} belongs to an existing NomiFun dataset; choose an empty folder or a new subfolder", target.display())));
        }
    }
    if fs::read_dir(target).map_err(|e| io_error("inspect target contents", target, e))?.next().is_some() {
        return Err(AppError::Conflict(format!("storage target {} is not empty; its files will not be overwritten or merged", target.display())));
    }
    Ok(())
}

fn validate_plan(plan: &RelocationPlan, control: &Path) -> Result<(), AppError> {
    if plan.version != VERSION || validate_uuidv7(&plan.operation_id).is_err()
        || validate_uuidv7(&plan.source.generation).is_err()
        || plan.source.generation != plan.destination.generation
        || !paths::paths_equivalent(&plan.control_dir, control)
        || !paths::paths_equivalent(&plan.destination.data_dir, &plan.destination.work_dir)
    { return Err(AppError::Internal("invalid storage relocation plan identity".into())); }
    for path in [&plan.source.data_dir, &plan.source.work_dir, &plan.destination.data_dir, &plan.staging_data_dir] { validate_absolute(path)?; }
    validate_absolute(&plan.source.origin_data_dir)?;
    if plan.source.origin_data_dir != plan.destination.origin_data_dir { return Err(AppError::Internal("storage relocation changed its original profile identity".into())); }
    require_disjoint(control, &plan.source.data_dir)?;
    require_disjoint(control, &plan.source.work_dir)?;
    require_disjoint(control, &plan.destination.data_dir)?;
    require_disjoint(&plan.source.data_dir, &plan.destination.data_dir)?;
    require_disjoint(&plan.source.work_dir, &plan.destination.data_dir)?;
    let expected = plan.destination.data_dir.parent().unwrap().join(format!(".nomifun-storage-move-{}", plan.operation_id));
    if plan.staging_data_dir != expected { return Err(AppError::Internal("storage relocation staging path does not match its operation".into())); }
    for entries in [plan.copied.as_ref(), plan.installed.as_ref()].into_iter().flatten() {
        for entry in entries {
            if entry.relative.as_os_str().is_empty() || entry.relative.components().any(|c| !matches!(c, Component::Normal(_)))
                || (entry.directory && (entry.digest.is_some() || entry.bytes != 0))
                || (!entry.directory && !entry.digest.as_ref().is_some_and(|d| d.len() == 64 && d.bytes().all(|c| c.is_ascii_hexdigit())))
            { return Err(AppError::Internal("invalid storage relocation manifest entry".into())); }
        }
    }
    Ok(())
}

/// Read only. A malformed locator fails closed rather than selecting a new dataset.
pub fn resolve_storage_location(control: &Path) -> Result<Option<StorageLocation>, AppError> {
    require_no_links(control)?;
    let Some(locator): Option<Locator> = read_json(&control.join(LOCATOR_FILE))? else { return Ok(None); };
    if locator.version != VERSION || validate_uuidv7(&locator.operation_id).is_err() || validate_uuidv7(&locator.location.generation).is_err() {
        return Err(AppError::Internal("invalid storage location pointer".into()));
    }
    validate_absolute(&locator.location.origin_data_dir)?;
    let root = real_directory(&locator.location.data_dir, false)?;
    if !paths::paths_equivalent(&root, &locator.location.work_dir) { return Err(AppError::Internal("storage pointer is not a single-root dataset".into())); }
    let descriptor: RootDescriptor = read_json(&root.join(LOCATION_MARKER))?.ok_or_else(|| io_error("read target location marker", &root, "missing"))?;
    validate_descriptor(&descriptor, &root, control)?;
    if descriptor.operation_id != locator.operation_id || !descriptor_generation_matches(&descriptor, &locator.location)? {
        return Err(AppError::Internal("storage pointer and target location identity disagree".into()));
    }
    Ok(Some(locator.location))
}

fn descriptor_generation_matches(descriptor: &RootDescriptor, location: &StorageLocation) -> Result<bool, AppError> {
    if descriptor.generation == location.generation { return Ok(true); }
    // A real factory reset/destructive release rotates this dataset's
    // generation at the same location. Its fully finalized receipt and native
    // owner binding, never an arbitrary configuration value, prove the change.
    factory_reset::require_current_v3_dataset_for_work_dir(&location.data_dir, &location.work_dir)?;
    let receipt: DatasetReceipt = read_json(&location.data_dir.join(factory_reset::V3_DATASET_RECEIPT_FILE))?.ok_or_else(|| AppError::Internal("missing finalized storage receipt".into()))?;
    Ok(receipt.generation == location.generation)
}

/// Call only after the app's authoritative DB probe, all-service initialization
/// and finalized reset/receipt. This keeps the same fixed location and profile
/// identity when a real reset rotates generation; relocation itself never does.
pub fn refresh_current_storage_location(control: &Path, data_dir: &Path) -> Result<(), AppError> {
    let (control, _lock) = control_lock(control)?;
    let Some(mut locator): Option<Locator> = read_json(&control.join(LOCATOR_FILE))? else { return Ok(()); };
    if !paths::paths_equivalent(&locator.location.data_dir, data_dir) { return Err(AppError::Conflict("cannot refresh a storage pointer for another active root".into())); }
    factory_reset::require_current_v3_dataset_for_work_dir(data_dir, &locator.location.work_dir)?;
    let receipt: DatasetReceipt = read_json(&data_dir.join(factory_reset::V3_DATASET_RECEIPT_FILE))?.ok_or_else(|| AppError::Internal("missing finalized receipt for pointer refresh".into()))?;
    if locator.location.generation != receipt.generation {
        locator.location.generation = receipt.generation;
        write_json(&control.join(LOCATOR_FILE), &locator, true)?;
    }
    Ok(())
}

fn validate_descriptor(marker: &RootDescriptor, root: &Path, control: &Path) -> Result<(), AppError> {
    if marker.version != VERSION || validate_uuidv7(&marker.operation_id).is_err() || validate_uuidv7(&marker.generation).is_err()
        || !paths::paths_equivalent(&marker.data_dir, root) || !paths::paths_equivalent(&marker.control_dir, control)
    { return Err(AppError::Internal("invalid storage root descriptor".into())); }
    validate_absolute(&marker.destination)?;
    Ok(())
}

/// Find the original fixed control scope even when a restarted process inherits
/// a relocated root through its environment. Pending plans are legitimate proof
/// during the seal-before-pointer crash window; they never authorize service use.
pub fn resolve_control_dir(root: &Path) -> Result<Option<PathBuf>, AppError> {
    resolve_control_dir_with_initial_scope(root, initial_control_dir)
}

fn resolve_control_dir_with_initial_scope(root: &Path, initial_scope: impl FnOnce(&Path) -> Result<PathBuf, AppError>) -> Result<Option<PathBuf>, AppError> {
    require_no_links(root)?;
    let root = match fs::symlink_metadata(root) {
        Ok(_) => real_directory(root, false)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A removed/offline initial root must not create a fresh dataset.
            // Only its exact fixed scope and verified origin can select the
            // already committed target; no other scopes are searched.
            let control = initial_scope(root)?;
            return resolve_missing_original_scope(root, &control);
        }
        Err(error) => return Err(io_error("inspect storage root", root, error)),
    };
    let marker: Option<RootDescriptor> = read_json(&root.join(SEALED_MARKER))?.or(read_json(&root.join(LOCATION_MARKER))?);
    let Some(marker) = marker else {
        // A crash may occur after the very first draft is durable and before
        // its source descriptor is published. Inspect only this root's exact
        // deterministic scope, never other installations or arbitrary plans.
        let control = initial_scope(&root)?;
        return resolve_initial_scope(&root, &control);
    };
    let control = real_directory(&marker.control_dir, false)?;
    validate_descriptor(&marker, &root, &control)?;
    let locator: Option<Locator> = read_json(&control.join(LOCATOR_FILE))?;
    let pending = pending_storage_relocation(&control)?;
    let completed: Option<RelocationPlan> = read_json(&control.join("completed-storage-relocations").join(format!("{}.json", marker.operation_id)))?;
    let proven_by_plan = pending.as_ref().into_iter().chain(completed.as_ref()).any(|p| {
        p.operation_id == marker.operation_id && p.source.generation == marker.generation
            && paths::paths_equivalent(&p.destination.data_dir, &marker.destination)
            && (paths::paths_equivalent(&p.source.data_dir, &root) || paths::paths_equivalent(&p.destination.data_dir, &root))
    });
    let proven_by_pointer = if let Some(l) = &locator {
        l.version == VERSION && l.operation_id == marker.operation_id && paths::paths_equivalent(&l.location.data_dir, &root)
            && descriptor_generation_matches(&marker, &l.location)?
    } else { false };
    if !proven_by_plan && !proven_by_pointer { return Err(AppError::Internal("storage root descriptor has no matching fixed control proof".into())); }
    if let Some(p) = &completed {
        validate_plan(p, &control)?;
        if p.phase != RelocationPhase::Completed && !(p.phase == RelocationPhase::Canceled && paths::paths_equivalent(&p.source.data_dir, &root)) {
            return Err(AppError::Internal("storage root proof is not a terminal relocation".into()));
        }
    }
    Ok(Some(control))
}

fn original_scope_location(root: &Path, control: &Path) -> Result<Option<StorageLocation>, AppError> {
    match fs::symlink_metadata(control) {
        Ok(_) => { real_directory(control, false)?; }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("inspect original fixed scope", control, error)),
    }
    let Some(location) = resolve_storage_location(control)? else { return Ok(None); };
    if !original_storage_address_matches(&location.origin_data_dir, root) {
        return Err(AppError::Conflict("fixed storage scope belongs to a different original root".into()));
    }
    Ok(Some(location))
}

fn resolve_missing_original_scope(root: &Path, control: &Path) -> Result<Option<PathBuf>, AppError> {
    if original_scope_location(root, control)?.is_some() { return Ok(Some(control.to_path_buf())); }
    if let Some(plan) = pending_storage_relocation(control)? {
        if !original_storage_address_matches(&plan.source.origin_data_dir, root) {
            return Err(AppError::Conflict("fixed storage scope pending operation belongs to a different original root".into()));
        }
        return Err(AppError::Conflict("the original storage root is unavailable and relocation has not committed; preserving its pending operation".into()));
    }
    Ok(None)
}

fn resolve_initial_scope(root: &Path, control: &Path) -> Result<Option<PathBuf>, AppError> {
    if original_scope_location(root, control)?.is_some() {
        // Existing data without a registered descriptor is a separate root,
        // even if it now occupies the original directory's old address.
        return Err(AppError::Conflict("the original storage address now exists without its registered descriptor; refusing to adopt or redirect its data".into()));
    }
    let Some(plan) = pending_storage_relocation(control)? else { return Ok(None); };
    if !paths::paths_equivalent(&plan.source.data_dir, root) { return Ok(None); }
    let current = source_location(&plan.source.data_dir, &plan.source.work_dir)?;
    if !same_location(&current, &plan.source) { return Err(AppError::Conflict("initial storage plan no longer matches its source dataset".into())); }
    Ok(Some(control.to_path_buf()))
}

pub fn control_dir_for_root(root: &Path) -> Result<PathBuf, AppError> {
    if let Some(control) = resolve_control_dir(root)? { return Ok(control); }
    initial_control_dir(root)
}

fn initial_control_dir(root: &Path) -> Result<PathBuf, AppError> {
    validate_absolute(root)?;
    let configuration = dirs::config_dir().ok_or_else(|| AppError::Internal("system configuration directory is unavailable".into()))?;
    initial_control_dir_in_configuration(root, &configuration)
}

fn initial_control_dir_in_configuration(root: &Path, configuration: &Path) -> Result<PathBuf, AppError> {
    validate_absolute(root)?;
    let normalized = if root.exists() { paths::canonicalize_simplified(root).map_err(|e| io_error("canonicalize control scope", root, e))? } else { paths::simplified(root) };
    let key = original_storage_address_key(&normalized);
    let scope = format!("{:x}", Sha256::digest(key.as_bytes()));
    Ok(configuration.join("NomiFun").join("storage").join(scope))
}

// This compares the durable initial address only. An offline root has no OS
// file identity to query; actual target/descriptor identities still use the
// strict filesystem comparison after resolving the committed locator.
fn original_storage_address_key(root: &Path) -> String {
    let normalized = paths::simplified(root);
    #[cfg(windows)]
    { normalized.to_string_lossy().replace('/', "\\").to_lowercase() }
    #[cfg(not(windows))]
    { normalized.to_string_lossy().into_owned() }
}

fn original_storage_address_matches(origin: &Path, requested: &Path) -> bool {
    paths::paths_equivalent(origin, requested) || original_storage_address_key(origin) == original_storage_address_key(requested)
}

pub fn require_storage_root_not_sealed(root: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(root.join(SEALED_MARKER)) {
        Ok(_) => return Err(AppError::Conflict(format!("storage root {} has been relocated and is sealed; use its fixed storage location pointer", root.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error("inspect source seal", root, error)),
    }
    Ok(())
}

pub fn pending_storage_relocation(control: &Path) -> Result<Option<RelocationPlan>, AppError> {
    require_no_links(control)?;
    let plan: Option<RelocationPlan> = read_json(&control.join(PLAN_FILE))?;
    if let Some(plan) = &plan { validate_plan(plan, control)?; }
    Ok(plan)
}

pub fn storage_relocation_status(control: &Path) -> Result<Option<RelocationPlan>, AppError> {
    if let Some(plan) = pending_storage_relocation(control)? { return Ok(Some(plan)); }
    let plan: Option<RelocationPlan> = read_json(&control.join(LAST_FILE))?;
    if let Some(plan) = &plan { validate_plan(plan, control)?; }
    Ok(plan)
}

/// Prepare a reviewable draft. Starting it is a distinct user action and only
/// arms next-boot work; it never copies a running database.
pub fn request_storage_relocation(control: &Path, source_data: &Path, source_work: &Path, target: &Path) -> Result<RelocationPlan, AppError> {
    let mut source = source_location(source_data, source_work)?;
    let target = target_path(target)?;
    let prospective_control = canonical_prospective_path(control)?;
    for root in [&source.data_dir, &source.work_dir, &target] { require_disjoint(&prospective_control, root)?; }
    require_disjoint(&source.data_dir, &target)?; require_disjoint(&source.work_dir, &target)?;
    if resolve_control_dir(&source.data_dir)?.is_some_and(|registered| !paths::paths_equivalent(&registered, &prospective_control)) {
        return Err(AppError::Conflict("storage source already belongs to another fixed control scope".into()));
    }
    let (control, _lock) = control_lock(&prospective_control)?;
    if let Some(existing) = pending_storage_relocation(&control)? {
        if same_location(&existing.source, &source) && paths::paths_equivalent(&existing.destination.data_dir, &target) { return Ok(existing); }
        return Err(AppError::Conflict("another storage relocation is already pending; cancel it before selecting another target".into()));
    }
    if let Some(current) = resolve_storage_location(&control)? {
        if !same_location(&current, &source) { return Err(AppError::Conflict("storage relocation source no longer matches the fixed location pointer".into())); }
        source.origin_data_dir = current.origin_data_dir;
    }
    require_empty_target(&target)?;
    real_directory(target.parent().unwrap(), true)?;
    let operation_id = Uuid::now_v7().to_string();
    let probe = target.parent().unwrap().join(format!(".nomifun-storage-probe-{operation_id}"));
    let file = OpenOptions::new().create_new(true).write(true).open(&probe).map_err(|e| AppError::BadRequest(format!("storage target is not writable: {}: {e}", target.display())))?;
    file.sync_all().map_err(|e| io_error("flush target write probe", &probe, e))?; drop(file);
    fs::remove_file(&probe).map_err(|e| io_error("remove target write probe", &probe, e))?;
    let destination = StorageLocation { data_dir: target.clone(), work_dir: target.clone(), generation: source.generation.clone(), origin_data_dir: source.origin_data_dir.clone() };
    let mut plan = RelocationPlan { version: VERSION, operation_id: operation_id.clone(), source, destination, control_dir: control.clone(), staging_data_dir: target.parent().unwrap().join(format!(".nomifun-storage-move-{operation_id}")), file_count: 0, directory_count: 0, total_bytes: 0, phase: RelocationPhase::Prepared, cancellable: true, error: None, copied: None, installed: None };
    let entries = source_inventory(&plan, false)?;
    plan.file_count = entries.iter().filter(|e| !e.directory).count() as u64;
    plan.directory_count = entries.iter().filter(|e| e.directory).count() as u64;
    plan.total_bytes = entries.iter().map(|e| e.bytes).sum();
    require_disk_space(&target, plan.total_bytes)?;
    validate_plan(&plan, &control)?;
    write_json(&control.join(PLAN_FILE), &plan, false)?;
    // Register the fixed control scope while the operation is still a draft.
    // Status and next boot can find its one plan before any pointer commit.
    // Keep a prior committed marker on relocated sources: its existing pointer
    // already proves the same control scope through subsequent move chains.
    if read_json::<RootDescriptor>(&plan.source.data_dir.join(LOCATION_MARKER))?.is_none() {
        write_json(&plan.source.data_dir.join(LOCATION_MARKER), &descriptor(&plan, &plan.source.data_dir), false)?;
    }
    Ok(plan)
}

pub fn start_storage_relocation(control: &Path, operation_id: &str) -> Result<RelocationPlan, AppError> {
    let (control, _lock) = control_lock(control)?;
    let mut plan = require_operation(&control, operation_id)?;
    if plan.phase == RelocationPhase::AwaitingRestart { return Ok(plan); }
    if plan.phase != RelocationPhase::Prepared && !(plan.phase == RelocationPhase::Failed && plan.cancellable) { return Err(AppError::Conflict("storage relocation is not a prepared or retryable draft".into())); }
    let current = source_location(&plan.source.data_dir, &plan.source.work_dir)?;
    if !same_location(&current, &plan.source) { return Err(AppError::Conflict("storage dataset generation changed after preflight; prepare a new relocation".into())); }
    require_empty_target(&plan.destination.data_dir)?;
    if plan.phase == RelocationPhase::Failed {
        // The source was allowed to resume normal use after failure. Explicit
        // retry takes a fresh offline snapshot rather than pinning old logs,
        // attachments or an application's partially prepared database.
        plan.copied = None;
    }
    plan.phase = RelocationPhase::AwaitingRestart; plan.error = None;
    write_json(&control.join(PLAN_FILE), &plan, true)?;
    Ok(plan)
}

fn require_operation(control: &Path, operation: &str) -> Result<RelocationPlan, AppError> {
    let plan = pending_storage_relocation(control)?.ok_or_else(|| AppError::Conflict("no storage relocation is pending".into()))?;
    if plan.operation_id != operation { return Err(AppError::Conflict("storage relocation operation changed; refresh before acting".into())); }
    Ok(plan)
}

fn archive_terminal(control: &Path, plan: &RelocationPlan) -> Result<(), AppError> {
    let archive = control.join("completed-storage-relocations"); real_directory(&archive, true)?;
    let path = archive.join(format!("{}.json", plan.operation_id));
    if let Some(existing) = read_json::<RelocationPlan>(&path)? {
        if existing != *plan { return Err(io_error("archive operation", &path, "different receipt already exists")); }
    } else { write_json(&path, plan, false)?; }
    write_json(&control.join(LAST_FILE), plan, true)?;
    fs::remove_file(control.join(PLAN_FILE)).map_err(|e| io_error("consume completed plan", control, e))?;
    sync_directory(control).map_err(|e| io_error("flush control directory", control, e))
}

pub fn cancel_storage_relocation(control: &Path, operation_id: &str) -> Result<RelocationPlan, AppError> {
    let (control, _lock) = control_lock(control)?;
    let mut plan = require_operation(&control, operation_id)?;
    if !plan.cancellable || fs::symlink_metadata(plan.source.data_dir.join(SEALED_MARKER)).is_ok() || plan.installed.is_some() {
        return Err(AppError::Conflict("storage relocation publication has started; resume this operation instead of canceling it".into()));
    }
    // Own staging files remain inert; cancel never removes user directories.
    plan.phase = RelocationPhase::Canceled; plan.error = None; plan.cancellable = false;
    archive_terminal(&control, &plan)?;
    Ok(plan)
}

/// The application drops PreparedRelocation's guards before reporting a failed
/// SQLite/domain preparation. Pre-publication failures keep the source usable
/// and require an explicit retry; publication failures resume on the next boot.
pub fn mark_storage_relocation_failed(control: &Path, operation_id: &str, error: &str) -> Result<RelocationPlan, AppError> {
    let (control, _lock) = control_lock(control)?;
    let mut plan = require_operation(&control, operation_id)?;
    plan.phase = RelocationPhase::Failed;
    plan.error = Some(error.chars().take(4096).collect());
    plan.cancellable = plan.installed.is_none() && fs::symlink_metadata(plan.source.data_dir.join(SEALED_MARKER)).is_err();
    write_json(&control.join(PLAN_FILE), &plan, true)?;
    Ok(plan)
}

fn payload_is_control(relative: &Path) -> bool {
    if relative.components().count() != 1 { return false; }
    matches!(relative.to_str(), Some(".nomifun-work-root.lock" | "server.lock" | "server.lock.info" | "nomifun-backend.db.migrate.lock" | ".nomifun-storage-location.json" | ".nomifun-storage-relocated.json" | ".nomifun-storage-staging.json" | "dir-config.json" | "dataset-v3.json" | ".nomifun-work-root-owner.json" | ".nomifun-work-root-binding.json"))
}

// This exact product-owned namespace is derived by the Knowledge owner from
// typed knowledge_bindings. It contains junctions/symlinks to both managed and
// user knowledge roots and must be regenerated, never followed or copied.
fn regenerable_knowledge_mount(relative: &Path) -> bool {
    let components = relative.components().filter_map(|c| match c { Component::Normal(v) => v.to_str(), _ => None }).collect::<Vec<_>>();
    components.len() == 4 && components[0] == "conversations"
        && validate_uuidv7(components[1]).is_ok()
        && components[2..] == [".nomi", "knowledge"]
}

fn source_inventory(plan: &RelocationPlan, digest: bool) -> Result<Vec<CopyEntry>, AppError> {
    let mut entries = Vec::new();
    let nested_work = plan.source.work_dir.strip_prefix(&plan.source.data_dir).ok().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
    scan_tree(&plan.source.data_dir, Path::new(""), CopyBase::Data, digest, &mut entries, &|relative| {
        payload_is_control(relative) || regenerable_knowledge_mount(relative) || nested_work.as_ref().is_some_and(|nested| {
            relative.starts_with(nested.join("conversations")) || relative == nested.join(WORK_ROOT_OWNER_FILE) || relative == nested.join(".nomifun-work-root.lock")
        })
    })?;
    if !paths::paths_equivalent(&plan.source.data_dir, &plan.source.work_dir) {
        if entries.iter().any(|e| e.relative.starts_with("conversations")) {
            return Err(AppError::Conflict("source data root and external work root both contain conversations; refusing to merge two unproven trees".into()));
        }
        let conversations = plan.source.work_dir.join("conversations");
        match fs::symlink_metadata(&conversations) {
            Ok(m) if m.is_dir() && !is_link(&m) => {
                entries.push(CopyEntry { base: CopyBase::Work, relative: PathBuf::from("conversations"), directory: true, bytes: 0, digest: None });
                scan_tree(&plan.source.work_dir, Path::new("conversations"), CopyBase::Work, digest, &mut entries, &regenerable_knowledge_mount)?;
            }
            Ok(_) => return Err(AppError::BadRequest("managed conversations root is a link or is not a directory".into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error("inspect managed work root", &conversations, e)),
        }
    }
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(entries)
}

fn scan_tree(root: &Path, relative: &Path, base: CopyBase, digest: bool, entries: &mut Vec<CopyEntry>, skip: &impl Fn(&Path) -> bool) -> Result<(), AppError> {
    let directory = root.join(relative);
    for entry in fs::read_dir(&directory).map_err(|e| io_error("list payload directory", &directory, e))? {
        let entry = entry.map_err(|e| io_error("read payload entry", &directory, e))?;
        let relative = relative.join(entry.file_name());
        if skip(&relative) { continue; }
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path).map_err(|e| io_error("inspect payload", &path, e))?;
        if is_link(&metadata) || (!metadata.is_file() && !metadata.is_dir()) { return Err(AppError::BadRequest(format!("storage relocation cannot copy symbolic links, reparse points, or special files: {}", path.display()))); }
        entries.push(CopyEntry { base, relative: relative.clone(), directory: metadata.is_dir(), bytes: if metadata.is_file() { metadata.len() } else { 0 }, digest: if metadata.is_file() && digest { Some(file_digest(&path)?) } else { None } });
        if metadata.is_dir() { scan_tree(root, &relative, base, digest, entries, skip)?; }
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String, AppError> {
    let mut options = OpenOptions::new(); options.read(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x00200000); }
    let mut file = options.open(path).map_err(|e| io_error("open payload", path, e))?;
    let metadata = file.metadata().map_err(|e| io_error("inspect opened payload", path, e))?;
    if !metadata.is_file() || is_link(&metadata) { return Err(io_error("hash payload", path, "not a regular file")); }
    let mut hash = Sha256::new(); let mut buffer = [0u8; 64 * 1024];
    loop { let bytes = file.read(&mut buffer).map_err(|e| io_error("read payload", path, e))?; if bytes == 0 { break; } hash.update(&buffer[..bytes]); }
    Ok(format!("{:x}", hash.finalize()))
}

fn descriptor(plan: &RelocationPlan, root: &Path) -> RootDescriptor {
    RootDescriptor { version: VERSION, operation_id: plan.operation_id.clone(), generation: plan.source.generation.clone(), control_dir: plan.control_dir.clone(), data_dir: root.to_path_buf(), destination: plan.destination.data_dir.clone() }
}

fn verify_copy(plan: &RelocationPlan, root: &Path, allow_database_change: bool) -> Result<(), AppError> {
    let expected = plan.copied.as_ref().ok_or_else(|| AppError::Internal("storage relocation has no verified source manifest".into()))?;
    let mut actual = Vec::new(); scan_tree(root, Path::new(""), CopyBase::Data, true, &mut actual, &|relative| payload_is_control(relative))?;
    let normalize = |entries: &[CopyEntry]| { let mut normalized = entries.iter().filter(|e| !(allow_database_change && is_database_family(&e.relative))).cloned().collect::<Vec<_>>(); for entry in &mut normalized { entry.base = CopyBase::Data; } normalized.sort_by(|a,b| a.relative.cmp(&b.relative)); normalized };
    if normalize(expected) != normalize(&actual) { return Err(AppError::Conflict("copied storage payload failed its size/checksum inventory; preserving the original dataset".into())); }
    Ok(())
}

fn is_database_family(path: &Path) -> bool { matches!(path.to_str(), Some("nomifun-backend.db" | "nomifun-backend.db-wal" | "nomifun-backend.db-shm")) }

/// Complete only an already committed operation. This runs before source locks
/// or DB probes: the old source drive may be unavailable after publication.
/// It never copies, mutates the database, or selects an uncommitted target.
pub fn finalize_committed_storage_relocation(control: &Path) -> Result<bool, AppError> {
    let (control, _lock) = control_lock(control)?;
    let Some(mut plan) = pending_storage_relocation(&control)? else { return Ok(false); };
    finalize_committed_locked(&control, &mut plan)
}

fn finalize_committed_locked(control: &Path, plan: &mut RelocationPlan) -> Result<bool, AppError> {
    if let Some(completed) = read_json::<RelocationPlan>(&control.join("completed-storage-relocations").join(format!("{}.json", plan.operation_id)))? {
        validate_plan(&completed, control)?;
        if completed.phase == RelocationPhase::Completed {
            if completed.operation_id != plan.operation_id || completed.source != plan.source
                || completed.destination != plan.destination || completed.control_dir != plan.control_dir
                || completed.staging_data_dir != plan.staging_data_dir {
                return Err(AppError::Conflict("replayed storage operation differs from its committed receipt".into()));
            }
            resolve_storage_location(control)?.ok_or_else(|| AppError::Internal("committed storage receipt has no current location pointer".into()))?;
            // A later relocation/reset may already own the current pointer and
            // last status. Consume only the old pending replay, preserving both.
            fs::remove_file(control.join(PLAN_FILE)).map_err(|e| io_error("consume committed plan replay", control, e))?;
            sync_directory(control).map_err(|e| io_error("flush committed replay consumption", control, e))?;
            return Ok(true);
        }
    }
    let Some(locator) = read_json::<Locator>(&control.join(LOCATOR_FILE))? else { return Ok(false); };
    if locator.operation_id != plan.operation_id || !same_location(&locator.location, &plan.destination)
        || locator.location.origin_data_dir != plan.destination.origin_data_dir { return Ok(false); }
    resolve_storage_location(control)?;
    plan.phase = RelocationPhase::Completed; plan.cancellable = false; plan.error = None;
    archive_terminal(control, plan)?;
    Ok(true)
}

/// Offline only: the application's source data/work locks must already be held
/// and all SQLite pools, browsers, tasks and other dataset writers closed.
pub fn prepare_pending_storage_relocation(control: &Path) -> Result<Option<PreparedRelocation>, AppError> {
    let (control, control_lock) = control_lock(control)?;
    let Some(mut plan) = pending_storage_relocation(&control)? else { return Ok(None); };
    if matches!(plan.phase, RelocationPhase::Prepared | RelocationPhase::Canceled | RelocationPhase::Completed)
        || (plan.phase == RelocationPhase::Failed && plan.cancellable) { return Ok(None); }
    if finalize_committed_locked(&control, &mut plan)? { return Ok(None); }
    let target_parent = real_directory(plan.destination.data_dir.parent().unwrap(), false)?;
    let target_hash = format!("{:x}", Sha256::digest(plan.destination.data_dir.to_string_lossy().as_bytes()));
    let target_lock = lock_file(&target_parent.join(format!(".nomifun-storage-target-{target_hash}.lock")))?;
    let result = prepare_locked(&control, &mut plan);
    if let Err(error) = result {
        plan.phase = RelocationPhase::Failed; plan.error = Some(error.to_string());
        plan.cancellable = plan.installed.is_none() && fs::symlink_metadata(plan.source.data_dir.join(SEALED_MARKER)).is_err();
        write_json(&control.join(PLAN_FILE), &plan, true)?;
        return Err(error);
    }
    Ok(Some(PreparedRelocation { source: plan.source.clone(), destination: plan.destination.clone(), staging_data_dir: if plan.installed.is_some() { plan.destination.data_dir.clone() } else { plan.staging_data_dir.clone() }, preparation_complete: plan.installed.is_some(), plan, _control_lock: control_lock, _target_lock: target_lock }))
}

fn prepare_locked(control: &Path, plan: &mut RelocationPlan) -> Result<(), AppError> {
    // A replay cannot select a previous dataset after a later relocation.
    if read_json::<RelocationPlan>(&control.join("completed-storage-relocations").join(format!("{}.json", plan.operation_id)))?.is_some() {
        return Err(AppError::Conflict("this storage relocation has already completed; refusing to replay it".into()));
    }
    if plan.installed.is_some() {
        if !plan.destination.data_dir.exists() {
            verify_installed_at(plan, &plan.staging_data_dir)?;
            consume_staging_marker(plan)?;
            rename_noreplace(&plan.staging_data_dir, &plan.destination.data_dir).map_err(|e| io_error("resume dataset directory publication", &plan.destination.data_dir, e))?;
            sync_directory(plan.destination.data_dir.parent().unwrap()).map_err(|e| io_error("flush resumed destination", &plan.destination.data_dir, e))?;
        }
        verify_installed(plan)?;
        plan.phase = RelocationPhase::Verifying; plan.cancellable = false;
        write_json(&control.join(PLAN_FILE), plan, true)?;
        return Ok(());
    }
    require_empty_target(&plan.destination.data_dir)?;
    let current = source_location(&plan.source.data_dir, &plan.source.work_dir)?;
    if !same_location(&current, &plan.source) { return Err(AppError::Conflict("storage source generation changed before copying".into())); }
    let inventory = source_inventory(plan, true)?;
    if let Some(previous) = &plan.copied {
        if previous != &inventory { return Err(AppError::Conflict("source data changed after the verified copy; prepare a new relocation".into())); }
        verify_copy(plan, &plan.staging_data_dir, true)?;
        plan.phase = RelocationPhase::Verifying; plan.cancellable = true; plan.error = None;
        write_json(&control.join(PLAN_FILE), plan, true)?;
        return Ok(());
    }
    require_disk_space(&plan.destination.data_dir, inventory.iter().map(|entry| entry.bytes).sum())?;
    plan.phase = RelocationPhase::Copying; plan.error = None;
    write_json(&control.join(PLAN_FILE), plan, true)?;
    if plan.staging_data_dir.exists() {
        let marker: RootDescriptor = read_json(&plan.staging_data_dir.join(STAGING_MARKER))?.ok_or_else(|| AppError::Conflict("staging directory already exists without this operation's ownership proof".into()))?;
        validate_descriptor(&marker, &plan.staging_data_dir, control)?;
        if marker.operation_id != plan.operation_id || marker.generation != plan.source.generation { return Err(AppError::Conflict("staging directory belongs to another storage operation".into())); }
        // An interrupted partial copy is strictly private to this immutable
        // operation. Validate every leaf before removing that staging tree.
        let canonical_staging = real_directory(&plan.staging_data_dir, false)?;
        if canonical_staging != plan.staging_data_dir { return Err(AppError::Conflict("staging directory identity changed before cleanup".into())); }
        let mut ignored = Vec::new(); scan_tree(&canonical_staging, Path::new(""), CopyBase::Data, false, &mut ignored, &|_| false)?;
        #[cfg(windows)]
        for entry in ignored.iter().filter(|entry| !entry.directory) {
            let path = canonical_staging.join(&entry.relative);
            let mut permissions = fs::symlink_metadata(&path).map_err(|e| io_error("inspect owned staging permissions", &path, e))?.permissions();
            if permissions.readonly() {
                permissions.set_readonly(false);
                fs::set_permissions(&path, permissions).map_err(|e| io_error("clear owned staging read-only flag", &path, e))?;
            }
        }
        fs::remove_dir_all(&plan.staging_data_dir).map_err(|e| io_error("discard incomplete owned staging", &plan.staging_data_dir, e))?;
    }
    fs::create_dir(&plan.staging_data_dir).map_err(|e| io_error("create staging directory", &plan.staging_data_dir, e))?;
    write_json(&plan.staging_data_dir.join(STAGING_MARKER), &descriptor(plan, &plan.staging_data_dir), false)?;
    for entry in &inventory {
        let destination = plan.staging_data_dir.join(&entry.relative);
        if entry.directory { fs::create_dir_all(&destination).map_err(|e| io_error("create copied directory", &destination, e))?; continue; }
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| io_error("create copied parent", &destination, e))?;
        let source = match entry.base { CopyBase::Data => &plan.source.data_dir, CopyBase::Work => &plan.source.work_dir }.join(&entry.relative);
        let copied = copy_regular_payload(&source, &destination)?;
        if copied != entry.bytes || Some(file_digest(&destination)?) != entry.digest { return Err(io_error("verify copied file", &destination, "size or checksum mismatch")); }
    }
    if source_inventory(plan, true)? != inventory { return Err(AppError::Conflict("a dataset writer changed the source during relocation; preserving it without switching".into())); }
    plan.copied = Some(inventory);
    verify_copy(plan, &plan.staging_data_dir, false)?;
    sync_tree(&plan.staging_data_dir)?;
    plan.phase = RelocationPhase::Verifying;
    write_json(&control.join(PLAN_FILE), plan, true)?;
    Ok(())
}

fn copy_regular_payload(source: &Path, destination: &Path) -> Result<u64, AppError> {
    let mut options = OpenOptions::new(); options.read(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK); }
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x00200000); }
    let mut input = options.open(source).map_err(|e| io_error("open source payload", source, e))?;
    let metadata = input.metadata().map_err(|e| io_error("inspect source payload", source, e))?;
    if !metadata.is_file() || is_link(&metadata) { return Err(io_error("copy source payload", source, "not a regular file")); }
    let mut output = OpenOptions::new().create_new(true).write(true).open(destination).map_err(|e| io_error("create copied payload", destination, e))?;
    let bytes = std::io::copy(&mut input, &mut output).map_err(|e| io_error("copy payload bytes", destination, e))?;
    output.sync_all().map_err(|e| io_error("flush copied payload", destination, e))?;
    fs::set_permissions(destination, metadata.permissions()).map_err(|e| io_error("preserve copied permissions", destination, e))?;
    Ok(bytes)
}

fn sync_tree(root: &Path) -> Result<(), AppError> {
    for entry in fs::read_dir(root).map_err(|e| io_error("list directory for flush", root, e))? {
        let path = entry.map_err(|e| io_error("read directory for flush", root, e))?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| io_error("inspect copied entry", &path, e))?;
        if is_link(&metadata) { return Err(io_error("flush copied entry", &path, "unexpected link")); }
        if metadata.is_dir() { sync_tree(&path)?; }
    }
    sync_directory(root).map_err(|e| io_error("flush copied directory", root, e))
}

fn verify_installed(plan: &RelocationPlan) -> Result<(), AppError> {
    verify_installed_at(plan, &plan.destination.data_dir)?;
    factory_reset::require_current_v3_dataset_for_work_dir(&plan.destination.data_dir, &plan.destination.work_dir)
}

fn verify_installed_at(plan: &RelocationPlan, physical_root: &Path) -> Result<(), AppError> {
    let expected = plan.installed.as_ref().ok_or_else(|| AppError::Internal("no installed storage inventory".into()))?;
    let root = real_directory(physical_root, false)?;
    let marker: RootDescriptor = read_json(&root.join(LOCATION_MARKER))?.ok_or_else(|| AppError::Conflict("published storage target is missing its operation marker".into()))?;
    validate_descriptor(&marker, &plan.destination.data_dir, &plan.control_dir)?;
    if marker.operation_id != plan.operation_id || marker.generation != plan.source.generation { return Err(AppError::Conflict("published storage target belongs to another operation".into())); }
    let mut actual = Vec::new(); scan_tree(&root, Path::new(""), CopyBase::Data, true, &mut actual, &|p| p.components().count()==1 && matches!(p.to_str(), Some(".nomifun-work-root.lock" | ".nomifun-storage-staging.json")))?;
    actual.sort_by(|a,b| a.relative.cmp(&b.relative));
    if actual != *expected { return Err(AppError::Conflict("published storage checksum inventory changed before location commit".into())); }
    Ok(())
}

fn consume_staging_marker(plan: &RelocationPlan) -> Result<(), AppError> {
    let path = plan.staging_data_dir.join(STAGING_MARKER);
    if let Some(marker) = read_json::<RootDescriptor>(&path)? {
        if marker != descriptor(plan, &plan.staging_data_dir) { return Err(AppError::Conflict("staging proof changed before publication".into())); }
        fs::remove_file(&path).map_err(|e| io_error("consume staging ownership proof", &path, e))?;
    }
    Ok(())
}

/// The application has proved and, if necessary, explicitly transitioned the
/// current canonical bindings in the staged DB. Immutable history is untouched.
pub fn commit_storage_relocation(control: &Path, prepared: &PreparedRelocation) -> Result<StorageLocation, AppError> {
    let control = real_directory(control, false)?;
    if !paths::paths_equivalent(&control, &prepared.plan.control_dir) { return Err(AppError::Conflict("prepared storage operation uses another control scope".into())); }
    let mut plan = require_operation(&control, &prepared.plan.operation_id)?;
    if plan.source != prepared.source || plan.destination != prepared.destination || plan.copied != prepared.plan.copied {
        return Err(AppError::Conflict("prepared storage operation no longer matches its durable plan".into()));
    }
    if let Some(locator) = read_json::<Locator>(&control.join(LOCATOR_FILE))? {
        if locator.operation_id == plan.operation_id {
            if !same_location(&locator.location, &plan.destination) { return Err(AppError::Conflict("committed storage pointer identity differs from the pending operation".into())); }
            // The pointer is the commit. Do not reject already committed data
            // merely because a later service has legitimately written to it.
            plan.phase = RelocationPhase::Completed; plan.cancellable = false; plan.error = None;
            archive_terminal(&control, &plan)?;
            return Ok(plan.destination);
        }
        if !same_location(&locator.location, &plan.source) { return Err(AppError::Conflict("fixed storage pointer changed before relocation commit".into())); }
    }
    if plan.installed.is_none() {
        if source_inventory(&plan, true)? != *plan.copied.as_ref().ok_or_else(|| AppError::Internal("relocation was not copied".into()))? { return Err(AppError::Conflict("source dataset changed while its copied database was being prepared".into())); }
        verify_copy(&plan, &plan.staging_data_dir, true)?;
        require_empty_target(&plan.destination.data_dir)?;
        if plan.destination.data_dir.exists() {
            fs::remove_dir(&plan.destination.data_dir).map_err(|e| io_error("reserve empty target", &plan.destination.data_dir, e))?;
        }
        factory_reset::write_v3_single_root_lifecycle_for_atomic_install(&plan.staging_data_dir, &plan.destination.data_dir, &plan.source.generation)?;
        crate::dir_config::set_work_dir(&plan.staging_data_dir, &plan.destination.work_dir)?;
        write_json(&plan.staging_data_dir.join(LOCATION_MARKER), &descriptor(&plan, &plan.destination.data_dir), true)?;
        sync_tree(&plan.staging_data_dir)?;
        let mut installed = Vec::new(); scan_tree(&plan.staging_data_dir, Path::new(""), CopyBase::Data, true, &mut installed, &|p| p == Path::new(STAGING_MARKER))?;
        installed.sort_by(|a,b| a.relative.cmp(&b.relative));
        plan.installed = Some(installed); plan.cancellable = false;
        // Publish intent and final checksums BEFORE rename. Recovery can prove
        // either the staging tree or the published destination, never guess.
        write_json(&control.join(PLAN_FILE), &plan, true)?;
        consume_staging_marker(&plan)?;
        rename_noreplace(&plan.staging_data_dir, &plan.destination.data_dir).map_err(|e| io_error("publish dataset directory", &plan.destination.data_dir, e))?;
        sync_directory(plan.destination.data_dir.parent().unwrap()).map_err(|e| io_error("flush published parent", &plan.destination.data_dir, e))?;
    }
    verify_installed(&plan)?;
    let seal = descriptor(&plan, &plan.source.data_dir);
    if let Some(existing) = read_json::<RootDescriptor>(&plan.source.data_dir.join(SEALED_MARKER))? {
        if existing != seal { return Err(AppError::Conflict("source storage was sealed by a different operation".into())); }
    } else { write_json(&plan.source.data_dir.join(SEALED_MARKER), &seal, false)?; }
    // This atomic pointer is the sole commit. The durable plan remains until
    // its terminal receipt is archived, covering seal/pointer/archive crashes.
    write_json(&control.join(LOCATOR_FILE), &Locator { version: VERSION, operation_id: plan.operation_id.clone(), location: plan.destination.clone() }, true)?;
    plan.phase = RelocationPhase::Completed; plan.cancellable = false; plan.error = None;
    archive_terminal(&control, &plan)?;
    Ok(plan.destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _directory: tempfile::TempDir,
        source: PathBuf,
        work: PathBuf,
        control: PathBuf,
        target: PathBuf,
        generation: String,
    }

    impl Fixture {
        fn new(external_work: bool) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let parent = paths::canonicalize_simplified(directory.path()).unwrap();
            let source = parent.join("source"); fs::create_dir(&source).unwrap();
            let work = if external_work { let work = parent.join("external-work"); fs::create_dir(&work).unwrap(); work } else { source.clone() };
            let generation = Uuid::now_v7().to_string();
            fs::write(source.join("storage-generation"), &generation).unwrap();
            fs::write(source.join("nomifun-backend.db"), b"canonical Session Turn Event Payload Effect").unwrap();
            for (name, bytes) in [("attachments/facts.bin", "attachment"), ("knowledge/base/README.md", "knowledge"), ("plugin-core/artifacts/asset.bin", "artifact"), ("plugin-data/local.json", "plugin state"), ("plugin-drafts/projects/draft/a.txt", "draft"), ("encryption_key", "encryption key")] {
                let file = source.join(name); fs::create_dir_all(file.parent().unwrap()).unwrap(); fs::write(file, bytes).unwrap();
            }
            fs::create_dir_all(work.join("conversations/session/project")).unwrap();
            fs::write(work.join("conversations/session/project/result.txt"), b"conversation result").unwrap();
            factory_reset::write_v3_dataset_receipt_for_work_dir(&source, &work, &generation).unwrap();
            Self { _directory: directory, source, work, control: parent.join("control"), target: parent.join("new-storage"), generation }
        }

        fn draft(&self) -> RelocationPlan {
            request_storage_relocation(&self.control, &self.source, &self.work, &self.target).unwrap()
        }

        fn stage(&self) -> PreparedRelocation {
            let plan = self.draft(); start_storage_relocation(&self.control, &plan.operation_id).unwrap();
            prepare_pending_storage_relocation(&self.control).unwrap().unwrap()
        }
    }

    #[test]
    fn draft_registers_fixed_scope_but_never_copies_or_arms_on_boot() {
        let f = Fixture::new(false); let plan = f.draft();
        assert_eq!(plan.phase, RelocationPhase::Prepared);
        assert!(plan.file_count >= 8 && plan.total_bytes > 0);
        assert_eq!(resolve_control_dir(&f.source).unwrap(), Some(f.control.clone()));
        assert!(prepare_pending_storage_relocation(&f.control).unwrap().is_none());
        assert!(!plan.staging_data_dir.exists()); assert!(!f.target.exists());
        assert!(resolve_storage_location(&f.control).unwrap().is_none());
        require_storage_root_not_sealed(&f.source).unwrap();
    }

    #[test]
    fn first_draft_durable_before_descriptor_is_recovered_only_for_exact_source() {
        let f = Fixture::new(false); let plan = f.draft();
        fs::remove_file(f.source.join(LOCATION_MARKER)).unwrap();
        assert_eq!(resolve_initial_scope(&f.source, &f.control).unwrap(), Some(f.control.clone()));
        assert!(resolve_initial_scope(&f.target, &f.control).unwrap().is_none());
        assert_eq!(pending_storage_relocation(&f.control).unwrap().unwrap(), plan);
        assert!(!f.target.exists()); assert!(!plan.staging_data_dir.exists());
    }

    #[test]
    fn relocation_preserves_dataset_facts_identity_all_side_stores_and_source() {
        let f = Fixture::new(false); let staged = f.stage();
        assert!(!staged.preparation_complete);
        assert_eq!(fs::read(staged.staging_data_dir.join("nomifun-backend.db")).unwrap(), fs::read(f.source.join("nomifun-backend.db")).unwrap());
        let destination = commit_storage_relocation(&f.control, &staged).unwrap();
        assert_eq!(destination.generation, f.generation);
        assert_eq!(destination.origin_data_dir, f.source);
        assert_eq!(destination.data_dir, f.target); assert_eq!(destination.work_dir, f.target);
        for relative in ["nomifun-backend.db", "attachments/facts.bin", "knowledge/base/README.md", "plugin-core/artifacts/asset.bin", "plugin-data/local.json", "plugin-drafts/projects/draft/a.txt", "encryption_key", "conversations/session/project/result.txt"] {
            assert_eq!(fs::read(f.source.join(relative)).unwrap(), fs::read(f.target.join(relative)).unwrap(), "{relative}");
        }
        assert!(matches!(require_storage_root_not_sealed(&f.source), Err(AppError::Conflict(_))));
        assert_eq!(resolve_storage_location(&f.control).unwrap(), Some(destination));
        assert_eq!(resolve_control_dir(&f.target).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_control_dir(&f.source).unwrap(), Some(f.control.clone()));
        assert_eq!(storage_relocation_status(&f.control).unwrap().unwrap().phase, RelocationPhase::Completed);
        factory_reset::require_current_v3_dataset_for_work_dir(&f.target, &f.target).unwrap();
    }

    #[test]
    fn only_managed_conversations_are_copied_from_external_work_root() {
        let f = Fixture::new(true);
        fs::write(f.work.join("my-user-project.txt"), b"not application data").unwrap();
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap();
        assert!(f.target.join("conversations/session/project/result.txt").is_file());
        assert!(!f.target.join("my-user-project.txt").exists());
        assert_eq!(fs::read(f.work.join("my-user-project.txt")).unwrap(), b"not application data");
        assert!(f.work.join("conversations/session/project/result.txt").is_file());
    }

    #[test]
    fn only_exact_owned_knowledge_namespace_is_regenerated_and_nested_user_files_are_kept() {
        let f = Fixture::new(false); let session = Uuid::now_v7().to_string();
        let owned = PathBuf::from("conversations").join(&session).join(".nomi/knowledge");
        let nested = PathBuf::from("conversations").join(&session).join("project/.nomi/knowledge");
        let skills = PathBuf::from("conversations").join(&session).join(".nomi/skills");
        for (relative, bytes) in [(&owned, "derived knowledge mount"), (&nested, "nested user data"), (&skills, "skill source")] {
            fs::create_dir_all(f.source.join(relative)).unwrap();
            fs::write(f.source.join(relative).join("README.md"), bytes).unwrap();
        }
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap();
        assert!(!f.target.join(owned).exists());
        assert_eq!(fs::read(f.target.join(nested).join("README.md")).unwrap(), b"nested user data");
        assert_eq!(fs::read(f.target.join(skills).join("README.md")).unwrap(), b"skill source");
    }

    #[test]
    fn database_preparation_may_change_only_database_family() {
        let f = Fixture::new(false); let staged = f.stage();
        fs::write(staged.staging_data_dir.join("nomifun-backend.db"), b"canonical binding transition, unchanged historical facts").unwrap();
        commit_storage_relocation(&f.control, &staged).unwrap();
        assert_eq!(fs::read(f.source.join("nomifun-backend.db")).unwrap(), b"canonical Session Turn Event Payload Effect");
        assert_eq!(fs::read(f.target.join("nomifun-backend.db")).unwrap(), b"canonical binding transition, unchanged historical facts");
    }

    #[test]
    fn corrupted_payload_or_late_source_write_never_commits_pointer() {
        let f = Fixture::new(false); let staged = f.stage();
        fs::write(staged.staging_data_dir.join("attachments/facts.bin"), b"corrupt").unwrap();
        assert!(commit_storage_relocation(&f.control, &staged).is_err());
        assert!(resolve_storage_location(&f.control).unwrap().is_none());
        assert!(!f.target.exists()); require_storage_root_not_sealed(&f.source).unwrap();
        assert_eq!(fs::read(f.source.join("attachments/facts.bin")).unwrap(), b"attachment");
        let f = Fixture::new(false); let staged = f.stage();
        fs::write(f.source.join("attachments/facts.bin"), b"late writer").unwrap();
        assert!(commit_storage_relocation(&f.control, &staged).is_err());
        assert!(resolve_storage_location(&f.control).unwrap().is_none()); require_storage_root_not_sealed(&f.source).unwrap();
    }

    #[test]
    fn draft_cancel_and_failed_preparation_allow_source_use_and_explicit_retry() {
        let f = Fixture::new(false); let plan = f.draft();
        cancel_storage_relocation(&f.control, &plan.operation_id).unwrap();
        assert!(pending_storage_relocation(&f.control).unwrap().is_none());
        assert_eq!(resolve_control_dir(&f.source).unwrap(), Some(f.control.clone()));
        require_storage_root_not_sealed(&f.source).unwrap();
        let staged = f.stage(); let operation = staged.plan.operation_id.clone(); drop(staged);
        let failed = mark_storage_relocation_failed(&f.control, &operation, "canonical DB validation failed").unwrap();
        assert!(failed.cancellable); assert_eq!(failed.phase, RelocationPhase::Failed);
        assert!(prepare_pending_storage_relocation(&f.control).unwrap().is_none());
        require_storage_root_not_sealed(&f.source).unwrap();
        // Normal source use after failure must be included in a fresh retry.
        fs::write(f.source.join("attachments/facts.bin"), b"attachment after failure").unwrap();
        fs::write(f.source.join("new-normal-use.log"), b"normal source writes resumed").unwrap();
        let old_stage = pending_storage_relocation(&f.control).unwrap().unwrap().staging_data_dir;
        fs::write(old_stage.join("nomifun-backend.db"), b"failed callback partial DB").unwrap();
        start_storage_relocation(&f.control, &operation).unwrap();
        let staged = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        commit_storage_relocation(&f.control, &staged).unwrap();
        assert_eq!(fs::read(f.target.join("attachments/facts.bin")).unwrap(), b"attachment after failure");
        assert_eq!(fs::read(f.target.join("new-normal-use.log")).unwrap(), b"normal source writes resumed");
        assert_eq!(fs::read(f.target.join("nomifun-backend.db")).unwrap(), b"canonical Session Turn Event Payload Effect");
    }

    #[test]
    fn generation_change_and_foreign_nonempty_or_overlapping_targets_fail_closed() {
        let f = Fixture::new(false);
        let overlap = f.source.join("nested/deep/target");
        assert!(request_storage_relocation(&f.control, &f.source, &f.work, &overlap).is_err());
        assert!(!f.source.join("nested").exists()); assert!(!f.control.exists());
        let foreign = f.target.clone(); fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join(WORK_ROOT_OWNER_FILE), b"another dataset").unwrap();
        assert!(matches!(request_storage_relocation(&f.control, &f.source, &f.work, &foreign), Err(AppError::Conflict(_))));
        assert_eq!(fs::read(foreign.join(WORK_ROOT_OWNER_FILE)).unwrap(), b"another dataset");
        let f = Fixture::new(false); let plan = f.draft();
        let different = Uuid::now_v7().to_string(); fs::write(f.source.join("storage-generation"), &different).unwrap();
        assert!(start_storage_relocation(&f.control, &plan.operation_id).is_err());
        assert!(!plan.staging_data_dir.exists()); assert!(!f.target.exists());
    }

    #[test]
    fn control_and_target_guards_cover_database_callback_and_skip_live_server_lock() {
        let f = Fixture::new(false);
        let server_lock = lock_file(&f.source.join("server.lock")).unwrap();
        let staged = f.stage();
        assert!(control_lock(&f.control).is_err());
        let parent = f.target.parent().unwrap(); let digest = format!("{:x}", Sha256::digest(f.target.to_string_lossy().as_bytes()));
        assert!(lock_file(&parent.join(format!(".nomifun-storage-target-{digest}.lock"))).is_err());
        assert!(!staged.staging_data_dir.join("server.lock").exists());
        commit_storage_relocation(&f.control, &staged).unwrap();
        drop(server_lock);
    }

    fn install_intent_without_pointer(f: &Fixture, staged: &PreparedRelocation, publish: bool, seal: bool) {
        let mut plan = staged.plan.clone();
        factory_reset::write_v3_single_root_lifecycle_for_atomic_install(&plan.staging_data_dir, &plan.destination.data_dir, &plan.source.generation).unwrap();
        crate::dir_config::set_work_dir(&plan.staging_data_dir, &plan.destination.work_dir).unwrap();
        write_json(&plan.staging_data_dir.join(LOCATION_MARKER), &descriptor(&plan, &plan.destination.data_dir), true).unwrap();
        let mut manifest = Vec::new(); scan_tree(&plan.staging_data_dir, Path::new(""), CopyBase::Data, true, &mut manifest, &|p| p==Path::new(STAGING_MARKER)).unwrap();
        manifest.sort_by(|a,b| a.relative.cmp(&b.relative));
        plan.installed = Some(manifest); plan.cancellable = false;
        write_json(&f.control.join(PLAN_FILE), &plan, true).unwrap();
        if publish { consume_staging_marker(&plan).unwrap(); rename_noreplace(&plan.staging_data_dir, &plan.destination.data_dir).unwrap(); }
        if seal { write_json(&f.source.join(SEALED_MARKER), &descriptor(&plan, &f.source), false).unwrap(); }
    }

    #[test]
    fn crash_after_final_inventory_before_rename_resumes_exact_install() {
        let f = Fixture::new(false); let staged = f.stage();
        install_intent_without_pointer(&f, &staged, false, false); drop(staged);
        let recovered = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        assert!(recovered.preparation_complete); assert_eq!(recovered.staging_data_dir, f.target);
        commit_storage_relocation(&f.control, &recovered).unwrap();
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().generation, f.generation);
    }

    #[test]
    fn sealed_source_before_pointer_is_discoverable_and_recoverable() {
        let f = Fixture::new(false); let staged = f.stage();
        install_intent_without_pointer(&f, &staged, true, true); drop(staged);
        assert_eq!(resolve_control_dir(&f.source).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_control_dir(&f.target).unwrap(), Some(f.control.clone()));
        let recovered = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        assert!(recovered.preparation_complete);
        commit_storage_relocation(&f.control, &recovered).unwrap();
        assert!(pending_storage_relocation(&f.control).unwrap().is_none());
    }

    #[test]
    fn pointer_commit_before_archive_never_repeats_database_preparation() {
        let f = Fixture::new(false); let staged = f.stage();
        install_intent_without_pointer(&f, &staged, true, true);
        write_json(&f.control.join(LOCATOR_FILE), &Locator { version: VERSION, operation_id: staged.plan.operation_id.clone(), location: staged.destination.clone() }, true).unwrap();
        drop(staged);
        fs::write(f.target.join("nomifun-backend.db"), b"legitimate new writes after pointer commit").unwrap();
        assert!(prepare_pending_storage_relocation(&f.control).unwrap().is_none());
        assert!(pending_storage_relocation(&f.control).unwrap().is_none());
        assert_eq!(fs::read(f.target.join("nomifun-backend.db")).unwrap(), b"legitimate new writes after pointer commit");
    }

    #[test]
    fn committed_finalization_does_not_require_old_source_or_work_drive() {
        let f = Fixture::new(true); let draft = f.draft();
        assert!(!finalize_committed_storage_relocation(&f.control).unwrap());
        assert_eq!(pending_storage_relocation(&f.control).unwrap().unwrap(), draft);
        let staged = f.stage(); let pending = staged.plan.clone();
        commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        fs::remove_file(f.control.join("completed-storage-relocations").join(format!("{}.json", pending.operation_id))).unwrap();
        write_json(&f.control.join(PLAN_FILE), &pending, false).unwrap();
        fs::rename(&f.source, f.source.with_file_name("source-offline")).unwrap();
        fs::rename(&f.work, f.work.with_file_name("work-offline")).unwrap();
        assert!(finalize_committed_storage_relocation(&f.control).unwrap());
        assert!(!f.source.exists()); assert!(!f.work.exists());
        assert!(pending_storage_relocation(&f.control).unwrap().is_none());
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().data_dir, f.target);
    }

    #[test]
    fn repeated_moves_keep_original_profile_identity_and_control_scope() {
        let f = Fixture::new(false); let first = f.stage();
        commit_storage_relocation(&f.control, &first).unwrap(); drop(first);
        let final_root = f.target.parent().unwrap().join("third-storage");
        let plan = request_storage_relocation(&f.control, &f.target, &f.target, &final_root).unwrap();
        assert_eq!(plan.source.origin_data_dir, f.source); assert_eq!(plan.destination.origin_data_dir, f.source);
        start_storage_relocation(&f.control, &plan.operation_id).unwrap();
        let staged = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        commit_storage_relocation(&f.control, &staged).unwrap();
        assert_eq!(resolve_control_dir(&f.source).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_control_dir(&f.target).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_control_dir(&final_root).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().origin_data_dir, f.source);
    }

    #[test]
    fn completed_old_plan_replay_never_reselects_old_dataset_or_blocks_current_location() {
        let f = Fixture::new(false); let first = f.stage();
        let old_pending = first.plan.clone();
        commit_storage_relocation(&f.control, &first).unwrap(); drop(first);
        let newest = f.target.with_file_name("newest-storage");
        let second = request_storage_relocation(&f.control, &f.target, &f.target, &newest).unwrap();
        start_storage_relocation(&f.control, &second.operation_id).unwrap();
        let staged = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        let latest_status = storage_relocation_status(&f.control).unwrap().unwrap();
        write_json(&f.control.join(PLAN_FILE), &old_pending, false).unwrap();
        assert!(finalize_committed_storage_relocation(&f.control).unwrap());
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().data_dir, newest);
        assert_eq!(storage_relocation_status(&f.control).unwrap().unwrap(), latest_status);
        assert!(pending_storage_relocation(&f.control).unwrap().is_none());
    }

    #[test]
    fn real_factory_reset_rotates_generation_at_the_same_fixed_location() {
        let f = Fixture::new(false); let first = f.stage(); commit_storage_relocation(&f.control, &first).unwrap(); drop(first);
        factory_reset::request_v3_dataset_reset(&f.target, &f.target).unwrap();
        assert!(resolve_storage_location(&f.control).unwrap().is_some(), "pending reset must reach its own authoritative startup coordinator");
        factory_reset::prepare_v3_dataset(&f.target, &f.target).unwrap();
        let reset = factory_reset::read_pending_v3_reset(&f.target, &f.target).unwrap().unwrap();
        fs::write(f.target.join("nomifun-backend.db"), b"fresh canonical dataset").unwrap();
        factory_reset::write_v3_dataset_receipt_for_work_dir(&f.target, &f.target, &reset.generation).unwrap();
        factory_reset::finalize_v3_dataset_reset(&f.target, &f.target).unwrap();
        refresh_current_storage_location(&f.control, &f.target).unwrap();
        let active = resolve_storage_location(&f.control).unwrap().unwrap();
        assert_eq!(active.generation, reset.generation); assert_ne!(active.generation, f.generation);
        assert_eq!(active.origin_data_dir, f.source);
        assert_eq!(resolve_control_dir(&f.target).unwrap(), Some(f.control.clone()));
    }
    fn fixture_initial_control(f: &mut Fixture) -> PathBuf {
        let configuration = paths::canonicalize_simplified(f._directory.path()).unwrap().join("configuration");
        f.control = initial_control_dir_in_configuration(&f.source, &configuration).unwrap();
        configuration
    }

    fn resolve_fixture_initial_control(root: &Path, configuration: &Path) -> Result<Option<PathBuf>, AppError> {
        resolve_control_dir_with_initial_scope(root, |initial| initial_control_dir_in_configuration(initial, configuration))
    }

    fn remove_fixture_root(f: &Fixture, root: &Path) {
        let fixture_parent = paths::canonicalize_simplified(f._directory.path()).unwrap();
        assert_eq!(root.parent(), Some(fixture_parent.as_path()), "only delete an explicitly owned temporary fixture root");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_original_root_follows_only_committed_fixed_location_across_repeated_moves() {
        let mut f = Fixture::new(false); let configuration = fixture_initial_control(&mut f);
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        remove_fixture_root(&f, &f.source);
        assert_eq!(resolve_fixture_initial_control(&f.source, &configuration).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().data_dir, f.target);
        assert!(!f.source.exists(), "lookup must not recreate the removed source");
        let newest = f.target.with_file_name("newest-storage");
        let second = request_storage_relocation(&f.control, &f.target, &f.target, &newest).unwrap();
        start_storage_relocation(&f.control, &second.operation_id).unwrap();
        let staged = prepare_pending_storage_relocation(&f.control).unwrap().unwrap();
        commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        remove_fixture_root(&f, &f.target);
        assert_eq!(resolve_fixture_initial_control(&f.source, &configuration).unwrap(), Some(f.control.clone()));
        let current = resolve_storage_location(&f.control).unwrap().unwrap();
        assert_eq!(current.data_dir, newest); assert_eq!(current.origin_data_dir, f.source);
        assert!(!f.source.exists()); assert!(!f.target.exists());
        assert_eq!(fs::read(current.data_dir.join("nomifun-backend.db")).unwrap(), b"canonical Session Turn Event Payload Effect");
    }

    #[test]
    fn missing_original_root_rejects_wrong_origin_or_target_descriptor() {
        let mut f = Fixture::new(false); let configuration = fixture_initial_control(&mut f);
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        remove_fixture_root(&f, &f.source);
        let mut pointer: Locator = read_json(&f.control.join(LOCATOR_FILE)).unwrap().unwrap();
        pointer.location.origin_data_dir = f.target.with_file_name("another-original-root");
        write_json(&f.control.join(LOCATOR_FILE), &pointer, true).unwrap();
        assert!(matches!(resolve_fixture_initial_control(&f.source, &configuration), Err(AppError::Conflict(_))));
        assert!(!f.source.exists());
        pointer.location.origin_data_dir = f.source.clone();
        write_json(&f.control.join(LOCATOR_FILE), &pointer, true).unwrap();
        let mut marker: RootDescriptor = read_json(&f.target.join(LOCATION_MARKER)).unwrap().unwrap();
        marker.operation_id = Uuid::now_v7().to_string();
        write_json(&f.target.join(LOCATION_MARKER), &marker, true).unwrap();
        assert!(resolve_fixture_initial_control(&f.source, &configuration).is_err());
        assert!(!f.source.exists());
    }

    #[test]
    fn reused_original_address_without_descriptor_never_adopts_or_redirects_foreign_dataset() {
        let mut f = Fixture::new(false); let configuration = fixture_initial_control(&mut f);
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        remove_fixture_root(&f, &f.source); fs::create_dir(&f.source).unwrap();
        let foreign_generation = Uuid::now_v7().to_string();
        fs::write(f.source.join("storage-generation"), &foreign_generation).unwrap();
        fs::write(f.source.join("nomifun-backend.db"), b"separate newly installed dataset").unwrap();
        factory_reset::write_v3_dataset_receipt_for_work_dir(&f.source, &f.source, &foreign_generation).unwrap();
        let pointer_before = fs::read(f.control.join(LOCATOR_FILE)).unwrap();
        assert!(matches!(resolve_fixture_initial_control(&f.source, &configuration), Err(AppError::Conflict(_))));
        assert!(!f.source.join(LOCATION_MARKER).exists()); assert!(!f.source.join(SEALED_MARKER).exists());
        assert_eq!(fs::read(f.source.join("nomifun-backend.db")).unwrap(), b"separate newly installed dataset");
        assert_eq!(fs::read(f.control.join(LOCATOR_FILE)).unwrap(), pointer_before);
    }

    #[test]
    fn unavailable_original_with_uncommitted_operation_never_obtains_empty_root_authority() {
        let mut f = Fixture::new(false); let configuration = fixture_initial_control(&mut f);
        let plan = f.draft(); remove_fixture_root(&f, &f.source);
        assert!(matches!(resolve_fixture_initial_control(&f.source, &configuration), Err(AppError::Conflict(_))));
        assert_eq!(pending_storage_relocation(&f.control).unwrap().unwrap(), plan);
        assert!(!f.source.exists()); assert!(!f.target.exists()); assert!(!plan.staging_data_dir.exists());
        let untouched = f.target.with_file_name("never-registered-source");
        assert!(resolve_fixture_initial_control(&untouched, &configuration).unwrap().is_none());
        assert!(!untouched.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_initial_scope_and_missing_origin_accept_case_and_separator_aliases() {
        let mut f = Fixture::new(false); let configuration = fixture_initial_control(&mut f);
        let alias = PathBuf::from(f.source.to_string_lossy().to_uppercase().replace('\\', "/"));
        assert_eq!(initial_control_dir_in_configuration(&alias, &configuration).unwrap(), f.control);
        let staged = f.stage(); commit_storage_relocation(&f.control, &staged).unwrap(); drop(staged);
        remove_fixture_root(&f, &f.source);
        assert_eq!(initial_control_dir_in_configuration(&alias, &configuration).unwrap(), f.control);
        assert_eq!(resolve_fixture_initial_control(&alias, &configuration).unwrap(), Some(f.control.clone()));
        assert_eq!(resolve_storage_location(&f.control).unwrap().unwrap().data_dir, f.target);
        assert!(!f.source.exists());
    }

}
