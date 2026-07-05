//! Wire DTOs for the `/api/workshop/*` surface (contract §3.1/§3.2). All fields
//! are snake_case (serde default) per the wire contract. These are response
//! shapes the frontend `types.ts` mirrors; the domain crate owns them (the
//! shared `api-types` crate is not in this module's ownership).

use nomifun_common::TimestampMs;
use nomifun_db::{WorkshopAssetRow, WorkshopCanvasRow};
use serde::Serialize;
use serde_json::Value;

/// A canvas index entry. `thumbnail_url` is `None` until thumbnail generation
/// lands (M3); the `workshop_canvases.thumbnail_rel_path` column is populated
/// then and a serve route wired.
#[derive(Debug, Clone, Serialize)]
pub struct WorkshopCanvasMeta {
    pub id: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub node_count: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<WorkshopCanvasRow> for WorkshopCanvasMeta {
    fn from(row: WorkshopCanvasRow) -> Self {
        // M0 has no thumbnail generation; keep None even if a rel_path exists so
        // we never advertise a URL with no serve route behind it.
        Self {
            id: row.id,
            title: row.title,
            thumbnail_url: None,
            node_count: row.node_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A workshop asset. `url` always points at the files route (a `text` asset has
/// no binary, so its `url` 404s — the frontend uses `text_content` for those).
#[derive(Debug, Clone, Serialize)]
pub struct WorkshopAsset {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub collection: Option<String>,
    pub tags: Vec<String>,
    pub mime: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: Option<i64>,
    pub in_library: bool,
    pub text_content: Option<String>,
    pub origin: Option<Value>,
    pub url: String,
    pub thumb_url: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<WorkshopAssetRow> for WorkshopAsset {
    fn from(row: WorkshopAssetRow) -> Self {
        // `tags` / `origin` are stored as JSON TEXT; parse leniently (a corrupt
        // value degrades to empty/none rather than failing the whole response).
        let tags = serde_json::from_str::<Vec<String>>(&row.tags).unwrap_or_default();
        let origin = row.origin.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok());
        let url = format!("/api/workshop/files/{}", row.id);
        let thumb_url = row
            .thumb_rel_path
            .as_ref()
            .map(|_| format!("/api/workshop/files/{}?thumb=1", row.id));
        Self {
            id: row.id,
            kind: row.kind,
            title: row.title,
            collection: row.collection,
            tags,
            mime: row.mime,
            width: row.width,
            height: row.height,
            bytes: row.bytes,
            in_library: row.in_library,
            text_content: row.text_content,
            origin,
            url,
            thumb_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset_row() -> WorkshopAssetRow {
        WorkshopAssetRow {
            id: "wsa_1".into(),
            kind: "image".into(),
            title: "t".into(),
            collection: Some("角色".into()),
            tags: r#"["a","b"]"#.into(),
            rel_path: Some("workshop/assets/wsa_1.png".into()),
            thumb_rel_path: None,
            mime: Some("image/png".into()),
            width: Some(10),
            height: Some(20),
            bytes: Some(99),
            text_content: None,
            in_library: true,
            origin: Some(r#"{"prompt":"cat"}"#.into()),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn asset_dto_parses_tags_origin_and_builds_url() {
        let dto = WorkshopAsset::from(asset_row());
        assert_eq!(dto.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(dto.origin.unwrap()["prompt"], "cat");
        assert_eq!(dto.url, "/api/workshop/files/wsa_1");
        assert!(dto.thumb_url.is_none());
    }

    #[test]
    fn asset_dto_corrupt_tags_degrade_to_empty() {
        let mut row = asset_row();
        row.tags = "not json".into();
        row.origin = Some("also not json".into());
        let dto = WorkshopAsset::from(row);
        assert!(dto.tags.is_empty());
        assert!(dto.origin.is_none());
    }

    #[test]
    fn canvas_meta_never_advertises_thumbnail_in_m0() {
        let row = WorkshopCanvasRow {
            id: "wsc_1".into(),
            title: "c".into(),
            thumbnail_rel_path: Some("workshop/canvases/wsc_1/thumb.webp".into()),
            node_count: 3,
            created_at: 1,
            updated_at: 2,
        };
        let meta = WorkshopCanvasMeta::from(row);
        assert!(meta.thumbnail_url.is_none());
        assert_eq!(meta.node_count, 3);
    }
}
