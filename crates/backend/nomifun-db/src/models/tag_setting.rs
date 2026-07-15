use nomifun_common::{TimestampMs, WebhookId};
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqliteRow};

/// Row in the `tag_settings` table — per-tag augmentation of the implicit
/// requirement tags (a bound webhook + a description). Tags themselves remain
/// derived from `requirements.tag`; this table only stores extra config keyed by
/// tag name, created on first write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSettingRow {
    pub tag: String,
    /// Bound webhook id (`webhooks.id`); `None` means no webhook is bound.
    pub webhook_id: Option<WebhookId>,
    pub description: String,
    /// Comma-separated subset of `done,failed,needs_review` controlling which
    /// completion events fire the bound webhook. Defaults to all three.
    pub notify_events: String,
    pub updated_at: TimestampMs,
}

impl<'row> sqlx::FromRow<'row, SqliteRow> for TagSettingRow {
    fn from_row(row: &'row SqliteRow) -> Result<Self, sqlx::Error> {
        let webhook_id = row
            .try_get::<Option<String>, _>("webhook_id")?
            .map(WebhookId::try_from)
            .transpose()
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
        Ok(Self {
            tag: row.try_get("tag")?,
            webhook_id,
            description: row.try_get("description")?,
            notify_events: row.try_get("notify_events")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_setting_row_roundtrips() {
        let row = TagSettingRow {
            tag: "alpha".into(),
            webhook_id: Some(WebhookId::new()),
            description: "team alpha queue".into(),
            notify_events: "done,failed,needs_review".to_string(),
            updated_at: 9,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: TagSettingRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tag, "alpha");
        assert_eq!(back.webhook_id, row.webhook_id);
    }

    #[tokio::test]
    async fn row_decode_rejects_noncanonical_webhook_id() {
        let db = crate::init_database_memory().await.unwrap();
        let result = sqlx::query_as::<_, TagSettingRow>(
            "SELECT 'alpha' AS tag, 'webhook_1' AS webhook_id, '' AS description, \
             'done' AS notify_events, 1 AS updated_at",
        )
        .fetch_one(db.pool())
        .await;
        assert!(result.is_err());
    }
}
