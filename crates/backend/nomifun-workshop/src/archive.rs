//! Strict Creative Studio v1 project archives.
//!
//! An archive is a ZIP with exactly one `manifest.json` plus one content entry
//! for every asset referenced by the canonical project document. The archive
//! is intentionally a closed `nomifun.creative-studio/v1` contract: there is no
//! reader for the retired Workshop canvas format and no best-effort upgrade
//! path for older or future archive versions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use nomifun_common::{AppError, WorkshopAssetId, zip_safe};
use nomifun_db::WorkshopAssetRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::creative_studio::{
    CREATIVE_STUDIO_SCHEMA, CreativeGenerationStatus, CreativeNodeData,
    CreativeProjectDocument, MAX_CREATIVE_PROJECT_DOCUMENT_BYTES,
};
use crate::MAX_ASSET_BYTES;

pub const CREATIVE_STUDIO_ARCHIVE_MIME: &str =
    "application/vnd.nomifun.creative-studio+zip";
pub const MAX_CREATIVE_ARCHIVE_COMPRESSED_BYTES: usize = 256 * 1024 * 1024;
const CREATIVE_STUDIO_ARCHIVE_KIND: &str = "project-archive";
const CREATIVE_STUDIO_ARCHIVE_VERSION: u32 = 1;
const MAX_CREATIVE_ARCHIVE_ASSETS: usize = 255;
const MAX_CREATIVE_ARCHIVE_ENTRIES: usize = MAX_CREATIVE_ARCHIVE_ASSETS + 1;
const MAX_CREATIVE_ARCHIVE_MANIFEST_BYTES: usize =
    MAX_CREATIVE_PROJECT_DOCUMENT_BYTES + 8 * 1024 * 1024;
const MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES: u64 =
    zip_safe::ZipExtractionBudget::DEFAULT_MAX_TOTAL_UNCOMPRESSED_BYTES;

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

    let referenced = collect_document_asset_ids(document)?;
    if referenced.len() > MAX_CREATIVE_ARCHIVE_ASSETS {
        return Err(AppError::BadRequest(format!(
            "creative project references too many assets: {} (max {MAX_CREATIVE_ARCHIVE_ASSETS})",
            referenced.len()
        )));
    }

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

pub(crate) fn parse_creative_project_archive(
    bytes: &[u8],
) -> Result<CreativeArchiveImport, AppError> {
    parse_creative_project_archive_with_limits(
        bytes,
        MAX_CREATIVE_ARCHIVE_UNCOMPRESSED_BYTES,
        MAX_CREATIVE_ARCHIVE_ENTRIES,
    )
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
    let referenced = collect_document_asset_ids(&manifest.project.document)?;
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
        remap_node_asset_ids(&mut node.data, &asset_ids)?;
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
        "canvas_id",
        "node_id",
        "creation_task_id",
        "task_id",
        "providerId",
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
            }
            CreativeNodeData::Video(data) => {
                insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?;
                insert_optional_asset(&mut asset_ids, data.poster_asset_id.as_deref())?;
            }
            CreativeNodeData::Audio(data) => {
                insert_optional_asset(&mut asset_ids, data.asset_id.as_deref())?
            }
            CreativeNodeData::Text(_)
            | CreativeNodeData::Director(_)
            | CreativeNodeData::Group(_) => {}
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

fn remap_node_asset_ids(
    data: &mut CreativeNodeData,
    asset_ids: &BTreeMap<String, String>,
) -> Result<(), AppError> {
    match data {
        CreativeNodeData::Image(data) => remap_optional_asset(&mut data.asset_id, asset_ids),
        CreativeNodeData::Panorama(data) => remap_optional_asset(&mut data.asset_id, asset_ids),
        CreativeNodeData::Config(data) => {
            remap_asset_vec(&mut data.input_asset_ids, asset_ids)?;
            remap_asset_vec(&mut data.result_asset_ids, asset_ids)
        }
        CreativeNodeData::Video(data) => {
            remap_optional_asset(&mut data.asset_id, asset_ids)?;
            remap_optional_asset(&mut data.poster_asset_id, asset_ids)
        }
        CreativeNodeData::Audio(data) => remap_optional_asset(&mut data.asset_id, asset_ids),
        CreativeNodeData::Text(_)
        | CreativeNodeData::Director(_)
        | CreativeNodeData::Group(_) => Ok(()),
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
    use crate::creative_studio::{CreativeConnection, CreativeNode, CreativeNodeType};

    const PROJECT_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000701";
    const ASSET_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000702";

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
                "naturalSize": null
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
        assert_ne!(remapped.assets[0].metadata.asset_id, ASSET_ID);
        assert!(WorkshopAssetId::parse(&remapped.assets[0].metadata.asset_id).is_ok());
        assert_eq!(remapped.document.nodes[0].node_type, CreativeNodeType::Image);
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
