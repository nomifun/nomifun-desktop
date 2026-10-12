//! Pure offline registry inspection and relocation. Dataset coordination owns
//! the source/staging locks and publication; this module never starts an owner,
//! reconciles processes, converts history, or writes the source registry.
use std::{collections::{BTreeMap, BTreeSet}, fs, io::Write, path::{Component, Path, PathBuf}};
use serde_json::Value;
use crate::{AppError, AppMetadata, AppState};
use crate::files::ensure_no_links;
use crate::owner::{StoredRecord, validate_id, validate_metadata, validate_name, validate_settings};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OfflineHostedAppInventory {
    /// Only exact registered application identities, never unregistered files
    /// under the data root or an arbitrary Session's selected workspace.
    pub app_ids: BTreeSet<String>,
    pub owned_directories: BTreeSet<PathBuf>,
}

struct RegistrySnapshot {
    inventory: OfflineHostedAppInventory,
    records: BTreeMap<String, (Vec<u8>, Value)>,
}

fn invalid(message: &str) -> AppError { AppError::Invalid(message.to_owned()) }

fn absolute_plain_path(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return Err(invalid("offline application storage roots must be exact absolute paths"));
    }
    // Check all existing ancestors, including the root itself. Missing final
    // destination components are allowed before dataset publication.
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        ensure_no_links(&current, &current)?;
        match fs::symlink_metadata(&current) {
            Ok(metadata) if !metadata.is_dir() => return Err(invalid("offline application storage root ancestor is not a directory")),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn existing_plain_root(path: &Path) -> Result<(), AppError> {
    absolute_plain_path(path)?;
    if !fs::metadata(path)?.is_dir() { return Err(invalid("offline application data root is not a directory")); }
    if dunce::canonicalize(path)? != path { return Err(invalid("offline application data root is not canonical")); }
    Ok(())
}

fn require_plain_directory(root: &Path, path: &Path) -> Result<(), AppError> {
    ensure_no_links(root, path)?;
    if !fs::symlink_metadata(path)?.is_dir() { return Err(invalid("offline application registry or owned root is not a directory")); }
    Ok(())
}

fn read_snapshot(data_root: &Path) -> Result<RegistrySnapshot, AppError> {
    existing_plain_root(data_root)?;
    let hosting = data_root.join("app-hosting");
    ensure_no_links(data_root, &hosting)?;
    match fs::symlink_metadata(&hosting) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(RegistrySnapshot {
            inventory: OfflineHostedAppInventory::default(), records: BTreeMap::new(),
        }),
        Err(error) => return Err(error.into()),
        Ok(_) => (),
    }
    require_plain_directory(data_root, &hosting)?;
    let registry = hosting.join("registry");
    let apps = hosting.join("apps");
    require_plain_directory(data_root, &registry)?;
    require_plain_directory(data_root, &apps)?;
    let mut records = BTreeMap::new();
    let mut inventory = OfflineHostedAppInventory::default();
    for entry in fs::read_dir(&registry)? {
        let path = entry?.path();
        ensure_no_links(data_root, &path)?;
        if !fs::symlink_metadata(&path)?.is_file() {
            return Err(invalid("offline application registry contains a non-file entry"));
        }
        if path.extension().is_none_or(|extension| extension != "json") { continue; }
        let bytes = fs::read(&path)?;
        let stored: StoredRecord = serde_json::from_slice(&bytes)?;
        let record = &stored.record;
        validate_id(&record.id)?;
        validate_name(record.name.clone())?;
        validate_metadata(AppMetadata { description: record.description.clone(), tags: record.tags.clone() })?;
        if let Some(session) = &record.development_session_id { validate_id(session)?; }
        if path.file_stem().is_none_or(|stem| stem != record.id.as_str()) {
            return Err(invalid("offline application registry identity mismatch"));
        }
        let directory = apps.join(&record.id);
        if record.directory != directory {
            return Err(invalid("offline application registry points outside its exact owned directory"));
        }
        require_plain_directory(data_root, &directory)?;
        validate_settings(&directory, &record.settings)?;
        if stored.identity.is_some() || record.pid.is_some() || record.state == AppState::Running || record.operation.is_some() {
            return Err(invalid("offline application relocation requires settled process identity, PID, state and operation"));
        }
        inventory.app_ids.insert(record.id.clone());
        inventory.owned_directories.insert(directory);
        // Retain all raw values, including allowed current-generation empty
        // fields. Relocation changes only record.directory, not registry shape.
        let raw = serde_json::from_slice(&bytes)?;
        records.insert(record.id.clone(), (bytes, raw));
    }
    Ok(RegistrySnapshot { inventory, records })
}

/// Read-only source validation. The caller must already stop the Host and hold
/// its dataset locks. No orphan recovery, registry normalization or process
/// termination is performed, including when validation fails.
pub fn inspect_offline_hosted_apps(source_data: &Path) -> Result<OfflineHostedAppInventory, AppError> {
    Ok(read_snapshot(source_data)?.inventory)
}

/// Rebase only copied registrations in an unpublished staging dataset. The
/// logical destination is separate from the staging directory. Retries accept
/// exactly the original copy or the already-rebased value and preserve source
/// bytes. Caller-held dataset locks protect inspection, copy and publication.
pub fn rewrite_offline_hosted_app_registries(source_data: &Path, staging_data: &Path, target_data: &Path)
    -> Result<BTreeSet<String>, AppError> {
    let source = read_snapshot(source_data)?;
    existing_plain_root(staging_data)?;
    absolute_plain_path(target_data)?;
    if source_data == staging_data || source_data == target_data
        || staging_data.starts_with(source_data) || source_data.starts_with(staging_data)
        || target_data.starts_with(source_data) || source_data.starts_with(target_data) {
        return Err(invalid("offline application relocation must use a separate source and destination dataset"));
    }
    let hosting = staging_data.join("app-hosting");
    ensure_no_links(staging_data, &hosting)?;
    if source.records.is_empty() && !hosting.exists() { return Ok(source.inventory.app_ids); }
    require_plain_directory(staging_data, &hosting)?;
    let registry = hosting.join("registry");
    let apps = hosting.join("apps");
    require_plain_directory(staging_data, &registry)?;
    require_plain_directory(staging_data, &apps)?;
    let staged_ids = fs::read_dir(&registry)?.map(|entry| {
        let path = entry?.path();
        ensure_no_links(staging_data, &path)?;
        if !fs::symlink_metadata(&path)?.is_file() { return Err(invalid("copied application registry contains a non-file entry")); }
        if path.extension().is_none_or(|extension| extension != "json") { return Ok(None); }
        let id = path.file_stem().and_then(|name| name.to_str()).ok_or_else(|| invalid("copied application registry identity is not UTF-8"))?;
        validate_id(id)?;
        Ok(Some(id.to_owned()))
    }).collect::<Result<Vec<_>, AppError>>()?.into_iter().flatten().collect::<BTreeSet<_>>();
    if staged_ids != source.inventory.app_ids { return Err(invalid("copied application registrations do not match the validated source")); }
    // Validate every copy before changing any staging file. Arbitrary edited
    // settings/state and forged paths cannot be accepted as relocation retries.
    let mut writes = Vec::new();
    for (id, (original_bytes, original)) in &source.records {
        require_plain_directory(staging_data, &apps.join(id))?;
        let path = registry.join(format!("{id}.json"));
        let current_bytes = fs::read(&path)?;
        let current: Value = serde_json::from_slice(&current_bytes)?;
        let mut rebased = original.clone();
        rebased["record"]["directory"] = serde_json::to_value(target_data.join("app-hosting").join("apps").join(id))?;
        if current_bytes != *original_bytes && current != rebased {
            return Err(invalid("copied application registry changed beyond its exact relocation directory"));
        }
        if current != rebased { writes.push((path, serde_json::to_vec_pretty(&rebased)?)); }
    }
    for (path, bytes) in writes {
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
        ensure_no_links(staging_data, &temporary)?;
        let written = (|| {
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            Ok::<(), std::io::Error>(())
        })();
        if written.is_err() { let _ = fs::remove_file(&temporary); }
        written?;
    }
    Ok(source.inventory.app_ids)
}
