//! Decoupled custom-figure **library**: figures live independently of any companion,
//! so a user can create/import a figure up-front (from the 电子伙伴 home page)
//! before a companion exists, reuse one figure across several companions, and pick a saved
//! figure when creating/editing a companion.
//!
//! Storage (shared, under the backend data dir):
//!   `{figures_dir}/{figure_id}.webp`  — the processed cutout image bytes
//!   `{figures_dir}/index.json`        — `{ figures: [FigureMeta, …] }`
//!
//! Ingest reuses [`crate::figure::validate_figure_source`] (same sandbox +
//! magic + size + dimension checks as the per-companion path). Index read-modify-write
//! is serialized by the caller ([`crate::service::CompanionService`] holds the lock),
//! so these functions stay pure over `figures_dir`.

use std::path::Path;

use nomifun_common::{AppError, FigureId, now_ms};
use serde::{Deserialize, Serialize};

use crate::profile::HeadBox;

const INDEX_FILE: &str = "index.json";
/// Cap on a figure's display name (chars). Generous; just stops abuse.
const MAX_NAME_CHARS: usize = 40;

/// One library figure. Mirrors `FigureMeta` in the UI (`characters/types.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FigureMeta {
    /// Stable id `figure_…` (cross-device, per the primary-key terminal state).
    pub id: String,
    /// User-facing label.
    pub name: String,
    /// width / height of the cutout image.
    pub aspect: f32,
    pub head_box: HeadBox,
    /// Desk size tier: "s" | "m" | "l".
    pub size_tier: String,
    /// Creation time, unix milliseconds.
    pub created_at: i64,
}

/// Editable library-figure metadata. Image bytes, id, aspect and created_at stay immutable.
#[derive(Debug, Clone, Default)]
pub struct FigureUpdate {
    pub name: Option<String>,
    pub head_box: Option<HeadBox>,
    pub size_tier: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct FigureIndex {
    #[serde(default)]
    figures: Vec<FigureMeta>,
}

fn image_name(id: &str) -> String {
    format!("{id}.webp")
}

/// Reject ids that could escape `figures_dir` (path separators / traversal) or
/// don't look like our minted ids. `read`/`delete` take the id from a URL path
/// param, so this is the trust boundary.
fn is_safe_id(id: &str) -> bool {
    FigureId::parse(id).is_ok()
}

fn sanitize_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let name: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    if name.is_empty() { "自定义形象".to_owned() } else { name }
}

fn normalize_tier(tier: &str) -> String {
    match tier {
        "s" | "l" => tier.to_owned(),
        _ => "m".to_owned(),
    }
}

fn load_index(figures_dir: &Path) -> FigureIndex {
    let mut index: FigureIndex =
        crate::fsio::load_json_or_default(&figures_dir.join(INDEX_FILE));
    let original_len = index.figures.len();
    index
        .figures
        .retain(|figure| FigureId::parse(&figure.id).is_ok());
    if index.figures.len() != original_len {
        tracing::warn!(
            figures_dir = %figures_dir.display(),
            rejected = original_len - index.figures.len(),
            "figure index contained noncanonical durable ids; rejected invalid entries"
        );
    }
    index
}

fn save_index(figures_dir: &Path, index: &FigureIndex) -> Result<(), AppError> {
    crate::fsio::save_json_atomic(figures_dir, INDEX_FILE, index)
        .map_err(|e| AppError::Internal(format!("save figure index: {e}")))
}

/// All saved figures, newest first.
pub fn list(figures_dir: &Path) -> Vec<FigureMeta> {
    let mut figures = load_index(figures_dir).figures;
    figures.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    figures
}

/// Ingest a validated upload as a new library figure; returns its metadata.
pub fn create(
    figures_dir: &Path,
    source_path: &Path,
    name: &str,
    aspect: f32,
    head_box: HeadBox,
    size_tier: &str,
) -> Result<FigureMeta, AppError> {
    let bytes = crate::figure::validate_figure_source(source_path)?;
    let id = FigureId::new().into_string();
    crate::fsio::save_bytes_atomic(figures_dir, &image_name(&id), &bytes)
        .map_err(|e| AppError::Internal(format!("save library figure: {e}")))?;

    let meta = FigureMeta {
        id: id.clone(),
        name: sanitize_name(name),
        aspect,
        head_box,
        size_tier: normalize_tier(size_tier),
        created_at: now_ms(),
    };
    let mut index = load_index(figures_dir);
    index.figures.push(meta.clone());
    save_index(figures_dir, &index)?;
    Ok(meta)
}

/// One figure's image bytes + mtime (unix seconds, the ETag input). `None` for
/// an unknown/invalid id or a missing image file.
pub fn read_image(figures_dir: &Path, id: &str) -> Option<(Vec<u8>, u64)> {
    if !is_safe_id(id) {
        return None;
    }
    let path = figures_dir.join(image_name(id));
    let mtime = std::fs::metadata(&path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let bytes = std::fs::read(&path).ok()?;
    Some((bytes, mtime))
}

/// Rename a figure. Unknown id → 404.
pub fn rename(figures_dir: &Path, id: &str, name: &str) -> Result<FigureMeta, AppError> {
    update(figures_dir, id, FigureUpdate { name: Some(name.to_owned()), head_box: None, size_tier: None })
}

/// Update editable figure metadata. Unknown id → 404.
pub fn update(figures_dir: &Path, id: &str, patch: FigureUpdate) -> Result<FigureMeta, AppError> {
    if !is_safe_id(id) {
        return Err(AppError::NotFound(format!("figure '{id}' not found")));
    }
    let mut index = load_index(figures_dir);
    let entry = index
        .figures
        .iter_mut()
        .find(|f| f.id == id)
        .ok_or_else(|| AppError::NotFound(format!("figure '{id}' not found")))?;
    if let Some(name) = patch.name {
        entry.name = sanitize_name(&name);
    }
    if let Some(head_box) = patch.head_box {
        entry.head_box = head_box;
    }
    if let Some(size_tier) = patch.size_tier {
        entry.size_tier = normalize_tier(&size_tier);
    }
    let updated = entry.clone();
    save_index(figures_dir, &index)?;
    Ok(updated)
}

/// Delete a figure (image + index entry). Idempotent: a missing image still
/// drops the index entry. Unknown id → 404.
pub fn remove(figures_dir: &Path, id: &str) -> Result<(), AppError> {
    if !is_safe_id(id) {
        return Err(AppError::NotFound(format!("figure '{id}' not found")));
    }
    let mut index = load_index(figures_dir);
    let before = index.figures.len();
    index.figures.retain(|f| f.id != id);
    if index.figures.len() == before {
        return Err(AppError::NotFound(format!("figure '{id}' not found")));
    }
    save_index(figures_dir, &index)?;
    // Best-effort image removal — the index no longer references it either way.
    let _ = std::fs::remove_file(figures_dir.join(image_name(id)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload_scratch() -> tempfile::TempDir {
        let root = std::env::temp_dir().join("nomifun");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new().prefix("figlib-test-").tempdir_in(root).unwrap()
    }

    /// A real 7×5 lossless WebP (VP8L), same bytes the figure.rs tests use.
    fn webp_bytes() -> Vec<u8> {
        vec![
            0x52, 0x49, 0x46, 0x46, 0x1E, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x4C, 0x11, 0x00, 0x00, 0x00, 0x2F, 0x06, 0x00, 0x01, 0x00, 0x07, 0x50, 0x8A,
            0x2A, 0xD4, 0xA3, 0xFF, 0x81, 0x88, 0xE8, 0x7F, 0x00, 0x00,
        ]
    }

    fn make_source(upload: &tempfile::TempDir, file: &str) -> std::path::PathBuf {
        let p = upload.path().join(file);
        std::fs::write(&p, webp_bytes()).unwrap();
        p
    }

    #[test]
    fn create_list_read_rename_delete_roundtrip() {
        let upload = upload_scratch();
        let figs = tempfile::tempdir().unwrap();
        let dir = figs.path();

        let hb = HeadBox { x: 0.3, y: 0.0, w: 0.4, h: 0.4 };
        let a = create(dir, &make_source(&upload, "a.webp"), "阿狸", 0.7, hb.clone(), "l").unwrap();
        let b = create(dir, &make_source(&upload, "b.webp"), "", 1.0, hb.clone(), "bogus").unwrap();

        assert!(FigureId::parse(&a.id).is_ok());
        assert_eq!(a.name, "阿狸");
        assert_eq!(a.size_tier, "l");
        assert_eq!(b.name, "自定义形象"); // empty → default
        assert_eq!(b.size_tier, "m"); // bogus tier → m

        // newest first
        let listed = list(dir);
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|f| f.id == a.id));
        assert!(listed.iter().any(|f| f.id == b.id));
        if b.created_at > a.created_at {
            assert_eq!(listed[0].id, b.id);
        }

        // image readable
        let (bytes, _) = read_image(dir, &a.id).unwrap();
        assert_eq!(bytes, webp_bytes());

        // rename
        let renamed = rename(dir, &a.id, "新名字").unwrap();
        assert_eq!(renamed.name, "新名字");
        assert_eq!(list(dir).iter().find(|f| f.id == a.id).unwrap().name, "新名字");

        // update editable framing metadata without touching immutable image/aspect.
        let updated_head = HeadBox { x: 0.1, y: 0.2, w: 0.5, h: 0.6 };
        let updated = update(
            dir,
            &a.id,
            FigureUpdate { name: Some("新取景".to_owned()), head_box: Some(updated_head.clone()), size_tier: Some("s".to_owned()) },
        )
        .unwrap();
        assert_eq!(updated.name, "新取景");
        assert_eq!(updated.aspect, a.aspect);
        assert_eq!(updated.created_at, a.created_at);
        assert_eq!(updated.head_box, updated_head);
        assert_eq!(updated.size_tier, "s");
        assert_eq!(list(dir).iter().find(|f| f.id == a.id).unwrap().head_box, updated_head);

        // delete drops index + image
        remove(dir, &a.id).unwrap();
        assert_eq!(list(dir).len(), 1);
        assert!(read_image(dir, &a.id).is_none());
        assert!(remove(dir, &a.id).is_err()); // already gone → 404
    }

    #[test]
    fn rejects_unsafe_ids() {
        let figs = tempfile::tempdir().unwrap();
        assert!(read_image(figs.path(), "../escape").is_none());
        assert!(read_image(figs.path(), "figure_../x").is_none());
        assert!(read_image(figs.path(), "notaprefix").is_none());
        assert!(
            read_image(figs.path(), "figure_550e8400-e29b-41d4-a716-446655440000").is_none(),
            "parseable non-v7 UUIDs are not canonical figure IDs"
        );
        assert!(rename(figs.path(), "../x", "n").is_err());
        assert!(update(figs.path(), "../x", FigureUpdate { name: Some("n".into()), head_box: None, size_tier: None }).is_err());
        assert!(remove(figs.path(), "figure_a/b").is_err());
    }
}
