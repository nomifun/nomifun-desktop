//! Bounded library capture for the canonical Revision/Snapshot. Runtime
//! consumers read its immutable content, never mutable library source paths.
use crate::{SkillPaths, constants::BUILTIN_AUTO_SKILLS_SUBDIR};
use nomifun_common::AppError;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nomifun_agent_contracts::{FrozenLibrarySkill, FrozenSkillContent, FrozenSkillResource, LibrarySkillSource};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableLibrarySkill {
    pub name: String,
    pub reason: String,
}

#[derive(Default)]
pub struct SkillInventory {
    pub skills: Vec<FrozenLibrarySkill>,
    pub unavailable: Vec<UnavailableLibrarySkill>,
}

/// Discover each package independently. A broken unselected package remains a
/// management item with diagnostics; it cannot poison unrelated Sessions and
/// never receives a mutable-path runtime fallback.
pub async fn capture_inventory(paths: &SkillPaths) -> Result<SkillInventory, AppError> {
    capture_listed_inventory(paths, crate::list_available_skills(paths).await?).await
}

/// Inspection can decode bounded pixels and read an entire local package. Run
/// that pure work off the async request executor, using the same listed rows.
pub(crate) async fn capture_listed_inventory(paths: &SkillPaths, items: Vec<crate::SkillListItem>) -> Result<SkillInventory, AppError> {
    let paths = paths.clone();
    tokio::task::spawn_blocking(move || capture_inventory_items(&paths, items)).await
        .map_err(|_| fail("library inspection task failed"))?
}

fn capture_inventory_items(paths: &SkillPaths, mut items: Vec<crate::SkillListItem>) -> Result<SkillInventory, AppError> {
    items.sort_by(|left, right| left.name.cmp(&right.name).then_with(|| left.location.cmp(&right.location)));
    let mut counts = BTreeMap::new();
    for item in &items { *counts.entry(item.name.clone()).or_insert(0usize) += 1; }
    let mut inventory = SkillInventory::default();
    let mut diagnosed = BTreeSet::new();
    for item in items {
        if counts[&item.name] != 1 {
            if diagnosed.insert(item.name.clone()) { inventory.unavailable.push(UnavailableLibrarySkill {
                name:item.name, reason:"multiple library folders use the same Skill name".into() }); }
            continue;
        }
        match capture_item(paths, &item).and_then(|skill| {
            nomifun_agent_contracts::validate_library_skill_inventory(inventory.skills.iter().chain(std::iter::once(&skill)))
                .map_err(|error| fail(error.message))?;
            Ok(skill)
        }) {
            Ok(skill) => inventory.skills.push(skill),
            Err(error) => inventory.unavailable.push(UnavailableLibrarySkill { name:item.name, reason:error.to_string() }),
        }
    }
    Ok(inventory)
}

/// Strict explicit selection: unsupported selected content is an error, not an
/// empty selection or a request to reread its source during runtime execution.
pub async fn capture_selected(paths: &SkillPaths, names: &[String]) -> Result<Vec<FrozenLibrarySkill>, AppError> {
    if names.len() > 128 { return Err(fail("at most 128 library Skills can be selected")); }
    let available = crate::list_available_skills(paths).await?;
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    for name in names {
        if !seen.insert(name) { return Err(fail("duplicate selected Skill")); }
        let mut matching = available.iter().filter(|item| &item.name == name);
        let item = matching.next().ok_or_else(|| fail(format!("Skill {name} is not available in the current library")))?;
        if matching.next().is_some() { return Err(fail(format!("Skill {name} has ambiguous library sources"))); }
        selected.push(capture_item(paths, item)?);
    }
    nomifun_agent_contracts::validate_library_skill_inventory(&selected).map_err(|error| fail(error.message))?;
    selected.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(selected)
}

fn capture_item(paths: &SkillPaths, item: &crate::SkillListItem) -> Result<FrozenLibrarySkill, AppError> {
    let (source, parent) = match item.source {
        crate::SkillSource::Custom => (LibrarySkillSource::Custom, paths.user_skills_dir.clone()),
        crate::SkillSource::Builtin if item.relative_location.as_deref().is_some_and(|path| path.starts_with("auto-inject/")) =>
            (LibrarySkillSource::BuiltinAuto, paths.builtin_skills_dir.join(BUILTIN_AUTO_SKILLS_SUBDIR)),
        crate::SkillSource::Builtin => (LibrarySkillSource::Builtin, paths.builtin_skills_dir.clone()),
    };
    let location = PathBuf::from(&item.location);
    let root = if location.file_name().is_some_and(|name| name == "SKILL.md") {
        location.parent().ok_or_else(|| fail("Skill has no source directory"))?.to_path_buf()
    } else { location };
    capture_root(&item.name, &item.description, source, &parent, &root)
}

/// Product defaults are the former auto-inject library entries. Selection is
/// still written into the canonical Snapshot; this function grants no authority.
pub async fn default_skill_names(paths: &SkillPaths) -> Result<Vec<String>, AppError> {
    let mut names = crate::list_builtin_auto_skills(paths).await?.into_iter().map(|item| item.name).collect::<Vec<_>>();
    names.sort(); names.dedup(); Ok(names)
}

fn fail(reason: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("Skill freeze: {reason}"))
}

fn capture_root(name: &str, description: &str, source: LibrarySkillSource, parent: &Path, root: &Path)
    -> Result<FrozenLibrarySkill, AppError> {
    if name.is_empty() || name.len() > 128 || name.trim() != name || name.contains(['/', '\\', ':'])
        || name.contains("..") || name.chars().any(char::is_control) { return Err(fail("invalid library Skill name")); }
    plain(parent, true)?;
    // Directory imports explicitly register a link in this library. Follow
    // that one selected root, while every child is still required to be plain.
    let root_meta = fs::symlink_metadata(root).map_err(fail)?;
    if (!root_meta.is_dir() && !is_link(&root_meta)) || root.parent() != Some(parent) {
        return Err(fail("Skill root is not an entry of its selected library"));
    }
    let canonical = fs::canonicalize(root).map_err(fail)?;
    plain(&canonical, true)?;
    let mut files = BTreeMap::new();
    let mut collisions = BTreeSet::new();
    let mut entries = 0usize;
    let mut total = 0usize;
    let mut text_total = 0usize;
    let mut image_count = 0usize;
    let mut stack = vec![(canonical.clone(), String::new(), 0usize)];
    while let Some((directory, prefix, depth)) = stack.pop() {
        plain(&directory, true)?;
        if !fs::canonicalize(&directory)
            .map_err(fail)?
            .starts_with(&canonical)
        {
            return Err(fail("directory escaped Skill root"));
        }
        for entry in fs::read_dir(&directory).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            entries += 1;
            if entries > 256 {
                return Err(fail("Skill exceeds 256 directory entries"));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| fail("non-UTF-8 resource path"))?;
            // Repository metadata and interpreter caches are not Skill
            // reference resources. Keep ordinary declared-looking files strict.
            if matches!(name.as_str(), ".git" | ".DS_Store" | "__pycache__") || name.ends_with(".pyc") { continue; }
            if name.is_empty()
                || name.len() > 255
                || name.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
                || name.ends_with(['.', ' '])
                || name.chars().any(char::is_control)
            {
                return Err(fail("unsafe resource path"));
            }
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if relative.len() > 128 || !collisions.insert(relative.to_lowercase()) {
                return Err(fail(
                    "resource paths must fit 128 UTF-8 bytes without case collisions",
                ));
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(fail)?;
            if is_link(&meta) {
                return Err(fail("symlinks and reparse points cannot be frozen"));
            }
            if meta.is_dir() {
                if depth >= 8 {
                    return Err(fail("Skill exceeds 8 resource directory levels"));
                }
                stack.push((path, relative, depth + 1));
                continue;
            }
            if !meta.is_file() {
                return Err(fail("only ordinary files can be frozen"));
            }
            if files.len() >= 65 {
                return Err(fail("Skill exceeds SKILL.md plus 64 resource files"));
            }
            let expected_path = fs::canonicalize(&path).map_err(fail)?;
            if !expected_path.starts_with(&canonical) {
                return Err(fail("resource escapes Skill root"));
            }
            let image = matches!(
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp")
            );
            let limit = if relative == "SKILL.md" {
                128 * 1024
            } else if image {
                4 * 1024 * 1024
            } else {
                256 * 1024
            };
            if meta.len() > limit as u64 {
                return Err(fail(format!("{relative} exceeds its file budget")));
            }
            let file = fs::File::open(&path).map_err(fail)?;
            let opened = file.metadata().map_err(fail)?;
            if !opened.is_file()
                || opened.len() != meta.len()
                || opened.modified().ok() != meta.modified().ok()
            {
                return Err(fail("source changed while opening"));
            }
            let mut bytes = Vec::new();
            file.take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(fail)?;
            let after = plain(&path, false)?;
            if bytes.len() > limit
                || bytes.len() as u64 != opened.len()
                || after.len() != opened.len()
                || after.modified().ok() != opened.modified().ok()
                || fs::canonicalize(&path).map_err(fail)? != expected_path
            {
                return Err(fail("source changed while capturing"));
            }
            if !image && std::str::from_utf8(&bytes).is_err() {
                return Err(fail(format!(
                    "{relative}: only UTF-8 text and PNG/JPEG/WebP resources are supported"
                )));
            }
            if image {
                image_count += 1;
            } else if relative != "SKILL.md" {
                text_total += bytes.len();
            }
            if image_count > 4 || text_total > 512 * 1024 {
                return Err(fail(
                    "Skill exceeds 4 image resources or 512 KiB text resources",
                ));
            }
            total += bytes.len();
            if total > 8 * 1024 * 1024 {
                return Err(fail("Skill exceeds 8 MiB captured bytes"));
            }
            files.insert(relative, bytes);
        }
    }
    let body = files
        .get("SKILL.md")
        .ok_or_else(|| fail("exact SKILL.md is required"))?;
    if std::str::from_utf8(body).map_err(fail)?.trim().is_empty() {
        return Err(fail("SKILL.md is empty"));
    }
    if fs::canonicalize(&root).map_err(fail)? != canonical {
        return Err(fail("Skill root changed during capture"));
    }
    let body = std::str::from_utf8(body).map_err(fail)?.to_owned();
    let mut resources = BTreeMap::new();
    for (path, bytes) in files {
        if path == "SKILL.md" { continue; }
        let image = matches!(Path::new(&path).extension().and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase).as_deref(), Some("png" | "jpg" | "jpeg" | "webp"));
        let content = if image { prepare_image(&path, &bytes)? }
            else { FrozenSkillContent::Text { text: String::from_utf8(bytes).map_err(fail)? } };
        resources.insert(path, FrozenSkillResource::new(content).map_err(fail)?);
    }
    let skill = FrozenLibrarySkill::new(name.to_owned(), description.to_owned(), source, body, resources).map_err(fail)?;
    skill.validate().map_err(|error| fail(error.message))?;
    Ok(skill)
}

fn plain(path: &Path, directory: bool) -> Result<fs::Metadata, AppError> {
    let meta = fs::symlink_metadata(path).map_err(fail)?;
    if is_link(&meta) || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(fail(
            "source must be an ordinary directory/file, not a symlink or reparse point",
        ));
    }
    Ok(meta)
}

fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}

/// Decode and re-encode captured pixels before any runtime can observe them.
fn prepare_image(path: &str, bytes: &[u8]) -> Result<FrozenSkillContent, AppError> {
    use image::{ImageReader, ImageFormat};
    use std::io::Cursor;
    let expected = match Path::new(path).extension().and_then(|value| value.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => ImageFormat::Png, Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        Some("webp") => ImageFormat::WebP, _ => return Err(fail("unsupported image format")),
    };
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(fail)?;
    if reader.format() != Some(expected) { return Err(fail("image bytes differ from their declared extension")); }
    let (width, height) = reader.into_dimensions().map_err(fail)?;
    if width == 0 || height == 0 || width > 16384 || height > 16384 || u64::from(width) * u64::from(height) > 40_000_000 {
        return Err(fail("image dimensions exceed the source envelope"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), expected);
    let mut limits = image::Limits::default(); limits.max_image_width = Some(16384); limits.max_image_height = Some(16384);
    limits.max_alloc = Some(192 * 1024 * 1024); reader.limits(limits);
    let decoded = reader.decode().map_err(fail)?.thumbnail(1568, 1568);
    let mut encoded = Cursor::new(Vec::new()); decoded.write_to(&mut encoded, ImageFormat::Png).map_err(fail)?;
    let mut media_type = "image/png";
    let mut encoded = encoded.into_inner();
    if encoded.len() > 1500 * 1024 {
        encoded.clear(); image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80)
            .encode_image(&decoded.to_rgb8()).map_err(fail)?; media_type = "image/jpeg";
    }
    let data_base64 = STANDARD.encode(encoded);
    if data_base64.len() > 2 * 1024 * 1024 { return Err(fail("prepared image exceeds the runtime envelope")); }
    Ok(FrozenSkillContent::Image { media_type: media_type.into(), data_base64 })
}
