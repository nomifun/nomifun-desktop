//! [`WorkshopService`] — the single handle the `/api/workshop/*` routes talk
//! to. Owns canvas CRUD + opaque-doc read/write, asset store/list/patch/delete,
//! and traversal-safe file serving. Canvas bodies + asset binaries live on disk
//! under the data dir; index rows live in `nomifun-db` via [`IWorkshopRepository`].

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use nomifun_common::{AppError, generate_prefixed_id, now_ms};
use nomifun_db::{IWorkshopRepository, ListAssetsParams, UpdateAssetParams, WorkshopAssetRow};
use serde_json::Value;

use crate::dto::{WorkshopAsset, WorkshopCanvasMeta};
use crate::{DEFAULT_DOC, MAX_ASSET_BYTES, MAX_DOC_BYTES, WORKSHOP_REL_DIR, fsio, imagemeta};

/// A canvas plus its (opaque) doc — the `GET /canvases/{id}` payload.
pub struct CanvasWithDoc {
    pub meta: WorkshopCanvasMeta,
    pub doc: Value,
}

/// A paginated asset listing.
pub struct AssetListPage {
    pub items: Vec<WorkshopAsset>,
    pub total: i64,
}

/// A served asset file (bytes + resolved Content-Type).
pub struct ServedFile {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A multipart asset upload (binary + optional metadata).
pub struct NewAssetUpload {
    pub file_name: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
    pub title: Option<String>,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

/// A `text`-kind asset (no binary; body lives in `text_content`).
pub struct NewTextAsset {
    pub title: String,
    pub text_content: String,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

/// Filters + pagination for [`WorkshopService::list_assets`].
#[derive(Default)]
pub struct AssetQuery {
    pub kind: Option<String>,
    pub collection: Option<String>,
    pub q: Option<String>,
    pub in_library: Option<bool>,
    pub page: i64,
    pub page_size: i64,
}

/// Partial asset update. A present field updates; an absent one keeps. For
/// `collection`, `Some("")` clears it to NULL.
#[derive(Default)]
pub struct AssetPatch {
    pub title: Option<String>,
    pub collection: Option<String>,
    pub tags: Option<Vec<String>>,
    pub in_library: Option<bool>,
}

pub struct WorkshopService {
    repo: Arc<dyn IWorkshopRepository>,
    /// Backend data dir root. Asset `rel_path`s are relative to this.
    data_dir: PathBuf,
}

impl WorkshopService {
    /// Build the service over its index repo + the data dir root.
    pub fn start(data_dir: &Path, repo: Arc<dyn IWorkshopRepository>) -> Arc<Self> {
        Arc::new(Self {
            repo,
            data_dir: data_dir.to_path_buf(),
        })
    }

    // ---- path helpers ----

    fn workshop_dir(&self) -> PathBuf {
        self.data_dir.join(WORKSHOP_REL_DIR)
    }

    fn canvas_dir(&self, id: &str) -> PathBuf {
        self.workshop_dir().join("canvases").join(id)
    }

    fn assets_dir(&self) -> PathBuf {
        self.workshop_dir().join("assets")
    }

    // ---- canvases ----

    pub async fn list_canvases(&self) -> Result<Vec<WorkshopCanvasMeta>, AppError> {
        Ok(self.repo.list_canvases().await?.into_iter().map(WorkshopCanvasMeta::from).collect())
    }

    pub async fn create_canvas(&self, title: Option<String>) -> Result<WorkshopCanvasMeta, AppError> {
        let id = generate_prefixed_id("wsc");
        let title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "未命名画布".to_string());
        let now = now_ms();
        // Write the empty doc first so a crash between INSERT and write can't
        // leave a row whose file is missing (the read path tolerates a missing
        // file, but writing first keeps disk ⊇ index).
        fsio::save_bytes_atomic(&self.canvas_dir(&id), "canvas.json", DEFAULT_DOC.as_bytes())
            .await
            .map_err(|e| AppError::Internal(format!("write canvas doc: {e}")))?;
        let row = self.repo.create_canvas(&id, &title, now).await?;
        Ok(row.into())
    }

    pub async fn get_canvas(&self, id: &str) -> Result<CanvasWithDoc, AppError> {
        let row = self
            .repo
            .get_canvas(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop canvas {id} not found")))?;
        let doc = self.read_doc(id).await;
        Ok(CanvasWithDoc { meta: row.into(), doc })
    }

    /// Read + parse the canvas doc; a missing or corrupt file falls back to the
    /// default empty doc (never fails the read).
    async fn read_doc(&self, id: &str) -> Value {
        let path = self.canvas_dir(id).join("canvas.json");
        match fsio::read_bytes_opt(&path).await {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(id, error = %e, "workshop canvas doc unreadable; serving default");
                default_doc_value()
            }),
            Ok(None) => default_doc_value(),
            Err(e) => {
                tracing::warn!(id, error = %e, "workshop canvas doc read failed; serving default");
                default_doc_value()
            }
        }
    }

    /// Persist an opaque doc (≤ [`MAX_DOC_BYTES`]), sync `node_count` from
    /// `doc.nodes`, and return the new `updated_at`.
    pub async fn save_doc(&self, id: &str, doc: &Value) -> Result<i64, AppError> {
        // Ensure the canvas exists before touching disk.
        if self.repo.get_canvas(id).await?.is_none() {
            return Err(AppError::NotFound(format!("workshop canvas {id} not found")));
        }
        let bytes = serde_json::to_vec(doc).map_err(|e| AppError::BadRequest(format!("invalid doc json: {e}")))?;
        if bytes.len() > MAX_DOC_BYTES {
            return Err(AppError::BadRequest(format!(
                "canvas doc is too large: {} bytes (max {MAX_DOC_BYTES})",
                bytes.len()
            )));
        }
        let node_count = doc
            .get("nodes")
            .and_then(Value::as_array)
            .map(|a| a.len() as i64)
            .unwrap_or(0);
        fsio::save_bytes_atomic(&self.canvas_dir(id), "canvas.json", &bytes)
            .await
            .map_err(|e| AppError::Internal(format!("write canvas doc: {e}")))?;
        let row = self.repo.touch_canvas(id, node_count, now_ms()).await?;
        Ok(row.updated_at)
    }

    pub async fn rename_canvas(&self, id: &str, title: &str) -> Result<WorkshopCanvasMeta, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        Ok(self.repo.rename_canvas(id, title, now_ms()).await?.into())
    }

    pub async fn delete_canvas(&self, id: &str) -> Result<(), AppError> {
        self.repo.delete_canvas(id).await?;
        // Best-effort remove the on-disk body dir (row is the source of truth).
        if let Err(e) = tokio::fs::remove_dir_all(self.canvas_dir(id)).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(id, error = %e, "workshop canvas dir remove failed (row deleted)");
        }
        Ok(())
    }

    // ---- assets ----

    pub async fn upload_asset(&self, input: NewAssetUpload) -> Result<WorkshopAsset, AppError> {
        if input.bytes.is_empty() {
            return Err(AppError::BadRequest("uploaded file is empty".into()));
        }
        if input.bytes.len() > MAX_ASSET_BYTES {
            return Err(AppError::BadRequest(format!(
                "asset is too large: {} bytes (max {MAX_ASSET_BYTES})",
                input.bytes.len()
            )));
        }
        let (ext, mime, kind) = classify_upload(&input.file_name, input.content_type.as_deref())?;
        let (width, height) = if kind == "image" {
            match imagemeta::image_dimensions(&input.bytes) {
                Some((w, h)) => (Some(w as i64), Some(h as i64)),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        let id = generate_prefixed_id("wsa");
        let disk_name = format!("{id}.{ext}");
        let rel_path = format!("{WORKSHOP_REL_DIR}/assets/{disk_name}");
        fsio::save_bytes_atomic(&self.assets_dir(), &disk_name, &input.bytes)
            .await
            .map_err(|e| AppError::Internal(format!("write asset file: {e}")))?;

        let title = input
            .title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| input.file_name.clone());
        let now = now_ms();
        let row = WorkshopAssetRow {
            id,
            kind: kind.to_string(),
            title,
            collection: normalize_opt(input.collection),
            tags: tags_json(input.tags),
            rel_path: Some(rel_path),
            thumb_rel_path: None,
            mime: Some(mime),
            width,
            height,
            bytes: Some(input.bytes.len() as i64),
            text_content: None,
            in_library: input.in_library.unwrap_or(true),
            origin: None,
            created_at: now,
            updated_at: now,
        };
        // Roll the file back if the row insert fails.
        match self.repo.create_asset(&row).await {
            Ok(saved) => Ok(saved.into()),
            Err(e) => {
                if let Some(rel) = &row.rel_path {
                    let _ = tokio::fs::remove_file(self.data_dir.join(rel)).await;
                }
                Err(e.into())
            }
        }
    }

    pub async fn create_text_asset(&self, input: NewTextAsset) -> Result<WorkshopAsset, AppError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(AppError::BadRequest("title must not be empty".into()));
        }
        let now = now_ms();
        let row = WorkshopAssetRow {
            id: generate_prefixed_id("wsa"),
            kind: "text".to_string(),
            title: title.to_string(),
            collection: normalize_opt(input.collection),
            tags: tags_json(input.tags),
            rel_path: None,
            thumb_rel_path: None,
            mime: None,
            width: None,
            height: None,
            bytes: None,
            text_content: Some(input.text_content),
            in_library: input.in_library.unwrap_or(true),
            origin: None,
            created_at: now,
            updated_at: now,
        };
        Ok(self.repo.create_asset(&row).await?.into())
    }

    pub async fn list_assets(&self, query: AssetQuery) -> Result<AssetListPage, AppError> {
        let (rows, total) = self
            .repo
            .list_assets(ListAssetsParams {
                kind: query.kind.as_deref(),
                collection: query.collection.as_deref(),
                q: query.q.as_deref().filter(|s| !s.trim().is_empty()),
                in_library: query.in_library,
                page: query.page,
                page_size: query.page_size,
            })
            .await?;
        Ok(AssetListPage {
            items: rows.into_iter().map(WorkshopAsset::from).collect(),
            total,
        })
    }

    pub async fn patch_asset(&self, id: &str, patch: AssetPatch) -> Result<WorkshopAsset, AppError> {
        // Own the JSON string so the borrowed params can reference it.
        let tags_owned = patch.tags.map(|t| serde_json::to_string(&t).unwrap_or_else(|_| "[]".to_string()));
        let collection = patch
            .collection
            .as_ref()
            .map(|c| if c.trim().is_empty() { None } else { Some(c.trim()) });
        let params = UpdateAssetParams {
            title: patch.title.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            collection,
            tags: tags_owned.as_deref(),
            in_library: patch.in_library,
        };
        Ok(self.repo.update_asset(id, params, now_ms()).await?.into())
    }

    pub async fn delete_asset(&self, id: &str) -> Result<(), AppError> {
        let row = self
            .repo
            .get_asset(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {id} not found")))?;
        self.repo.delete_asset(id).await?;
        for rel in [row.rel_path.as_deref(), row.thumb_rel_path.as_deref()].into_iter().flatten() {
            let abs = self.data_dir.join(rel);
            if let Err(e) = tokio::fs::remove_file(&abs).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(id, path = %abs.display(), error = %e, "workshop asset file remove failed (row deleted)");
            }
        }
        Ok(())
    }

    /// Serve an asset's original (or its thumbnail when `thumb` and one exists).
    /// Traversal-safe: the resolved path must canonicalize within the workshop
    /// dir. Missing file → NotFound.
    pub async fn serve_file(&self, asset_id: &str, thumb: bool) -> Result<ServedFile, AppError> {
        let row = self
            .repo
            .get_asset(asset_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workshop asset {asset_id} not found")))?;

        // ?thumb=1 falls back to the original when no thumbnail exists (contract §3.2).
        let (rel, is_thumb) = match (thumb, row.thumb_rel_path.as_deref()) {
            (true, Some(t)) => (t, true),
            _ => (
                row.rel_path
                    .as_deref()
                    .ok_or_else(|| AppError::NotFound(format!("asset {asset_id} has no file")))?,
                false,
            ),
        };
        let abs = self.resolve_within_workshop(rel)?;
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|_| AppError::NotFound(format!("asset {asset_id} file is missing")))?;
        let mime = if is_thumb {
            "image/webp".to_string()
        } else {
            row.mime.clone().unwrap_or_else(|| "application/octet-stream".to_string())
        };
        Ok(ServedFile { mime, bytes })
    }

    /// Resolve a data-dir-relative path and guarantee it stays inside the
    /// workshop dir (defense-in-depth; `rel_path`s are minted by us).
    fn resolve_within_workshop(&self, rel: &str) -> Result<PathBuf, AppError> {
        if rel.contains('\0') || Path::new(rel).components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(AppError::Forbidden("asset path contains invalid traversal".into()));
        }
        let abs = self.data_dir.join(rel);
        let canonical = std::fs::canonicalize(&abs)
            .map_err(|_| AppError::NotFound("asset file is missing".into()))?;
        let root = std::fs::canonicalize(self.workshop_dir())
            .map_err(|e| AppError::Internal(format!("resolve workshop dir: {e}")))?;
        if !canonical.starts_with(&root) {
            return Err(AppError::Forbidden("asset path escapes the workshop sandbox".into()));
        }
        Ok(canonical)
    }
}

fn default_doc_value() -> Value {
    serde_json::from_str(DEFAULT_DOC).expect("DEFAULT_DOC is valid json")
}

fn normalize_opt(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn tags_json(tags: Option<Vec<String>>) -> String {
    serde_json::to_string(&tags.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string())
}

/// Resolve `(ext, mime, kind)` for an upload. Only image/* and video/* are
/// accepted; anything else is a bad request.
fn classify_upload(file_name: &str, content_type: Option<&str>) -> Result<(String, String, &'static str), AppError> {
    let ext_from_name = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty());
    let guessed_raw = mime_guess::from_path(file_name).first_raw();
    let mime = content_type
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "application/octet-stream")
        .map(str::to_string)
        .or_else(|| guessed_raw.map(str::to_string))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let kind = if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("video/") {
        "video"
    } else {
        return Err(AppError::BadRequest(format!(
            "unsupported media type '{mime}': only image/* and video/* uploads are accepted"
        )));
    };

    let ext = ext_from_name
        .or_else(|| {
            mime_guess::get_mime_extensions_str(&mime).and_then(|exts| exts.first().map(|e| e.to_string()))
        })
        .unwrap_or_else(|| "bin".to_string());
    Ok((ext, mime, kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::SqliteWorkshopRepository;

    async fn service() -> (Arc<WorkshopService>, tempfile::TempDir) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo: Arc<dyn IWorkshopRepository> = Arc::new(SqliteWorkshopRepository::new(db.pool().clone()));
        Box::leak(Box::new(db));
        let dir = tempfile::tempdir().unwrap();
        (WorkshopService::start(dir.path(), repo), dir)
    }

    // A 1x1 PNG.
    fn png_1x1() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
        b.extend_from_slice(&[0, 0, 0, 13]);
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b
    }

    #[tokio::test]
    async fn canvas_create_read_save_delete() {
        let (svc, dir) = service().await;
        let meta = svc.create_canvas(None).await.unwrap();
        assert_eq!(meta.title, "未命名画布");
        assert!(meta.id.starts_with("wsc_"));
        assert!(dir.path().join("workshop/canvases").join(&meta.id).join("canvas.json").exists());

        // default doc parses; save a doc with 2 nodes → node_count syncs.
        let read = svc.get_canvas(&meta.id).await.unwrap();
        assert_eq!(read.doc["schema"], 1);
        let doc = serde_json::json!({"schema":1,"nodes":[{"id":"a"},{"id":"b"}],"edges":[]});
        let updated_at = svc.save_doc(&meta.id, &doc).await.unwrap();
        assert!(updated_at >= meta.created_at);
        let all = svc.list_canvases().await.unwrap();
        assert_eq!(all[0].node_count, 2);

        // rename
        let renamed = svc.rename_canvas(&meta.id, "  我的画布  ").await.unwrap();
        assert_eq!(renamed.title, "我的画布");
        assert!(svc.rename_canvas(&meta.id, "   ").await.is_err());

        // delete removes row + dir
        svc.delete_canvas(&meta.id).await.unwrap();
        assert!(!dir.path().join("workshop/canvases").join(&meta.id).exists());
        assert!(svc.get_canvas(&meta.id).await.is_err());
    }

    #[tokio::test]
    async fn save_doc_rejects_oversize_and_unknown_canvas() {
        let (svc, _dir) = service().await;
        assert!(svc.save_doc("wsc_missing", &serde_json::json!({})).await.is_err());
    }

    #[tokio::test]
    async fn upload_image_extracts_dimensions_and_serves() {
        let (svc, _dir) = service().await;
        let asset = svc
            .upload_asset(NewAssetUpload {
                file_name: "shot.png".into(),
                content_type: Some("image/png".into()),
                bytes: png_1x1(),
                title: None,
                collection: Some("角色".into()),
                tags: Some(vec!["a".into()]),
                in_library: None,
            })
            .await
            .unwrap();
        assert_eq!(asset.kind, "image");
        assert_eq!(asset.width, Some(1));
        assert_eq!(asset.height, Some(1));
        assert!(asset.in_library);
        assert_eq!(asset.url, format!("/api/workshop/files/{}", asset.id));

        // serve returns the bytes + mime
        let served = svc.serve_file(&asset.id, false).await.unwrap();
        assert_eq!(served.mime, "image/png");
        assert_eq!(served.bytes, png_1x1());
        // thumb=1 falls back to original when no thumb exists
        let served_thumb = svc.serve_file(&asset.id, true).await.unwrap();
        assert_eq!(served_thumb.bytes, png_1x1());
    }

    #[tokio::test]
    async fn upload_rejects_non_media() {
        let (svc, _dir) = service().await;
        let err = svc
            .upload_asset(NewAssetUpload {
                file_name: "notes.txt".into(),
                content_type: Some("text/plain".into()),
                bytes: b"hi".to_vec(),
                title: None,
                collection: None,
                tags: None,
                in_library: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn text_asset_list_patch_delete() {
        let (svc, _dir) = service().await;
        let a = svc
            .create_text_asset(NewTextAsset {
                title: "描述".into(),
                text_content: "武松打虎".into(),
                collection: None,
                tags: None,
                in_library: Some(false),
            })
            .await
            .unwrap();
        assert_eq!(a.kind, "text");
        assert!(!a.in_library);
        assert_eq!(a.text_content.as_deref(), Some("武松打虎"));

        let patched = svc
            .patch_asset(
                &a.id,
                AssetPatch {
                    title: Some("新标题".into()),
                    collection: Some("场景".into()),
                    in_library: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(patched.title, "新标题");
        assert_eq!(patched.collection.as_deref(), Some("场景"));
        assert!(patched.in_library);

        let page = svc
            .list_assets(AssetQuery { page: 1, page_size: 20, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(page.total, 1);

        // text asset has no file → serve is NotFound
        assert!(svc.serve_file(&a.id, false).await.is_err());
        svc.delete_asset(&a.id).await.unwrap();
        assert!(svc.serve_file(&a.id, false).await.is_err());
    }
}
