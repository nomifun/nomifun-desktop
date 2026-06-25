use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row in the `webhooks` table — a reusable outbound webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookRow {
    pub id: i64,
    pub name: String,
    /// Platform discriminator; `lark` is the only supported value in v1.
    pub platform: String,
    pub url: String,
    /// Optional signing secret (Lark "加签"); never returned to clients.
    pub secret: Option<String>,
    pub description: String,
    pub enabled: bool,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_row_roundtrips() {
        let row = WebhookRow {
            id: 1,
            name: "Team bot".into(),
            platform: "lark".into(),
            url: "https://open.feishu.cn/open-apis/bot/v2/hook/xxx".into(),
            secret: Some("s3cr3t".into()),
            description: "notifications".into(),
            enabled: true,
            created_at: 1,
            updated_at: 2,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: WebhookRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, row.id);
        assert_eq!(back.platform, "lark");
        assert!(back.enabled);
    }
}
