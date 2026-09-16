//! Explicit, bounded capture for publication. Runtime consumers must use the
//! resulting immutable artifact, never these mutable library source paths.
use crate::{SkillPaths, constants::BUILTIN_AUTO_SKILLS_SUBDIR};
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySkillSource {
    Custom,
    Builtin,
    BuiltinAuto,
    Cron,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarySkillSelection {
    pub name: String,
    pub source: LibrarySkillSource,
}

pub struct FrozenLibrarySkill {
    pub selection: LibrarySkillSelection,
    pub source_digest: String,
    /// Relative source names and captured bytes. SKILL.md remains the body;
    /// all other ordinary files are included, never executed on publication.
    pub files: BTreeMap<String, Vec<u8>>,
}

fn fail(reason: impl std::fmt::Display) -> AppError {
    AppError::BadRequest(format!("Skill freeze: {reason}"))
}

pub fn capture(
    paths: &SkillPaths,
    selection: LibrarySkillSelection,
) -> Result<FrozenLibrarySkill, AppError> {
    // A logical library entry, never an arbitrary client-selected path.
    let name = &selection.name;
    if name.is_empty()
        || name.len() > 128
        || name.trim() != name
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.ends_with(['.', ' '])
        || name.chars().any(char::is_control)
        || name == "."
        || name.contains("..")
    {
        return Err(fail("invalid library Skill name"));
    }
    let parent = match selection.source {
        LibrarySkillSource::Custom => paths.user_skills_dir.clone(),
        LibrarySkillSource::Builtin => paths.builtin_skills_dir.clone(),
        LibrarySkillSource::BuiltinAuto => {
            paths.builtin_skills_dir.join(BUILTIN_AUTO_SKILLS_SUBDIR)
        }
        LibrarySkillSource::Cron => paths.cron_skills_dir.clone(),
    };
    let root = parent.join(name);
    // Do not silently dereference imported symlinks/junctions. The user must
    // explicitly copy such a Skill into the library before freezing it.
    plain(&parent, true)?;
    plain(&root, true)?;
    let parent = fs::canonicalize(parent).map_err(fail)?;
    let canonical = fs::canonicalize(&root).map_err(fail)?;
    if canonical.parent() != Some(parent.as_path()) {
        return Err(fail("Skill root escapes its selected library"));
    }
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
                16 * 1024
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
    let mut hash = Sha256::new();
    hash.update(b"nomifun-library-skill-v1\0");
    hash.update(serde_json::to_vec(&selection).map_err(fail)?);
    for (path, bytes) in &files {
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    Ok(FrozenLibrarySkill {
        selection,
        source_digest: format!("{:x}", hash.finalize()),
        files,
    })
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
