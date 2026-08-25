//! Strict Creative Studio archive readers and writers.
//!
//! An archive is a ZIP with exactly one `manifest.json` plus one content entry
//! for every asset referenced by the canonical project document. The archive
//! has a versioned product manifest. v1 project archives remain readable for
//! compatibility; new canonical exports use the v2 Canvas manifest.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use nomifun_common::{AppError, WorkshopAssetId, zip_safe};
use nomifun_db::WorkshopAssetRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::creative_studio::{
    CREATIVE_STUDIO_SCHEMA, CreativeCanvasDocument, CreativeConfigOperation,
    CreativeGenerationStatus, CreativeNodeData, CreativeProjectDocument,
    MAX_CREATIVE_PROJECT_DOCUMENT_BYTES,
};
use crate::MAX_ASSET_BYTES;

pub const CREATIVE_STUDIO_ARCHIVE_MIME: &str =
    "application/vnd.nomifun.creative-studio+zip";
pub const MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES: usize = 256 * 1024 * 1024;
const CREATIVE_STUDIO_ARCHIVE_KIND: &str = "project-archive";
const CREATIVE_STUDIO_ARCHIVE_VERSION: u32 = 1;
pub const CREATIVE_CANVAS_ARCHIVE_KIND: &str = "canvas-archive";
pub const CREATIVE_CANVAS_ARCHIVE_VERSION: u32 = 2;
// A legal Director v1 sidecar may own 5,000 captures plus 2,000 entity assets.
// Keep the archive below the shared hardened ZIP entry ceiling rather than
// silently making those canonical projects non-exportable.
const MAX_CREATIVE_ARCHIVE_ASSETS: usize = zip_safe::ZipExtractionBudget::DEFAULT_MAX_ENTRIES - 1;
const MAX_CREATIVE_ARCHIVE_ENTRIES: usize = MAX_CREATIVE_ARCHIVE_ASSETS + 1;
const MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES: usize =
    MAX_CREATIVE_PROJECT_DOCUMENT_BYTES + 8 * 1024 * 1024;
const MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES: u64 =
    zip_safe::ZipExtractionBudget::DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES;
const DIRECTOR_PROJECT_KIND: &str = "nomifun.director.project";
const DIRECTOR_PROJECT_VERSION: u64 = 1;
const MAX_DIRECTOR_SIDECAR_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct CreativeArchiveAssetSnapshot {
    pub row: WorkshopAssetRow,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct CreativeArchiveImport {
    pub title: String,
    pub document: CreativeProjectDocument,
    pub assets: Vec<CreativeArchiveImportedAsset>,
}

#[derive(Debug)]
pub(crate) struct CreativeArchiveImportedAsset {
    pub metadata: CreativeArchiveAsset,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeArchiveManifest {
    schema: String,
    kind: String,
    version: u32,
    exported_at: i64,
    project: CreativeArchiveProject,
    assets: Vec<CreativeArchiveAsset>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeArchiveProject {
    title: String,
    document: CreativeProjectDocument,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeCanvasArchiveManifest {
    schema: String,
    kind: String,
    version: u32,
    exported_at: i64,
    canvas: CreativeCanvasArchiveCanvas,
    assets: Vec<CreativeArchiveAsset>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreativeCanvasArchiveCanvas {
    canvas_id: String,
    title: String,
    document: CreativeCanvasDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreativeArchiveAsset {
    pub asset_id: String,
    pub kind: String,
    pub title: String,
    pub collection: Option<String>,
    pub tags: Vec<String>,
    pub mime: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub byte_length: u64,
    pub in_library: bool,
    pub origin: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub content_path: String,
    pub sha256: String,
}

pub(crate) fn build_creative_project_archive(
    title: &str,
    document: &CreativeProjectDocument,
    assets: Vec<CreativeArchiveAssetSnapshot>,
    exported_at: i64,
) -> Result<Vec<u8>, AppError> {
    document.validate_for_project(&document.project_id).map_err(|error| {
        AppError::Conflict(format!(
            "cannot export invalid creative project document: {error}"
        ))
    })?;
    validate_archive_title(title)?;

    let mut snapshots = BTreeMap::new();
    for snapshot in assets {
        let asset_id = snapshot.row.asset_id.clone();
        WorkshopAssetId::parse(&asset_id).map_err(|error| {
            AppError::Conflict(format!(
                "creative project asset {asset_id:?} has an invalid UUIDv7: {error}"
            ))
        })?;
        if snapshots.insert(asset_id.clone(), snapshot).is_some() {
            return Err(AppError::Conflict(format!(
                "duplicate creative project asset {asset_id:?}"
            )));
        }
    }
    let referenced = collect_archive_asset_ids_from_snapshots(document, &snapshots)?;
    if referenced.len() > MAX_CREATIVE_ARCHIVE_ASSETS {
        return Err(AppError::BadRequest(format!(
            "creative project references too many assets: {} (max {MAX_CREATIVE_ARCHIVE_ASSETS})",
            referenced.len()
        )));
    }
    let present = snapshots.keys().cloned().collect::<BTreeSet<_>>();
    if present != referenced {
        return Err(AppError::Conflict(describe_asset_set_mismatch(
            &referenced,
            &present,
        )));
    }

    let mut manifest_assets = Vec::with_capacity(snapshots.len());
    let mut total_uncompressed = 0u64;
    for (asset_id, snapshot) in &snapshots {
        let metadata = archive_asset_from_snapshot(snapshot)?;
        debug_assert_eq!(&metadata.asset_id, asset_id);
        total_uncompressed = total_uncompressed
            .checked_add(metadata.byte_length)
            .ok_or_else(|| AppError::BadRequest("creative archive size overflow".into()))?;
        if total_uncompressed > MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES {
            return Err(AppError::BadRequest(format!(
                "creative archive assets exceed {MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES} bytes"
            )));
        }
        manifest_assets.push(metadata);
    }

    let manifest = CreativeArchiveManifest {
        schema: CREATIVE_STUDIO_SCHEMA.to_owned(),
        kind: CREATIVE_STUDIO_ARCHIVE_KIND.to_owned(),
        version: CREATIVE_STUDIO_ARCHIVE_VERSION,
        exported_at,
        project: CreativeArchiveProject {
            title: title.to_owned(),
            document: document.clone(),
        },
        assets: manifest_assets,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| AppError::Internal(format!("encode creative archive manifest: {error}")))?;
    if manifest_bytes.len() > MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative archive manifest is too large: {} bytes",
            manifest_bytes.len()
        )));
    }
    total_uncompressed = total_uncompressed
        .checked_add(manifest_bytes.len() as u64)
        .ok_or_else(|| AppError::BadRequest("creative archive size overflow".into()))?;
    if total_uncompressed > MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative archive expands beyond {MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES} bytes"
        )));
    }

    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    writer
        .start_file("manifest.json", options)
        .map_err(internal_zip_error)?;
    writer
        .write_all(&manifest_bytes)
        .map_err(|error| AppError::Internal(format!("write creative archive manifest: {error}")))?;
    for metadata in &manifest.assets {
        let snapshot = snapshots
            .get(&metadata.asset_id)
            .expect("manifest assets originate from the snapshot map");
        writer
            .start_file(&metadata.content_path, options)
            .map_err(internal_zip_error)?;
        writer.write_all(&snapshot.bytes).map_err(|error| {
            AppError::Internal(format!(
                "write creative archive asset {}: {error}",
                metadata.asset_id
            ))
        })?;
    }
    let bytes = writer.finish().map_err(internal_zip_error)?.into_inner();
    if bytes.len() > MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative archive is too large: {} compressed bytes",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Build the canonical Canvas v2 archive while retaining the v1 asset
/// closure/metadata writer as the single source of truth. The v1 bytes are
/// decoded as structured data and rewritten with the product-facing Canvas
/// envelope; no string replacement is used for document or asset JSON.
pub(crate) fn build_creative_canvas_archive(
    title: &str,
    document: &CreativeProjectDocument,
    assets: Vec<CreativeArchiveAssetSnapshot>,
    exported_at: i64,
) -> Result<Vec<u8>, AppError> {
    let legacy_bytes = build_creative_project_archive(title, document, assets, exported_at)?;
    let mut entries = read_archive_entries(
        &legacy_bytes,
        MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES,
        MAX_CREATIVE_ARCHIVE_ENTRIES,
        "creative project archive",
    )?;
    let manifest_bytes = entries
        .remove(Path::new("manifest.json"))
        .ok_or_else(|| AppError::Internal("generated creative archive is missing manifest".into()))?;
    let legacy_manifest: CreativeArchiveManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| {
            AppError::Internal(format!("decode generated creative archive manifest: {error}"))
        })?;
    validate_manifest_envelope(&legacy_manifest).map_err(|error| {
        AppError::Internal(format!("generated creative archive failed validation: {error}"))
    })?;

    let canvas_document = CreativeCanvasDocument::from(legacy_manifest.project.document);
    let canvas_id = canvas_document.canvas_id.clone();
    let manifest = CreativeCanvasArchiveManifest {
        schema: legacy_manifest.schema,
        kind: CREATIVE_CANVAS_ARCHIVE_KIND.to_owned(),
        version: CREATIVE_CANVAS_ARCHIVE_VERSION,
        exported_at: legacy_manifest.exported_at,
        canvas: CreativeCanvasArchiveCanvas {
            canvas_id: canvas_id.clone(),
            title: legacy_manifest.project.title,
            document: canvas_document,
        },
        assets: legacy_manifest.assets,
    };
    validate_canvas_manifest_envelope(&manifest).map_err(|error| {
        AppError::Internal(format!("generated creative Canvas archive failed validation: {error}"))
    })?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| AppError::Internal(format!("encode creative Canvas archive manifest: {error}")))?;
    if manifest_bytes.len() > MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative Canvas archive manifest is too large: {} bytes",
            manifest_bytes.len()
        )));
    }
    entries.insert(PathBuf::from("manifest.json"), manifest_bytes);
    let entries = entries
        .into_iter()
        .map(|(path, bytes)| (archive_path_string(&path), bytes))
        .collect::<BTreeMap<_, _>>();
    write_archive_entries(entries, "creative Canvas archive")
}

pub(crate) fn parse_creative_project_archive(
    bytes: &[u8],
) -> Result<CreativeArchiveImport, AppError> {
    parse_creative_project_archive_with_limits(
        bytes,
        MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES,
        MAX_CREATIVE_ARCHIVE_ENTRIES,
    )
}

/// Parse either a legacy v1 project archive or the canonical v2 Canvas
/// archive. Both paths end in the unchanged v1 asset/document validator and
/// therefore produce the same normalized import representation.
pub(crate) fn parse_creative_archive(
    bytes: &[u8],
) -> Result<CreativeArchiveImport, AppError> {
    let mut entries = read_archive_entries(
        bytes,
        MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES,
        MAX_CREATIVE_ARCHIVE_ENTRIES,
        "creative archive",
    )?;
    let manifest_bytes = entries
        .remove(Path::new("manifest.json"))
        .ok_or_else(|| AppError::BadRequest("creative archive is missing manifest.json".into()))?;
    let value: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        AppError::BadRequest(format!("invalid creative archive manifest: {error}"))
    })?;
    let version = value.get("version").and_then(Value::as_u64);
    let kind = value.get("kind").and_then(Value::as_str);
    if version == Some(CREATIVE_STUDIO_ARCHIVE_VERSION as u64)
        || kind == Some(CREATIVE_STUDIO_ARCHIVE_KIND)
    {
        return parse_creative_project_archive(bytes);
    }
    if version != Some(CREATIVE_CANVAS_ARCHIVE_VERSION as u64)
        && kind != Some(CREATIVE_CANVAS_ARCHIVE_KIND)
    {
        return Err(AppError::BadRequest(
            "creative archive has an unsupported kind/version".into(),
        ));
    }

    let manifest: CreativeCanvasArchiveManifest =
        serde_json::from_value(value).map_err(|error| {
            AppError::BadRequest(format!("invalid creative Canvas archive manifest: {error}"))
        })?;
    validate_canvas_manifest_envelope(&manifest)?;
    let canvas_document = manifest.canvas.document;
    let project_document = canvas_document.into_project_document();
    project_document
        .validate_for_project(&manifest.canvas.canvas_id)
        .map_err(|error| {
            AppError::BadRequest(format!(
                "invalid creative Canvas document in archive: {error}"
            ))
        })?;
    let legacy_manifest = CreativeArchiveManifest {
        schema: manifest.schema,
        kind: CREATIVE_STUDIO_ARCHIVE_KIND.to_owned(),
        version: CREATIVE_STUDIO_ARCHIVE_VERSION,
        exported_at: manifest.exported_at,
        project: CreativeArchiveProject {
            title: manifest.canvas.title,
            document: project_document,
        },
        assets: manifest.assets,
    };
    let legacy_manifest_bytes = serde_json::to_vec(&legacy_manifest).map_err(|error| {
        AppError::Internal(format!("normalize creative Canvas archive manifest: {error}"))
    })?;
    entries.insert(PathBuf::from("manifest.json"), legacy_manifest_bytes);
    let legacy_entries = entries
        .into_iter()
        .map(|(path, bytes)| (archive_path_string(&path), bytes))
        .collect::<BTreeMap<_, _>>();
    let legacy_bytes = write_archive_entries(legacy_entries, "creative Canvas archive")?;
    parse_creative_project_archive(&legacy_bytes)
}

fn read_archive_entries(
    bytes: &[u8],
    max_uncompressed_bytes: u64,
    max_entries: usize,
    label: &str,
) -> Result<HashMap<PathBuf, Vec<u8>>, AppError> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest(format!("{label} is empty")));
    }
    if bytes.len() > MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "{label} exceeds {MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES} compressed bytes"
        )));
    }

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| AppError::BadRequest(format!("not a {label}")))?;
    let mut budget = zip_safe::ZipExtractionBudget::new(max_uncompressed_bytes, max_entries);
    budget
        .check_entry_count(archive.len())
        .map_err(|error| AppError::BadRequest(error.to_string()))?;

    let mut entries = HashMap::<PathBuf, Vec<u8>>::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            AppError::BadRequest(format!("corrupt {label}: {error}"))
        })?;
        let entry_name = entry.name().to_owned();
        if entry.encrypted() || zip_safe::zip_entry_is_symlink(entry.unix_mode()) {
            return Err(AppError::BadRequest(format!(
                "unsafe {label} entry: {entry_name}"
            )));
        }
        let path = zip_safe::safe_zip_entry_path(&entry_name, zip_safe::ZipColonPolicy::RejectAll)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "unsafe {label} entry: {entry_name}"
                ))
            })?;
        if entry.is_dir() || !is_allowed_archive_path(&path) {
            return Err(AppError::BadRequest(format!(
                "unsupported {label} entry: {entry_name}"
            )));
        }
        if entries.contains_key(&path) {
            return Err(AppError::BadRequest(format!(
                "duplicate {label} entry: {entry_name}"
            )));
        }

        let entry_limit = if path == Path::new("manifest.json") {
            MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES
        } else {
            MAX_ASSET_BYTES
        };
        if entry.size() > entry_limit as u64 {
            return Err(AppError::BadRequest(format!(
                "{label} entry is too large: {entry_name}"
            )));
        }
        let mut content = Vec::with_capacity((entry.size() as usize).min(entry_limit));
        (&mut entry)
            .take(entry_limit as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|error| {
                AppError::BadRequest(format!(
                    "cannot read {label} entry {entry_name}: {error}"
                ))
            })?;
        if content.len() > entry_limit {
            return Err(AppError::BadRequest(format!(
                "{label} entry is too large: {entry_name}"
            )));
        }
        budget
            .record_written(content.len() as u64)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        entries.insert(path, content);
    }
    Ok(entries)
}

fn write_archive_entries(
    entries: BTreeMap<String, Vec<u8>>,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    let total_uncompressed = entries.values().try_fold(0u64, |total, bytes| {
        total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| AppError::BadRequest(format!("{label} size overflow")))
    })?;
    if total_uncompressed > MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "{label} expands beyond {MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES} bytes"
        )));
    }

    let cursor = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    for (path, bytes) in entries {
        writer.start_file(path, options).map_err(internal_zip_error)?;
        writer
            .write_all(&bytes)
            .map_err(|error| AppError::Internal(format!("write {label} entry: {error}")))?;
    }
    let bytes = writer.finish().map_err(internal_zip_error)?.into_inner();
    if bytes.len() > MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "{label} is too large: {} compressed bytes",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn archive_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_creative_project_archive_with_limits(
    bytes: &[u8],
    max_uncompressed_bytes: u64,
    max_entries: usize,
) -> Result<CreativeArchiveImport, AppError> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest("creative project archive is empty".into()));
    }
    if bytes.len() > MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES {
        return Err(AppError::BadRequest(format!(
            "creative project archive exceeds {MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES} compressed bytes"
        )));
    }

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| AppError::BadRequest("not a Creative Studio v1 project archive".into()))?;
    let mut budget = zip_safe::ZipExtractionBudget::new(max_uncompressed_bytes, max_entries);
    budget
        .check_entry_count(archive.len())
        .map_err(|error| AppError::BadRequest(error.to_string()))?;

    let mut entries = HashMap::<PathBuf, Vec<u8>>::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            AppError::BadRequest(format!("corrupt creative project archive: {error}"))
        })?;
        let entry_name = entry.name().to_owned();
        if entry.encrypted() || zip_safe::zip_entry_is_symlink(entry.unix_mode()) {
            return Err(AppError::BadRequest(format!(
                "unsafe creative project archive entry: {entry_name}"
            )));
        }
        let path = zip_safe::safe_zip_entry_path(&entry_name, zip_safe::ZipColonPolicy::RejectAll)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "unsafe creative project archive entry: {entry_name}"
                ))
            })?;
        if entry.is_dir() || !is_allowed_archive_path(&path) {
            return Err(AppError::BadRequest(format!(
                "unsupported creative project archive entry: {entry_name}"
            )));
        }
        if entries.contains_key(&path) {
            return Err(AppError::BadRequest(format!(
                "duplicate creative project archive entry: {entry_name}"
            )));
        }

        let entry_limit = if path == Path::new("manifest.json") {
            MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES
        } else {
            MAX_ASSET_BYTES
        };
        if entry.size() > entry_limit as u64 {
            return Err(AppError::BadRequest(format!(
                "creative project archive entry is too large: {entry_name}"
            )));
        }
        let mut content = Vec::with_capacity((entry.size() as usize).min(entry_limit));
        (&mut entry)
            .take(entry_limit as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|error| {
                AppError::BadRequest(format!(
                    "cannot read creative project archive entry {entry_name}: {error}"
                ))
            })?;
        if content.len() > entry_limit {
            return Err(AppError::BadRequest(format!(
                "creative project archive entry is too large: {entry_name}"
            )));
        }
        budget
            .record_written(content.len() as u64)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        entries.insert(path, content);
    }

    let manifest_bytes = entries
        .remove(Path::new("manifest.json"))
        .ok_or_else(|| AppError::BadRequest("creative project archive is missing manifest.json".into()))?;
    let manifest: CreativeArchiveManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        AppError::BadRequest(format!("invalid creative project archive manifest: {error}"))
    })?;
    validate_manifest_envelope(&manifest)?;
    validate_archive_title(&manifest.project.title)?;
    manifest
        .project
        .document
        .validate_for_project(&manifest.project.document.project_id)
        .map_err(|error| {
            AppError::BadRequest(format!(
                "invalid creative project document in archive: {error}"
            ))
        })?;

    if manifest.assets.len() > MAX_CREATIVE_ARCHIVE_ASSETS {
        return Err(AppError::BadRequest(format!(
            "creative project archive has too many assets: {}",
            manifest.assets.len()
        )));
    }
    let mut referenced = collect_document_asset_ids(&manifest.project.document)?;
    let mut declared = BTreeSet::new();
    let mut imported_assets = Vec::with_capacity(manifest.assets.len());
    for metadata in manifest.assets {
        validate_archive_asset_metadata(&metadata)?;
        if !declared.insert(metadata.asset_id.clone()) {
            return Err(AppError::BadRequest(format!(
                "duplicate creative project archive asset {:?}",
                metadata.asset_id
            )));
        }
        let path = PathBuf::from(&metadata.content_path);
        let content = entries.remove(&path).ok_or_else(|| {
            AppError::BadRequest(format!(
                "creative project archive is missing {}",
                metadata.content_path
            ))
        })?;
        if metadata.byte_length != content.len() as u64 {
            return Err(AppError::BadRequest(format!(
                "creative project archive asset {} byte length does not match its manifest",
                metadata.asset_id
            )));
        }
        let actual_sha256 = sha256_bytes(&content);
        if metadata.sha256 != actual_sha256 {
            return Err(AppError::BadRequest(format!(
                "creative project archive asset {} failed SHA-256 verification",
                metadata.asset_id
            )));
        }
        validate_asset_content(&metadata, &content)?;
        imported_assets.push(CreativeArchiveImportedAsset {
            metadata,
            bytes: content,
        });
    }
    if !entries.is_empty() {
        let extra = entries
            .keys()
            .next()
            .expect("non-empty map has one key")
            .display();
        return Err(AppError::BadRequest(format!(
            "creative project archive contains undeclared entry {extra}"
        )));
    }
    extend_archive_asset_ids_from_import(
        &manifest.project.document,
        &imported_assets,
        &mut referenced,
    )?;
    if declared != referenced {
        return Err(AppError::BadRequest(describe_asset_set_mismatch(
            &referenced,
            &declared,
        )));
    }

    Ok(CreativeArchiveImport {
        title: manifest.project.title,
        document: manifest.project.document,
        assets: imported_assets,
    })
}

pub(crate) fn remap_creative_archive_for_import(
    mut archive: CreativeArchiveImport,
    new_project_id: &str,
) -> Result<CreativeArchiveImport, AppError> {
    let old_project_id = archive.document.project_id.clone();
    let mut asset_ids = BTreeMap::new();
    for asset in &archive.assets {
        asset_ids.insert(
            asset.metadata.asset_id.clone(),
            WorkshopAssetId::new().into_string(),
        );
    }

    let mut node_ids = BTreeMap::new();
    for node in &archive.document.nodes {
        node_ids.insert(node.id.clone(), uuid::Uuid::now_v7().to_string());
    }
    remap_archive_director_sidecars(
        &archive.document,
        &mut archive.assets,
        &old_project_id,
        new_project_id,
        &asset_ids,
    )?;
    for node in &mut archive.document.nodes {
        node.id = node_ids
            .get(&node.id)
            .expect("node map was built from this document")
            .clone();
        if let Some(group_id) = node.group_id.as_mut() {
            *group_id = node_ids.get(group_id).cloned().ok_or_else(|| {
                AppError::BadRequest("creative archive contains a dangling groupId".into())
            })?;
        }
        remap_node_references(&mut node.data, &asset_ids, &node_ids)?;
        if let CreativeNodeData::Config(config) = &mut node.data {
            config.task_id = None;
            if matches!(
                config.status,
                CreativeGenerationStatus::Queued | CreativeGenerationStatus::Running
            ) {
                config.status = CreativeGenerationStatus::Idle;
            }
        }
    }
    for connection in &mut archive.document.connections {
        connection.id = uuid::Uuid::now_v7().to_string();
        connection.source_node_id = node_ids
            .get(&connection.source_node_id)
            .cloned()
            .ok_or_else(|| {
                AppError::BadRequest(
                    "creative archive contains a dangling connection source".into(),
                )
            })?;
        connection.target_node_id = node_ids
            .get(&connection.target_node_id)
            .cloned()
            .ok_or_else(|| {
                AppError::BadRequest(
                    "creative archive contains a dangling connection target".into(),
                )
            })?;
    }

    let mut chat_ids = BTreeMap::new();
    for chat in &archive.document.chat_sessions {
        chat_ids.insert(chat.id.clone(), uuid::Uuid::now_v7().to_string());
    }
    for chat in &mut archive.document.chat_sessions {
        chat.id = chat_ids
            .get(&chat.id)
            .expect("chat map was built from this document")
            .clone();
        // Conversation messages live outside the archive and therefore cannot
        // be imported as valid references.
        chat.message_ids.clear();
        // An idempotency key can only resume the exporting installation's
        // dedicated Conversation. Imported sessions retain their selected
        // model but always begin without an in-flight turn.
        chat.pending_turn = None;
    }
    if let Some(active_chat_id) = archive.document.active_chat_id.as_mut() {
        *active_chat_id = chat_ids.get(active_chat_id).cloned().ok_or_else(|| {
            AppError::BadRequest("creative archive contains a dangling activeChatId".into())
        })?;
    }
    archive.document.pending_task_ids.clear();
    archive.document.project_id = new_project_id.to_owned();

    for asset in &mut archive.assets {
        asset.metadata.asset_id = asset_ids
            .get(&asset.metadata.asset_id)
            .expect("asset map was built from this archive")
            .clone();
        asset.metadata.content_path = asset_content_path(&asset.metadata.asset_id);
    }
    let mut remapped_references = collect_document_asset_ids(&archive.document)?;
    extend_archive_asset_ids_from_import(
        &archive.document,
        &archive.assets,
        &mut remapped_references,
    )?;
    let remapped_assets = archive
        .assets
        .iter()
        .map(|asset| asset.metadata.asset_id.clone())
        .collect::<BTreeSet<_>>();
    if remapped_assets != remapped_references {
        return Err(AppError::BadRequest(describe_asset_set_mismatch(
            &remapped_references,
            &remapped_assets,
        )));
    }
    archive
        .document
        .validate_for_project(new_project_id)
        .map_err(|error| {
            AppError::BadRequest(format!(
                "remapped creative project archive is invalid: {error}"
            ))
        })?;
    Ok(archive)
}

pub(crate) fn sanitized_archive_origin(origin: Option<Value>) -> Result<Option<String>, AppError> {
    let Some(Value::Object(mut object)) = origin else {
        return match origin {
            None => Ok(None),
            Some(_) => Err(AppError::BadRequest(
                "creative archive asset origin must be a JSON object".into(),
            )),
        };
    };
    // Provider/task/canvas/message data is not part of a project archive. Keep
    // portable provenance such as prompt/model/parameters, but remove every
    // durable reference that could point into another installation.
    for key in [
        "provider_id",
        "project_id",
        "template_id",
        "template_run_id",
        "template_step_id",
        "canvas_id",
        "node_id",
        "creation_task_id",
        "task_id",
        "providerId",
        "projectId",
        "templateId",
        "templateRunId",
        "templateStepId",
        "canvasId",
        "nodeId",
        "creationTaskId",
    ] {
        object.remove(key);
    }
    if object.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(&object)
        .map(Some)
        .map_err(|error| AppError::Internal(format!("encode imported asset origin: {error}")))
}

fn archive_asset_from_snapshot(
    snapshot: &CreativeArchiveAssetSnapshot,
) -> Result<CreativeArchiveAsset, AppError> {
    let row = &snapshot.row;
    let tags = serde_json::from_str::<Vec<String>>(&row.tags).map_err(|error| {
        AppError::Conflict(format!(
            "creative project asset {} has invalid tags: {error}",
            row.asset_id
        ))
    })?;
    let origin = row
        .origin
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| {
            AppError::Conflict(format!(
                "creative project asset {} has invalid origin: {error}",
                row.asset_id
            ))
        })?;
    let mime = if row.kind == "text" {
        "text/plain; charset=utf-8".to_owned()
    } else {
        row.mime.clone().ok_or_else(|| {
            AppError::Conflict(format!(
                "creative project asset {} has no MIME type",
                row.asset_id
            ))
        })?
    };
    let metadata = CreativeArchiveAsset {
        asset_id: row.asset_id.clone(),
        kind: row.kind.clone(),
        title: row.title.clone(),
        collection: row.collection.clone(),
        tags,
        mime,
        width: row.width,
        height: row.height,
        byte_length: snapshot.bytes.len() as u64,
        in_library: row.in_library,
        origin,
        created_at: row.created_at,
        updated_at: row.updated_at,
        content_path: asset_content_path(&row.asset_id),
        sha256: sha256_bytes(&snapshot.bytes),
    };
    validate_archive_asset_metadata(&metadata).map_err(|error| match error {
        AppError::BadRequest(message) => AppError::Conflict(format!(
            "cannot export invalid creative project asset {}: {message}",
            row.asset_id
        )),
        other => other,
    })?;
    validate_asset_content(&metadata, &snapshot.bytes).map_err(|error| match error {
        AppError::BadRequest(message) => AppError::Conflict(format!(
            "cannot export invalid creative project asset {}: {message}",
            row.asset_id
        )),
        other => other,
    })?;
    Ok(metadata)
}

fn validate_manifest_envelope(manifest: &CreativeArchiveManifest) -> Result<(), AppError> {
    if manifest.schema != CREATIVE_STUDIO_SCHEMA {
        return Err(AppError::BadRequest(format!(
            "creative project archive schema must be {CREATIVE_STUDIO_SCHEMA:?}"
        )));
    }
    if manifest.kind != CREATIVE_STUDIO_ARCHIVE_KIND {
        return Err(AppError::BadRequest(
            "archive is not a Creative Studio project archive".into(),
        ));
    }
    if manifest.version != CREATIVE_STUDIO_ARCHIVE_VERSION {
        return Err(AppError::BadRequest(format!(
            "creative project archive version must be exactly {CREATIVE_STUDIO_ARCHIVE_VERSION}"
        )));
    }
    if manifest.exported_at < 0 {
        return Err(AppError::BadRequest(
            "creative project archive exportedAt must be non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_canvas_manifest_envelope(
    manifest: &CreativeCanvasArchiveManifest,
) -> Result<(), AppError> {
    if manifest.schema != CREATIVE_STUDIO_SCHEMA {
        return Err(AppError::BadRequest(format!(
            "creative Canvas archive schema must be {CREATIVE_STUDIO_SCHEMA:?}"
        )));
    }
    if manifest.kind != CREATIVE_CANVAS_ARCHIVE_KIND {
        return Err(AppError::BadRequest(
            "archive is not a Creative Studio Canvas archive".into(),
        ));
    }
    if manifest.version != CREATIVE_CANVAS_ARCHIVE_VERSION {
        return Err(AppError::BadRequest(format!(
            "creative Canvas archive version must be exactly {CREATIVE_CANVAS_ARCHIVE_VERSION}"
        )));
    }
    if manifest.exported_at < 0 {
        return Err(AppError::BadRequest(
            "creative Canvas archive exportedAt must be non-negative".into(),
        ));
    }
    nomifun_common::validate_uuidv7(&manifest.canvas.canvas_id).map_err(|error| {
        AppError::BadRequest(format!(
            "creative Canvas archive canvasId must be a canonical UUIDv7: {error}"
        ))
    })?;
    validate_archive_title(&manifest.canvas.title)?;
    manifest
        .canvas
        .document
        .validate_for_canvas(&manifest.canvas.canvas_id)
        .map_err(|error| {
            AppError::BadRequest(format!(
                "invalid creative Canvas document in archive: {error}"
            ))
        })?;
    Ok(())
}

fn validate_archive_title(title: &str) -> Result<(), AppError> {
    let chars = title.encode_utf16().count();
    if title.trim().is_empty() || title.trim() != title || chars > 1_000 {
        return Err(AppError::BadRequest(
            "creative project archive title must be trimmed, non-empty, and at most 1000 UTF-16 code units"
                .into(),
        ));
    }
    Ok(())
}

fn validate_archive_asset_metadata(asset: &CreativeArchiveAsset) -> Result<(), AppError> {
    WorkshopAssetId::parse(&asset.asset_id).map_err(|error| {
        AppError::BadRequest(format!(
            "creative archive assetId {:?} is not a UUIDv7: {error}",
            asset.asset_id
        ))
    })?;
    if asset.content_path != asset_content_path(&asset.asset_id) {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has a non-canonical contentPath",
            asset.asset_id
        )));
    }
    if asset.title.trim().is_empty()
        || asset.title.trim() != asset.title
        || asset.title.encode_utf16().count() > 1_000
    {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has an invalid title",
            asset.asset_id
        )));
    }
    if asset
        .collection
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value != value.trim() || value.encode_utf16().count() > 1_000)
    {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has an invalid collection",
            asset.asset_id
        )));
    }
    if asset.tags.len() > 256
        || asset
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag != tag.trim() || tag.encode_utf16().count() > 256)
        || asset.tags.iter().collect::<HashSet<_>>().len() != asset.tags.len()
    {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has invalid tags",
            asset.asset_id
        )));
    }
    if asset.created_at < 0 || asset.updated_at < asset.created_at {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has invalid timestamps",
            asset.asset_id
        )));
    }
    if asset.byte_length > MAX_ASSET_BYTES as u64 {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} exceeds {MAX_ASSET_BYTES} bytes",
            asset.asset_id
        )));
    }
    if asset.sha256.len() != 64
        || asset
            .sha256
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} has an invalid SHA-256 digest",
            asset.asset_id
        )));
    }
    if asset.origin.as_ref().is_some_and(|origin| !origin.is_object()) {
        return Err(AppError::BadRequest(format!(
            "creative archive asset {} origin must be a JSON object",
            asset.asset_id
        )));
    }
    match asset.kind.as_str() {
        "text" => {
            if asset.mime != "text/plain; charset=utf-8"
                || asset.width.is_some()
                || asset.height.is_some()
            {
                return Err(AppError::BadRequest(format!(
                    "creative archive text asset {} has invalid media metadata",
                    asset.asset_id
                )));
            }
        }
        "image" | "video" | "audio" => {
            let expected_prefix = format!("{}/", asset.kind);
            if !asset.mime.to_ascii_lowercase().starts_with(&expected_prefix) {
                return Err(AppError::BadRequest(format!(
                    "creative archive asset {} kind does not match MIME type",
                    asset.asset_id
                )));
            }
            if asset.byte_length == 0 {
                return Err(AppError::BadRequest(format!(
                    "creative archive binary asset {} is empty",
                    asset.asset_id
                )));
            }
            let dimensions_valid = match (asset.width, asset.height) {
                (None, None) => true,
                (Some(width), Some(height)) => {
                    asset.kind == "image" && width > 0 && height > 0
                }
                _ => false,
            };
            if !dimensions_valid {
                return Err(AppError::BadRequest(format!(
                    "creative archive asset {} has invalid dimensions",
                    asset.asset_id
                )));
            }
        }
        _ => {
            return Err(AppError::BadRequest(format!(
                "creative archive asset {} has unsupported kind {:?}",
                asset.asset_id, asset.kind
            )));
        }
    }
    Ok(())
}

fn validate_asset_content(
    metadata: &CreativeArchiveAsset,
    content: &[u8],
) -> Result<(), AppError> {
    if metadata.kind == "text" {
        std::str::from_utf8(content).map_err(|_| {
            AppError::BadRequest(format!(
                "creative archive text asset {} is not valid UTF-8",
                metadata.asset_id
            ))
        })?;
    }
    if metadata.kind == "image" {
        let declared = metadata.width.zip(metadata.height);
        let actual = crate::imagemeta::image_dimensions(content)
            .map(|(width, height)| (i64::from(width), i64::from(height)));
        if declared != actual {
            return Err(AppError::BadRequest(format!(
                "creative archive image asset {} dimensions do not match its content",
                metadata.asset_id
            )));
        }
    }
    Ok(())
}

pub(crate) fn collect_document_asset_ids(
    document: &CreativeProjectDocument,
) -> Result<BTreeSet<String>, AppError> {
    let mut asset_ids = BTreeSet::new();
    let node_ids = document
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    for node in &document.nodes {
        match &node.data {
            CreativeNodeData::Image(data) => insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?,
            CreativeNodeData::Panorama(data) => {
                insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?
            }
            CreativeNodeData::Config(data) => {
                for asset_id in data.input_asset_ids.iter().chain(&data.result_asset_ids) {
                    insert_asset(&mut asset_ids, asset_id)?;
                }
                collect_config_operation_asset_ids(data, &mut asset_ids, &node_ids)?;
            }
            CreativeNodeData::Video(data) => {
                insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?;
                insert_optional_asset(&mut asset_ids, data.poster_asset_id.as_deref())?;
            }
            CreativeNodeData::Audio(data) => {
                insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?
            }
            CreativeNodeData::Director(data) => {
                insert_optional_asset(&mut asset_ids, data.scene_id.as_deref())?
            }
            CreativeNodeData::Text(_) | CreativeNodeData::Group(_) => {}
        }
    }
    Ok(asset_ids)
}

fn insert_optional_asset(
    ids: &mut BTreeSet<String>,
    asset_id: Option<&str>,
) -> Result<(), AppError> {
    if let Some(asset_id) = asset_id {
        insert_asset(ids, asset_id)?;
    }
    Ok(())
}

fn insert_asset(ids: &mut BTreeSet<String>, asset_id: &str) -> Result<(), AppError> {
    WorkshopAssetId::parse(asset_id).map_err(|error| {
        AppError::BadRequest(format!(
            "creative project references invalid assetId {asset_id:?}: {error}"
        ))
    })?;
    ids.insert(asset_id.to_owned());
    Ok(())
}

fn collect_config_operation_asset_ids(
    data: &crate::creative_studio::CreativeConfigNodeData,
    asset_ids: &mut BTreeSet<String>,
    node_ids: &BTreeSet<&str>,
) -> Result<(), AppError> {
    let Some(operation) = &data.operation else {
        return Ok(());
    };
    match operation {
        CreativeConfigOperation::ImageNodeCompose {
            source_node_id,
            source_asset_id,
        }
        | CreativeConfigOperation::VideoNodeCompose {
            source_node_id,
            source_asset_id,
        }
        | CreativeConfigOperation::AudioNodeCompose {
            source_node_id,
            source_asset_id,
        } => {
            validate_config_source_node(source_node_id, operation, node_ids)?;
            if let Some(asset_id) = source_asset_id {
                insert_asset(asset_ids, asset_id)?;
            }
        }
        CreativeConfigOperation::ImageMaskEdit {
            source_node_id,
            source_asset_id,
            marked_reference_asset_id,
        } => {
            validate_config_source_node(source_node_id, operation, node_ids)?;
            insert_asset(asset_ids, source_asset_id)?;
            insert_asset(asset_ids, marked_reference_asset_id)?;
        }
    }
    Ok(())
}

fn validate_config_source_node(
    source_node_id: &str,
    operation: &CreativeConfigOperation,
    node_ids: &BTreeSet<&str>,
) -> Result<(), AppError> {
    if !node_ids.contains(source_node_id) {
        return Err(AppError::BadRequest(format!(
            "creative config operation {operation:?} references missing source node {source_node_id:?}"
        )));
    }
    Ok(())
}

fn remap_config_identity(
    value: &mut String,
    identities: &BTreeMap<String, String>,
    identity_kind: &str,
) -> Result<(), AppError> {
    let new_id = identities.get(value).cloned().ok_or_else(|| {
        AppError::BadRequest(format!(
            "creative archive config operation references undeclared {identity_kind} {value:?}"
        ))
    })?;
    *value = new_id;
    Ok(())
}

fn remap_config_operation(
    data: &mut crate::creative_studio::CreativeConfigNodeData,
    asset_ids: &BTreeMap<String, String>,
    node_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    let Some(operation) = &mut data.operation else {
        return Ok(());
    };
    match operation {
        CreativeConfigOperation::ImageNodeCompose {
            source_node_id,
            source_asset_id,
        }
        | CreativeConfigOperation::VideoNodeCompose {
            source_node_id,
            source_asset_id,
        }
        | CreativeConfigOperation::AudioNodeCompose {
            source_node_id,
            source_asset_id,
        } => {
            remap_config_identity(source_node_id, node_ids, "node")?;
            if let Some(asset_id) = source_asset_id {
                remap_config_identity(asset_id, asset_ids, "asset")?;
            }
            Ok(())
        }
        CreativeConfigOperation::ImageMaskEdit {
            source_node_id,
            source_asset_id,
            marked_reference_asset_id,
        } => {
            remap_config_identity(source_node_id, node_ids, "node")?;
            remap_config_identity(source_asset_id, asset_ids, "asset")?;
            remap_config_identity(marked_reference_asset_id, asset_ids, "asset")
        }
    }
}

fn director_scene_asset_ids(document: &CreativeProjectDocument) -> BTreeSet<String> {
    document
        .nodes
        .iter()
        .filter_map(|node| match &node.data {
            CreativeNodeData::Director(data) => data.scene_id.clone(),
            _ => None,
        })
        .collect()
}

fn parse_director_sidecar(
    bytes: &[u8],
    expected_project_id: &str,
) -> Result<Value, String> {
    if bytes.len() > MAX_DIRECTOR_SIDECAR_BYTES {
        return Err(format!(
            "Director sidecar exceeds {MAX_DIRECTOR_SIDECAR_BYTES} bytes"
        ));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Director sidecar is not valid JSON: {error}"))?;
    let root = value
        .as_object()
        .ok_or_else(|| "Director sidecar root must be an object".to_owned())?;
    let expected_keys = ["kind", "version", "project"].into_iter().collect::<BTreeSet<_>>();
    let actual_keys = root.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_keys != expected_keys {
        return Err("Director sidecar must contain exactly kind, version, and project".into());
    }
    if root.get("kind").and_then(Value::as_str) != Some(DIRECTOR_PROJECT_KIND) {
        return Err(format!(
            "Director sidecar kind must be {DIRECTOR_PROJECT_KIND:?}"
        ));
    }
    if root.get("version").and_then(Value::as_u64) != Some(DIRECTOR_PROJECT_VERSION) {
        return Err(format!(
            "Director sidecar version must be {DIRECTOR_PROJECT_VERSION}"
        ));
    }
    let project = root
        .get("project")
        .and_then(Value::as_object)
        .ok_or_else(|| "Director sidecar project must be an object".to_owned())?;
    let project_id = project
        .get("projectId")
        .and_then(Value::as_str)
        .ok_or_else(|| "Director sidecar project.projectId must be a string".to_owned())?;
    if project_id != expected_project_id {
        return Err(format!(
            "Director sidecar projectId {project_id:?} does not match Creative Studio project {expected_project_id:?}"
        ));
    }
    Ok(value)
}

fn collect_nested_asset_ids(
    value: &Value,
    path: &str,
    asset_ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if key == "assetId" {
                    let asset_id = child.as_str().ok_or_else(|| {
                        format!("Director sidecar {child_path} must be an asset id string")
                    })?;
                    WorkshopAssetId::parse(asset_id).map_err(|error| {
                        format!("Director sidecar {child_path} is invalid: {error}")
                    })?;
                    asset_ids.insert(asset_id.to_owned());
                } else {
                    collect_nested_asset_ids(child, &child_path, asset_ids)?;
                }
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_nested_asset_ids(child, &format!("{path}[{index}]"), asset_ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn director_sidecar_asset_ids(
    bytes: &[u8],
    expected_project_id: &str,
) -> Result<BTreeSet<String>, String> {
    let value = parse_director_sidecar(bytes, expected_project_id)?;
    let mut asset_ids = BTreeSet::new();
    collect_nested_asset_ids(&value, "$", &mut asset_ids)?;
    Ok(asset_ids)
}

fn collect_archive_asset_ids_from_snapshots(
    document: &CreativeProjectDocument,
    snapshots: &BTreeMap<String, CreativeArchiveAssetSnapshot>,
) -> Result<BTreeSet<String>, AppError> {
    let mut asset_ids = collect_document_asset_ids(document)?;
    for scene_id in director_scene_asset_ids(document) {
        let snapshot = snapshots.get(&scene_id).ok_or_else(|| {
            AppError::Conflict(format!(
                "creative project is missing Director sidecar asset {scene_id}"
            ))
        })?;
        if snapshot.row.kind != "text" {
            return Err(AppError::Conflict(format!(
                "creative project Director sidecar {scene_id} is not a text asset"
            )));
        }
        let nested = director_sidecar_asset_ids(&snapshot.bytes, &document.project_id).map_err(
            |error| {
                AppError::Conflict(format!(
                    "creative project Director sidecar {scene_id} is invalid: {error}"
                ))
            },
        )?;
        asset_ids.extend(nested);
    }
    Ok(asset_ids)
}

fn extend_archive_asset_ids_from_import(
    document: &CreativeProjectDocument,
    assets: &[CreativeArchiveImportedAsset],
    asset_ids: &mut BTreeSet<String>,
) -> Result<(), AppError> {
    for scene_id in director_scene_asset_ids(document) {
        let sidecar = assets
            .iter()
            .find(|asset| asset.metadata.asset_id == scene_id)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "creative project archive is missing Director sidecar asset {scene_id}"
                ))
            })?;
        if sidecar.metadata.kind != "text" {
            return Err(AppError::BadRequest(format!(
                "creative project archive Director sidecar {scene_id} is not a text asset"
            )));
        }
        let nested = director_sidecar_asset_ids(&sidecar.bytes, &document.project_id).map_err(
            |error| {
                AppError::BadRequest(format!(
                    "creative project archive Director sidecar {scene_id} is invalid: {error}"
                ))
            },
        )?;
        asset_ids.extend(nested);
    }
    Ok(())
}

fn remap_nested_asset_ids(
    value: &mut Value,
    path: &str,
    asset_ids: &BTreeMap<String, String>,
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if key == "assetId" {
                    let old_id = child.as_str().ok_or_else(|| {
                        format!("Director sidecar {child_path} must be an asset id string")
                    })?;
                    let new_id = asset_ids.get(old_id).cloned().ok_or_else(|| {
                        format!(
                            "Director sidecar {child_path} references undeclared asset {old_id:?}"
                        )
                    })?;
                    *child = Value::String(new_id);
                } else {
                    remap_nested_asset_ids(child, &child_path, asset_ids)?;
                }
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter_mut().enumerate() {
                remap_nested_asset_ids(child, &format!("{path}[{index}]"), asset_ids)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn remap_director_sidecar_bytes(
    bytes: &[u8],
    old_project_id: &str,
    new_project_id: &str,
    asset_ids: &BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let mut value = parse_director_sidecar(bytes, old_project_id)?;
    let project = value
        .get_mut("project")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "Director sidecar project must be an object".to_owned())?;
    project.insert(
        "projectId".to_owned(),
        Value::String(new_project_id.to_owned()),
    );
    remap_nested_asset_ids(&mut value, "$", asset_ids)?;
    serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("encode remapped Director sidecar: {error}"))
}

fn remap_archive_director_sidecars(
    document: &CreativeProjectDocument,
    assets: &mut [CreativeArchiveImportedAsset],
    old_project_id: &str,
    new_project_id: &str,
    asset_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    for scene_id in director_scene_asset_ids(document) {
        let sidecar = assets
            .iter_mut()
            .find(|asset| asset.metadata.asset_id == scene_id)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "creative archive is missing Director sidecar asset {scene_id}"
                ))
            })?;
        if sidecar.metadata.kind != "text" {
            return Err(AppError::BadRequest(format!(
                "creative archive Director sidecar {scene_id} is not a text asset"
            )));
        }
        sidecar.bytes = remap_director_sidecar_bytes(
            &sidecar.bytes,
            old_project_id,
            new_project_id,
            asset_ids,
        )
        .map_err(|error| {
            AppError::BadRequest(format!(
                "creative archive Director sidecar {scene_id} cannot be remapped: {error}"
            ))
        })?;
        sidecar.metadata.byte_length = sidecar.bytes.len() as u64;
        sidecar.metadata.sha256 = sha256_bytes(&sidecar.bytes);
    }
    Ok(())
}

fn remap_node_references(
    data: &mut CreativeNodeData,
    asset_ids: &BTreeMap<String, String>,
    node_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    match data {
        CreativeNodeData::Image(data) => {
            remap_optional_asset(&mut data.asset_id, asset_ids)?;
            if let Some(composer) = data.composer.as_mut() {
                for mention in &mut composer.mentions {
                    if let Some(remapped) = node_ids.get(&mention.source_node_id) {
                        mention.source_node_id = remapped.clone();
                    }
                }
            }
            Ok(())
        }
        CreativeNodeData::Panorama(data) => remap_optional_asset(&mut data.asset_id, asset_ids),
        CreativeNodeData::Config(data) => {
            remap_asset_vec(&mut data.input_asset_ids, asset_ids)?;
            remap_asset_vec(&mut data.result_asset_ids, asset_ids)?;
            remap_config_operation(data, asset_ids, node_ids)
        }
        CreativeNodeData::Video(data) => {
            remap_optional_asset(&mut data.asset_id, asset_ids)?;
            remap_optional_asset(&mut data.poster_asset_id, asset_ids)
        }
        CreativeNodeData::Audio(data) => remap_optional_asset(&mut data.asset_id, asset_ids),
        CreativeNodeData::Director(data) => remap_optional_asset(&mut data.scene_id, asset_ids),
        CreativeNodeData::Text(_) | CreativeNodeData::Group(_) => Ok(()),
    }
}

fn remap_optional_asset(
    asset_id: &mut Option<String>,
    asset_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    if let Some(value) = asset_id.as_mut() {
        *value = asset_ids.get(value).cloned().ok_or_else(|| {
            AppError::BadRequest(format!(
                "creative archive references undeclared asset {value:?}"
            ))
        })?;
    }
    Ok(())
}

fn remap_asset_vec(
    values: &mut [String],
    asset_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    for value in values {
        *value = asset_ids.get(value).cloned().ok_or_else(|| {
            AppError::BadRequest(format!(
                "creative archive references undeclared asset {value:?}"
            ))
        })?;
    }
    Ok(())
}

fn is_allowed_archive_path(path: &Path) -> bool {
    if path == Path::new("manifest.json") {
        return true;
    }
    let parts = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "assets" || parts[2] != "content" {
        return false;
    }
    WorkshopAssetId::parse(parts[1].to_string()).is_ok()
}

fn asset_content_path(asset_id: &str) -> String {
    format!("assets/{asset_id}/content")
}

fn describe_asset_set_mismatch(
    referenced: &BTreeSet<String>,
    declared: &BTreeSet<String>,
) -> String {
    if let Some(missing) = referenced.difference(declared).next() {
        return format!("creative project archive is missing referenced asset {missing:?}");
    }
    if let Some(extra) = declared.difference(referenced).next() {
        return format!("creative project archive contains unreferenced asset {extra:?}");
    }
    "creative project archive asset set does not match the document".into()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn internal_zip_error(error: zip::result::ZipError) -> AppError {
    AppError::Internal(format!("write creative project archive: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creative_studio::{
        CreativeConfigOperation, CreativeConnection, CreativeNode, CreativeNodeType,
    };

    const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000701";
    const ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000702";
    const DIRECTOR_SCENE_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000705";
    const DIRECTOR_PANORAMA_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000707";
    const DIRECTOR_CHARACTER_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000708";
    const DIRECTOR_OBJECT_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000709";
    const DIRECTOR_CAPTURE_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000710";
    const MASK_REFERENCE_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000711";
    const CONFIG_RESULT_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000712";
    const AUDIO_SOURCE_ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000714";

    fn image_document() -> CreativeProjectDocument {
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.into());
        let image: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "source-node",
            "type": "image",
            "position": { "x": 0, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": {
                "assetId": ASSET_ID,
                "caption": "",
                "alt": "asset",
                "fit": "contain",
                "naturalSize": null,
                "composer": {
                    "prompt": "@备注",
                    "mentions": [{
                        "id": "mention-archive",
                        "sourceNodeId": "target-node",
                        "fallbackLabel": "备注",
                        "start": 0,
                        "end": 3
                    }],
                    "model": null,
                    "interfaceMode": "images",
                    "quality": "auto",
                    "width": 1024,
                    "height": 1024,
                    "aspectRatio": "1:1",
                    "count": 1
                }
            }
        }))
        .unwrap();
        let text: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "target-node",
            "type": "text",
            "position": { "x": 400, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 2,
            "locked": false,
            "data": {
                "text": "note",
                "format": "plain",
                "fontSize": 16,
                "textAlign": "left"
            }
        }))
        .unwrap();
        document.nodes = vec![image, text];
        document.connections = vec![CreativeConnection {
            id: "connection-a".into(),
            source_node_id: "source-node".into(),
            target_node_id: "target-node".into(),
            source_handle: None,
            target_handle: None,
        }];
        document
    }

    fn config_reference_document() -> CreativeProjectDocument {
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.into());
        let source: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "config-source-node",
            "type": "image",
            "position": { "x": 0, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": {
                "assetId": null,
                "caption": "",
                "alt": "",
                "fit": "contain",
                "naturalSize": null
            }
        }))
        .unwrap();
        let compose: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "compose-config",
            "type": "config",
            "position": { "x": 400, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 2,
            "locked": false,
            "data": {
                "task": "image_edit",
                "capability": "i2i",
                "providerId": null,
                "model": null,
                "prompt": "edit",
                "negativePrompt": "",
                "parameters": {
                    "canvasOperation": "image-node-compose",
                    "sourceNodeId": "config-source-node",
                    "sourceAssetId": ASSET_ID
                },
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [CONFIG_RESULT_ASSET_ID],
                "status": "succeeded",
                "errorMessage": null
            }
        }))
        .unwrap();
        let mask: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "mask-config",
            "type": "config",
            "position": { "x": 800, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 3,
            "locked": false,
            "data": {
                "task": "image_edit",
                "capability": "i2i",
                "providerId": null,
                "model": null,
                "prompt": "mask edit",
                "negativePrompt": "",
                "parameters": {
                    "canvasOperation": "image-mask-edit",
                    "sourceNodeId": "config-source-node",
                    "sourceAssetId": ASSET_ID,
                    "markedReferenceAssetId": MASK_REFERENCE_ASSET_ID
                },
                "inputAssetIds": [MASK_REFERENCE_ASSET_ID],
                "taskId": null,
                "resultAssetIds": [],
                "status": "succeeded",
                "errorMessage": null
            }
        }))
        .unwrap();
        let t2i: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "t2i-config",
            "type": "config",
            "position": { "x": 1200, "y": 0 },
            "size": { "width": 320, "height": 180 },
            "groupId": null,
            "zIndex": 4,
            "locked": false,
            "data": {
                "task": "image_generation",
                "capability": "t2i",
                "providerId": null,
                "model": null,
                "prompt": "generate",
                "negativePrompt": "",
                "parameters": {
                    "canvasOperation": "image-node-compose",
                    "sourceNodeId": "config-source-node",
                    "sourceAssetId": null
                },
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [],
                "status": "idle",
                "errorMessage": null
            }
        }))
        .unwrap();
        let video_source: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "video-source-node",
            "type": "video",
            "position": { "x": 0, "y": 300 },
            "size": { "width": 420, "height": 236 },
            "groupId": null,
            "zIndex": 5,
            "locked": false,
            "data": {
                "assetId": null,
                "posterAssetId": null,
                "autoplay": false,
                "loop": false,
                "muted": true,
                "trimStartMs": 0,
                "trimEndMs": null,
                "composer": null
            }
        }))
        .unwrap();
        let video_config: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "video-config",
            "type": "config",
            "position": { "x": 500, "y": 300 },
            "size": { "width": 440, "height": 240 },
            "groupId": null,
            "zIndex": 6,
            "locked": false,
            "data": {
                "task": "video_generation",
                "capability": "t2v",
                "providerId": null,
                "model": null,
                "prompt": "video",
                "negativePrompt": "",
                "operation": {
                    "kind": "video-node-compose",
                    "sourceNodeId": "video-source-node",
                    "sourceAssetId": null
                },
                "parameters": {
                    "prompt": "video",
                    "seconds": 5,
                    "width": 1920,
                    "height": 1080
                },
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [],
                "status": "idle",
                "errorMessage": null
            }
        }))
        .unwrap();
        let audio_source: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "audio-source-node",
            "type": "audio",
            "position": { "x": 0, "y": 600 },
            "size": { "width": 340, "height": 160 },
            "groupId": null,
            "zIndex": 7,
            "locked": false,
            "data": {
                "assetId": AUDIO_SOURCE_ASSET_ID,
                "title": "Voice reference",
                "loop": false,
                "volume": 1,
                "trimStartMs": 0,
                "trimEndMs": null,
                "composer": null
            }
        }))
        .unwrap();
        let audio_config: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "audio-config",
            "type": "config",
            "position": { "x": 500, "y": 600 },
            "size": { "width": 440, "height": 240 },
            "groupId": null,
            "zIndex": 8,
            "locked": false,
            "data": {
                "task": "speech_synthesis",
                "capability": "tts",
                "providerId": null,
                "model": null,
                "prompt": "literal narration",
                "negativePrompt": "",
                "operation": {
                    "kind": "audio-node-compose",
                    "sourceNodeId": "audio-source-node",
                    "sourceAssetId": AUDIO_SOURCE_ASSET_ID
                },
                "parameters": {
                    "prompt": "literal narration",
                    "voice": "alloy",
                    "format": "mp3"
                },
                "inputAssetIds": [],
                "taskId": null,
                "resultAssetIds": [],
                "status": "idle",
                "errorMessage": null
            }
        }))
        .unwrap();
        document.nodes = vec![
            source,
            compose,
            mask,
            t2i,
            video_source,
            video_config,
            audio_source,
            audio_config,
        ];
        document
    }

    fn asset_snapshot() -> CreativeArchiveAssetSnapshot {
        let bytes = b"not-a-decoded-image-but-real-bytes".to_vec();
        CreativeArchiveAssetSnapshot {
            row: WorkshopAssetRow {
                id: 1,
                asset_id: ASSET_ID.into(),
                kind: "image".into(),
                title: "参考图".into(),
                collection: Some("导入".into()),
                tags: r#"["reference"]"#.into(),
                rel_path: Some(format!("workshop/assets/{ASSET_ID}.png")),
                thumb_rel_path: None,
                mime: Some("image/png".into()),
                width: None,
                height: None,
                bytes: Some(bytes.len() as i64),
                text_content: None,
                in_library: true,
                origin: Some(r#"{"prompt":"reference","provider_id":"0190f5fe-7c00-7a00-8abc-000000000703"}"#.into()),
                created_at: 10,
                updated_at: 20,
            },
            bytes,
        }
    }

    fn opaque_image_asset_snapshot(
        asset_id: &str,
        title: &str,
        in_library: bool,
    ) -> CreativeArchiveAssetSnapshot {
        let bytes = format!("opaque-image-{asset_id}").into_bytes();
        CreativeArchiveAssetSnapshot {
            row: WorkshopAssetRow {
                id: 0,
                asset_id: asset_id.into(),
                kind: "image".into(),
                title: title.into(),
                collection: None,
                tags: "[]".into(),
                rel_path: Some(format!("workshop/assets/{asset_id}.png")),
                thumb_rel_path: None,
                mime: Some("image/png".into()),
                width: None,
                height: None,
                bytes: Some(bytes.len() as i64),
                text_content: None,
                in_library,
                origin: None,
                created_at: 10,
                updated_at: 20,
            },
            bytes,
        }
    }

    fn opaque_audio_asset_snapshot(
        asset_id: &str,
        title: &str,
        in_library: bool,
    ) -> CreativeArchiveAssetSnapshot {
        let bytes = format!("opaque-audio-{asset_id}").into_bytes();
        CreativeArchiveAssetSnapshot {
            row: WorkshopAssetRow {
                id: 0,
                asset_id: asset_id.into(),
                kind: "audio".into(),
                title: title.into(),
                collection: None,
                tags: "[]".into(),
                rel_path: Some(format!("workshop/assets/{asset_id}.mp3")),
                thumb_rel_path: None,
                mime: Some("audio/mpeg".into()),
                width: None,
                height: None,
                bytes: Some(bytes.len() as i64),
                text_content: None,
                in_library,
                origin: None,
                created_at: 10,
                updated_at: 20,
            },
            bytes,
        }
    }

    fn director_document() -> CreativeProjectDocument {
        let mut document = CreativeProjectDocument::empty(PROJECT_ID.into());
        let director: CreativeNode = serde_json::from_value(serde_json::json!({
            "id": "director-node",
            "type": "director",
            "position": { "x": 0, "y": 0 },
            "size": { "width": 640, "height": 360 },
            "groupId": null,
            "zIndex": 1,
            "locked": false,
            "data": {
                "sceneId": DIRECTOR_SCENE_ASSET_ID,
                "cameraId": null,
                "timelineMs": 0,
                "durationMs": 5000
            }
        }))
        .unwrap();
        document.nodes = vec![director];
        document
    }

    fn director_scene_text() -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "kind": "nomifun.director.project",
            "version": 1,
            "project": {
                "projectId": PROJECT_ID,
                "name": "3D 导演项目",
                "scene": {
                    "name": "主场景",
                    "transform": {
                        "position": { "x": 0, "y": 0, "z": 0 },
                        "rotation": { "x": 0, "y": 0, "z": 0 },
                        "scale": { "x": 1, "y": 1, "z": 1 }
                    },
                    "environment": {
                        "skyColor": "#101820",
                        "panorama": { "assetId": DIRECTOR_PANORAMA_ASSET_ID },
                        "panoramaYawDegrees": 0,
                        "panoramaRadius": 50,
                        "groundVisible": true,
                        "gridVisible": true,
                        "snapToGrid": false,
                        "characterLabelsVisible": true
                    }
                },
                "cameras": [{
                    "kind": "camera",
                    "id": "camera-1",
                    "name": "主机位",
                    "transform": {
                        "position": { "x": 0, "y": 2, "z": 8 },
                        "rotation": { "x": 0, "y": 0, "z": 0 },
                        "scale": { "x": 1, "y": 1, "z": 1 }
                    },
                    "visible": true,
                    "locked": false,
                    "projection": "perspective",
                    "focalLengthMm": 50,
                    "orthographicSize": 10,
                    "nearClip": 0.1,
                    "farClip": 1000,
                    "aspectRatio": { "width": 16, "height": 9 },
                    "guides": { "frame": true, "center": true, "thirds": true, "safeArea": false }
                }],
                "characters": [{
                    "kind": "character",
                    "id": "character-1",
                    "name": "角色",
                    "transform": {
                        "position": { "x": -1, "y": 0, "z": 0 },
                        "rotation": { "x": 0, "y": 0, "z": 0 },
                        "scale": { "x": 1, "y": 1, "z": 1 }
                    },
                    "visible": true,
                    "locked": false,
                    "asset": { "assetId": DIRECTOR_CHARACTER_ASSET_ID }
                }],
                "objects": [{
                    "kind": "object",
                    "id": "object-1",
                    "name": "道具",
                    "transform": {
                        "position": { "x": 1, "y": 0, "z": 0 },
                        "rotation": { "x": 0, "y": 0, "z": 0 },
                        "scale": { "x": 1, "y": 1, "z": 1 }
                    },
                    "visible": true,
                    "locked": false,
                    "asset": { "assetId": DIRECTOR_OBJECT_ASSET_ID }
                }],
                "lights": [],
                "activeCameraId": "camera-1",
                "selection": null,
                "viewMode": "director",
                "panels": {
                    "leftSidebarOpen": true,
                    "rightSidebarOpen": true,
                    "timelineOpen": true
                },
                "timeline": {
                    "durationSeconds": 5,
                    "currentTimeSeconds": 0,
                    "framesPerSecond": 24,
                    "loop": false,
                    "tracks": []
                },
                "capture": {
                    "settings": {
                        "width": 1920,
                        "height": 1080,
                        "imageFormat": "png",
                        "videoFramesPerSecond": 24
                    },
                    "records": [{
                        "id": "capture-1",
                        "kind": "image",
                        "cameraId": "camera-1",
                        "assetId": DIRECTOR_CAPTURE_ASSET_ID,
                        "capturedAt": 123,
                        "width": 1920,
                        "height": 1080,
                        "format": "png"
                    }]
                }
            }
        }))
        .unwrap()
    }

    fn director_scene_asset_snapshot() -> CreativeArchiveAssetSnapshot {
        let text = director_scene_text();
        CreativeArchiveAssetSnapshot {
            row: WorkshopAssetRow {
                id: 2,
                asset_id: DIRECTOR_SCENE_ASSET_ID.into(),
                kind: "text".into(),
                title: "3D 导演场景".into(),
                collection: None,
                tags: r#"["director-scene"]"#.into(),
                rel_path: None,
                thumb_rel_path: None,
                mime: None,
                width: None,
                height: None,
                bytes: Some(text.len() as i64),
                text_content: Some(text.clone()),
                in_library: false,
                origin: None,
                created_at: 10,
                updated_at: 20,
            },
            bytes: text.into_bytes(),
        }
    }

    #[test]
    fn v1_archive_round_trips_and_remaps_every_owned_identity() {
        let bytes = build_creative_project_archive(
            "归档项目",
            &image_document(),
            vec![asset_snapshot()],
            30,
        )
        .unwrap();
        let parsed = parse_creative_project_archive(&bytes).unwrap();
        assert_eq!(parsed.title, "归档项目");
        assert_eq!(parsed.assets[0].bytes, asset_snapshot().bytes);

        let imported_project = "0190f5fe-7c00-7a00-8abc-000000000704";
        let remapped = remap_creative_archive_for_import(parsed, imported_project).unwrap();
        assert_eq!(remapped.document.project_id, imported_project);
        assert_ne!(remapped.document.nodes[0].id, "source-node");
        assert_ne!(remapped.document.connections[0].id, "connection-a");
        assert!(nomifun_common::validate_uuidv7(&remapped.document.nodes[0].id).is_ok());
        assert!(
            nomifun_common::validate_uuidv7(&remapped.document.connections[0].id).is_ok()
        );
        assert_eq!(
            remapped.document.connections[0].source_node_id,
            remapped.document.nodes[0].id
        );
        let CreativeNodeData::Image(image) = &remapped.document.nodes[0].data else {
            panic!("expected image node")
        };
        assert_eq!(image.asset_id.as_deref(), Some(remapped.assets[0].metadata.asset_id.as_str()));
        assert_eq!(
            image.composer.as_ref().unwrap().mentions[0].source_node_id,
            remapped.document.nodes[1].id
        );
        assert_ne!(remapped.assets[0].metadata.asset_id, ASSET_ID);
        assert!(WorkshopAssetId::parse(&remapped.assets[0].metadata.asset_id).is_ok());
        assert_eq!(remapped.document.nodes[0].node_type, CreativeNodeType::Image);
    }

    #[test]
    fn v2_canvas_writer_uses_canvas_manifest_and_reader_accepts_both_versions() {
        let document = image_document();
        let bytes = build_creative_canvas_archive(
            "归档画布",
            &document,
            vec![asset_snapshot()],
            30,
        )
        .unwrap();
        let files = unzip_to_map(&bytes);
        let manifest: Value = serde_json::from_slice(files.get("manifest.json").unwrap()).unwrap();

        assert_eq!(manifest["kind"], CREATIVE_CANVAS_ARCHIVE_KIND);
        assert_eq!(manifest["version"], CREATIVE_CANVAS_ARCHIVE_VERSION);
        assert_eq!(
            manifest["canvas"]["canvasId"],
            Value::String(PROJECT_ID.into())
        );
        assert_eq!(
            manifest["canvas"]["document"]["canvasId"],
            Value::String(PROJECT_ID.into())
        );
        assert!(manifest.get("project").is_none());
        assert!(!String::from_utf8(files["manifest.json"].clone())
            .unwrap()
            .contains("projectId"));

        let parsed_v2 = parse_creative_archive(&bytes).unwrap();
        assert_eq!(parsed_v2.title, "归档画布");
        assert_eq!(parsed_v2.document.project_id, PROJECT_ID);
        assert_eq!(parsed_v2.assets.len(), 1);

        let v1_bytes = build_creative_project_archive(
            "旧归档项目",
            &document,
            vec![asset_snapshot()],
            30,
        )
        .unwrap();
        let parsed_v1 = parse_creative_archive(&v1_bytes).unwrap();
        assert_eq!(parsed_v1.title, "旧归档项目");
        assert_eq!(parsed_v1.document.project_id, PROJECT_ID);
    }

    #[test]
    fn known_canvas_config_parameters_join_the_archive_closure_and_remap() {
        let bytes = build_creative_project_archive(
            "生成历史项目",
            &config_reference_document(),
            vec![
                opaque_image_asset_snapshot(ASSET_ID, "源图片", false),
                opaque_image_asset_snapshot(MASK_REFERENCE_ASSET_ID, "遮罩参考", false),
                opaque_image_asset_snapshot(CONFIG_RESULT_ASSET_ID, "生成结果", false),
                opaque_audio_asset_snapshot(AUDIO_SOURCE_ASSET_ID, "音频来源", false),
            ],
            30,
        )
        .unwrap();
        let parsed = parse_creative_project_archive(&bytes).unwrap();
        assert_eq!(parsed.assets.len(), 4);

        let imported_project = "0190f5fe-7c00-7a00-8abc-000000000713";
        let remapped = remap_creative_archive_for_import(parsed, imported_project).unwrap();
        let source = &remapped.document.nodes[0];
        assert_ne!(source.id, "config-source-node");
        let asset_by_title = remapped
            .assets
            .iter()
            .map(|asset| (asset.metadata.title.as_str(), asset.metadata.asset_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        let CreativeNodeData::Config(compose) = &remapped.document.nodes[1].data else {
            panic!("expected compose config")
        };
        let Some(CreativeConfigOperation::ImageNodeCompose {
            source_node_id,
            source_asset_id,
        }) = &compose.operation
        else {
            panic!("expected image compose operation")
        };
        assert_eq!(source_node_id, &source.id);
        assert_eq!(source_asset_id.as_deref(), Some(asset_by_title["源图片"]));
        assert_eq!(compose.result_asset_ids, [asset_by_title["生成结果"]]);
        let CreativeNodeData::Config(mask) = &remapped.document.nodes[2].data else {
            panic!("expected mask config")
        };
        let Some(CreativeConfigOperation::ImageMaskEdit {
            source_node_id,
            source_asset_id,
            marked_reference_asset_id,
        }) = &mask.operation
        else {
            panic!("expected image mask operation")
        };
        assert_eq!(source_node_id, &source.id);
        assert_eq!(source_asset_id, asset_by_title["源图片"]);
        assert_eq!(marked_reference_asset_id, asset_by_title["遮罩参考"]);
        assert_eq!(mask.input_asset_ids, [asset_by_title["遮罩参考"]]);
        let CreativeNodeData::Config(t2i) = &remapped.document.nodes[3].data else {
            panic!("expected t2i config")
        };
        let Some(CreativeConfigOperation::ImageNodeCompose {
            source_node_id,
            source_asset_id,
        }) = &t2i.operation
        else {
            panic!("expected t2i operation")
        };
        assert_eq!(source_node_id, &source.id);
        assert_eq!(source_asset_id, &None);
        let video_source = &remapped.document.nodes[4];
        let CreativeNodeData::Config(video_config) = &remapped.document.nodes[5].data else {
            panic!("expected video config")
        };
        let Some(CreativeConfigOperation::VideoNodeCompose {
            source_node_id,
            source_asset_id,
        }) = &video_config.operation
        else {
            panic!("expected video compose operation")
        };
        assert_eq!(source_node_id, &video_source.id);
        assert_eq!(source_asset_id, &None);
        let audio_source = &remapped.document.nodes[6];
        let CreativeNodeData::Audio(audio_source_data) = &audio_source.data else {
            panic!("expected audio source node")
        };
        assert_eq!(
            audio_source_data.asset_id.as_deref(),
            Some(asset_by_title["音频来源"])
        );
        let CreativeNodeData::Config(audio_config) = &remapped.document.nodes[7].data else {
            panic!("expected audio config")
        };
        let Some(CreativeConfigOperation::AudioNodeCompose {
            source_node_id,
            source_asset_id,
        }) = &audio_config.operation
        else {
            panic!("expected audio compose operation")
        };
        assert_eq!(source_node_id, &audio_source.id);
        assert_eq!(
            source_asset_id.as_deref(),
            Some(asset_by_title["音频来源"])
        );
        let remapped_json = serde_json::to_string(&remapped.document).unwrap();
        for old_id in [
            "config-source-node",
            "video-source-node",
            "audio-source-node",
            ASSET_ID,
            MASK_REFERENCE_ASSET_ID,
            CONFIG_RESULT_ASSET_ID,
            AUDIO_SOURCE_ASSET_ID,
        ] {
            assert!(!remapped_json.contains(old_id));
        }

    }

    #[test]
    fn audio_config_operation_rejects_missing_source_node_and_invalid_asset_identity() {
        let mut missing_source = config_reference_document();
        let CreativeNodeData::Config(config) = &mut missing_source.nodes[7].data else {
            panic!("expected audio config")
        };
        let Some(CreativeConfigOperation::AudioNodeCompose { source_node_id, .. }) =
            &mut config.operation
        else {
            panic!("expected audio compose operation")
        };
        *source_node_id = "missing-audio-source".into();
        let error = collect_document_asset_ids(&missing_source).unwrap_err();
        assert!(matches!(
            error,
            AppError::BadRequest(ref message)
                if message.contains("missing source node")
                    && message.contains("missing-audio-source")
        ));

        let mut invalid_asset = config_reference_document();
        let CreativeNodeData::Config(config) = &mut invalid_asset.nodes[7].data else {
            panic!("expected audio config")
        };
        let Some(CreativeConfigOperation::AudioNodeCompose {
            source_asset_id, ..
        }) = &mut config.operation
        else {
            panic!("expected audio compose operation")
        };
        *source_asset_id = Some("not-an-asset-id".into());
        let error = collect_document_asset_ids(&invalid_asset).unwrap_err();
        assert!(matches!(
            error,
            AppError::BadRequest(ref message)
                if message.contains("invalid assetId")
                    && message.contains("not-an-asset-id")
        ));
    }

    #[test]
    fn director_scene_sidecar_round_trips_and_remaps_its_asset_pointer() {
        let bytes = build_creative_project_archive(
            "3D 导演项目",
            &director_document(),
            vec![
                director_scene_asset_snapshot(),
                opaque_image_asset_snapshot(
                    DIRECTOR_PANORAMA_ASSET_ID,
                    "导演全景",
                    false,
                ),
                opaque_image_asset_snapshot(
                    DIRECTOR_CHARACTER_ASSET_ID,
                    "角色资产",
                    false,
                ),
                opaque_image_asset_snapshot(DIRECTOR_OBJECT_ASSET_ID, "道具资产", false),
                opaque_image_asset_snapshot(
                    DIRECTOR_CAPTURE_ASSET_ID,
                    "未发送截图",
                    false,
                ),
            ],
            30,
        )
        .unwrap();
        let parsed = parse_creative_project_archive(&bytes).unwrap();
        assert_eq!(parsed.assets.len(), 5);
        assert!(parsed
            .assets
            .iter()
            .any(|asset| asset.metadata.asset_id == DIRECTOR_SCENE_ASSET_ID));

        let imported_project = "0190f5fe-7c00-7a00-8abc-000000000706";
        let remapped = remap_creative_archive_for_import(parsed, imported_project).unwrap();
        let CreativeNodeData::Director(director) = &remapped.document.nodes[0].data else {
            panic!("expected Director node")
        };
        let sidecar = remapped
            .assets
            .iter()
            .find(|asset| asset.metadata.kind == "text")
            .expect("Director sidecar must remain a text asset");
        assert_eq!(director.scene_id.as_deref(), Some(sidecar.metadata.asset_id.as_str()));
        assert_ne!(director.scene_id.as_deref(), Some(DIRECTOR_SCENE_ASSET_ID));
        assert!(WorkshopAssetId::parse(&sidecar.metadata.asset_id).is_ok());

        let sidecar_value: Value = serde_json::from_slice(&sidecar.bytes).unwrap();
        assert_eq!(
            sidecar_value["project"]["projectId"],
            Value::String(imported_project.into())
        );
        let nested = director_sidecar_asset_ids(&sidecar.bytes, imported_project).unwrap();
        let imported_media = remapped
            .assets
            .iter()
            .filter(|asset| asset.metadata.kind != "text")
            .map(|asset| asset.metadata.asset_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(nested, imported_media);
        for old_id in [
            PROJECT_ID,
            DIRECTOR_PANORAMA_ASSET_ID,
            DIRECTOR_CHARACTER_ASSET_ID,
            DIRECTOR_OBJECT_ASSET_ID,
            DIRECTOR_CAPTURE_ASSET_ID,
        ] {
            assert!(!String::from_utf8(sidecar.bytes.clone()).unwrap().contains(old_id));
        }
        assert_eq!(sidecar.metadata.byte_length, sidecar.bytes.len() as u64);
        assert_eq!(sidecar.metadata.sha256, sha256_bytes(&sidecar.bytes));
    }

    #[test]
    fn director_sidecar_closure_rejects_missing_or_nonportable_dependencies() {
        let missing = build_creative_project_archive(
            "3D 导演项目",
            &director_document(),
            vec![director_scene_asset_snapshot()],
            30,
        )
        .unwrap_err();
        assert!(matches!(
            missing,
            AppError::Conflict(message) if message.contains("missing referenced asset")
        ));

        let mut wrong_project = director_scene_asset_snapshot();
        let mut value: Value = serde_json::from_slice(&wrong_project.bytes).unwrap();
        value["project"]["projectId"] =
            Value::String("0190f5fe-7c00-7a00-8abc-000000000799".into());
        wrong_project.bytes = serde_json::to_vec_pretty(&value).unwrap();
        wrong_project.row.text_content = Some(String::from_utf8(wrong_project.bytes.clone()).unwrap());
        let error = build_creative_project_archive(
            "3D 导演项目",
            &director_document(),
            vec![
                wrong_project,
                opaque_image_asset_snapshot(
                    DIRECTOR_PANORAMA_ASSET_ID,
                    "导演全景",
                    false,
                ),
                opaque_image_asset_snapshot(
                    DIRECTOR_CHARACTER_ASSET_ID,
                    "角色资产",
                    false,
                ),
                opaque_image_asset_snapshot(DIRECTOR_OBJECT_ASSET_ID, "道具资产", false),
                opaque_image_asset_snapshot(
                    DIRECTOR_CAPTURE_ASSET_ID,
                    "未发送截图",
                    false,
                ),
            ],
            30,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::Conflict(message) if message.contains("does not match Creative Studio project")
        ));

        let invalid_asset = director_scene_text().replace(
            DIRECTOR_CAPTURE_ASSET_ID,
            "https://example.invalid/capture.png",
        );
        assert!(director_sidecar_asset_ids(invalid_asset.as_bytes(), PROJECT_ID)
            .unwrap_err()
            .contains("assetId"));
    }

    #[test]
    fn parser_rejects_zip_slip_duplicate_and_undeclared_entries() {
        let malicious = zip_with(&[("../manifest.json", b"{}"), ("manifest.json", b"{}")]);
        assert!(matches!(
            parse_creative_project_archive(&malicious),
            Err(AppError::BadRequest(message)) if message.contains("unsafe")
        ));

        let duplicate = zip_with(&[("manifest.json", b"{}"), ("./manifest.json", b"{}")]);
        assert!(matches!(
            parse_creative_project_archive(&duplicate),
            Err(AppError::BadRequest(message)) if message.contains("duplicate")
        ));

        let extra = zip_with(&[("manifest.json", b"{}"), ("extra.json", b"{}")]);
        assert!(matches!(
            parse_creative_project_archive(&extra),
            Err(AppError::BadRequest(message)) if message.contains("unsupported")
        ));
    }

    #[test]
    fn parser_enforces_entry_count_and_actual_uncompressed_budget() {
        let asset_path = format!("assets/{ASSET_ID}/content");
        let too_many = zip_owned(BTreeMap::from([
            ("manifest.json".to_owned(), b"{}".to_vec()),
            (asset_path, b"x".to_vec()),
        ]));
        assert!(matches!(
            parse_creative_project_archive_with_limits(&too_many, 1024, 1),
            Err(AppError::BadRequest(message)) if message.contains("too many entries")
        ));

        let bytes = build_creative_project_archive(
            "归档项目",
            &image_document(),
            vec![asset_snapshot()],
            30,
        )
        .unwrap();
        assert!(matches!(
            parse_creative_project_archive_with_limits(&bytes, 16, MAX_CREATIVE_ARCHIVE_ENTRIES),
            Err(AppError::BadRequest(message)) if message.contains("decompression bomb")
        ));
    }

    #[test]
    fn parser_rejects_checksum_and_schema_drift() {
        let bytes = build_creative_project_archive(
            "归档项目",
            &image_document(),
            vec![asset_snapshot()],
            30,
        )
        .unwrap();
        let mut files = unzip_to_map(&bytes);
        let manifest = files.get_mut("manifest.json").unwrap();
        let mut value: Value = serde_json::from_slice(manifest).unwrap();
        value["assets"][0]["sha256"] = Value::String("0".repeat(64));
        *manifest = serde_json::to_vec(&value).unwrap();
        let corrupted = zip_owned(files);
        assert!(matches!(
            parse_creative_project_archive(&corrupted),
            Err(AppError::BadRequest(message)) if message.contains("SHA-256")
        ));

        let mut files = unzip_to_map(&bytes);
        let manifest = files.get_mut("manifest.json").unwrap();
        let mut value: Value = serde_json::from_slice(manifest).unwrap();
        value["schema"] = Value::String("nomifun.creative-studio/v2".into());
        *manifest = serde_json::to_vec(&value).unwrap();
        let wrong_schema = zip_owned(files);
        assert!(matches!(
            parse_creative_project_archive(&wrong_schema),
            Err(AppError::BadRequest(message)) if message.contains("schema")
        ));
    }

    #[test]
    fn manifest_asset_set_must_exactly_match_document_references() {
        let err = build_creative_project_archive("归档项目", &image_document(), Vec::new(), 30)
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(message) if message.contains("missing referenced asset")));
    }

    #[test]
    fn imported_origin_drops_nonportable_references() {
        let origin = serde_json::json!({
            "prompt": "cat",
            "provider_id": "0190f5fe-7c00-7a00-8abc-000000000703",
            "project_id": "0190f5fe-7c00-7a00-8abc-000000000704",
            "template_id": "0190f5fe-7c00-7a00-8abc-000000000706",
            "template_run_id": "0190f5fe-7c00-7a00-8abc-000000000707",
            "template_step_id": "0190f5fe-7c00-7a00-8abc-000000000708",
            "projectId": "0190f5fe-7c00-7a00-8abc-000000000709",
            "templateId": "0190f5fe-7c00-7a00-8abc-000000000710",
            "templateRunId": "0190f5fe-7c00-7a00-8abc-000000000711",
            "templateStepId": "0190f5fe-7c00-7a00-8abc-000000000712",
            "node_id": "0190f5fe-7c00-7a00-8abc-000000000705"
        });
        let value: Value = serde_json::from_str(
            sanitized_archive_origin(Some(origin)).unwrap().as_deref().unwrap(),
        )
        .unwrap();
        assert_eq!(value, serde_json::json!({ "prompt": "cat" }));
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn unzip_to_map(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut files = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut content = Vec::new();
            entry.read_to_end(&mut content).unwrap();
            files.insert(entry.name().to_owned(), content);
        }
        files
    }

    fn zip_owned(files: BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            writer.start_file(name, options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }
}
