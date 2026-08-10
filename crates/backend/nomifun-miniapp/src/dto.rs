//! HTTP DTOs for mini-apps.
//!
//! Wire shape is snake_case (preset-style, matching the `miniapps` client
//! namespace), and request DTOs are `Deserialize`-only with
//! `deny_unknown_fields`. No response carries the stored HTML document: the
//! library grid and the detail view need a size, not megabytes, and the one
//! consumer of the body is the serve route that streams it straight into an
//! iframe. `html_size` is the stored document's byte length so a client can show
//! "how big is this app" without asking for it.
use nomifun_common::TimestampMs;
use nomifun_db::MiniAppRow;
use serde::{Deserialize, Serialize};

/// Owner-visible view of a solidified mini-app. Metadata only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MiniAppResponse {
    pub miniapp_id: String,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub source_conversation_id: Option<String>,
    /// Byte length of the stored HTML document.
    pub html_size: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<MiniAppRow> for MiniAppResponse {
    fn from(r: MiniAppRow) -> Self {
        MiniAppResponse {
            miniapp_id: r.miniapp_id,
            name: r.name,
            description: r.description,
            icon: r.icon,
            source_conversation_id: r.source_conversation_id,
            html_size: r.html_size,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Solidify request. `html` is the complete self-contained document; it is
/// stored verbatim and never echoed back.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct CreateMiniAppRequest {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub html: String,
    /// The conversation this app came from, when there was one.
    pub source_conversation_id: Option<String>,
}

/// Edit request. A `None` field is left unchanged; an empty `icon` clears it. At
/// least one field must be present — an empty body is a client bug, and
/// answering it with an unchanged row would hide that.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct UpdateMiniAppRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub html: Option<String>,
}

impl UpdateMiniAppRequest {
    /// Whether the request asks for any change at all.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.icon.is_none()
            && self.html.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> MiniAppRow {
        MiniAppRow {
            id: 7,
            miniapp_id: "0190f5fe-7c00-7a00-8000-000000000001".to_string(),
            user_id: "0190f5fe-7c00-7a00-8000-0000000000ff".to_string(),
            name: "Timer".to_string(),
            description: "a pomodoro timer".to_string(),
            icon: Some("⏱".to_string()),
            source_conversation_id: Some("0190f5fe-7c00-7a00-8000-000000000002".to_string()),
            html_size: 2048,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_500,
        }
    }

    #[test]
    fn response_is_snake_case_and_carries_no_html() {
        let value = serde_json::to_value(MiniAppResponse::from(row())).expect("serialize");
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        // Sorted, because serde_json's map ordering is not part of the contract —
        // the exact key set is, and `html` must never join it.
        let mut expected = vec![
            "miniapp_id",
            "name",
            "description",
            "icon",
            "source_conversation_id",
            "html_size",
            "created_at",
            "updated_at",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(value["html_size"], 2048);
    }

    #[test]
    fn absent_optional_fields_deserialize_to_none() {
        let req: CreateMiniAppRequest =
            serde_json::from_str(r#"{"name":"Timer","html":"<h1>hi</h1>"}"#).expect("parse");
        assert_eq!(req.description, None);
        assert_eq!(req.icon, None);
        assert_eq!(req.source_conversation_id, None);
    }

    #[test]
    fn unknown_fields_are_refused_on_both_requests() {
        assert!(
            serde_json::from_str::<CreateMiniAppRequest>(
                r#"{"name":"a","html":"<p/>","bogus":1}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<UpdateMiniAppRequest>(r#"{"bogus":1}"#).is_err());
    }

    #[test]
    fn an_all_none_update_is_recognized_as_empty() {
        let req: UpdateMiniAppRequest = serde_json::from_str("{}").expect("parse");
        assert!(req.is_empty());
        let req: UpdateMiniAppRequest = serde_json::from_str(r#"{"name":"x"}"#).expect("parse");
        assert!(!req.is_empty());
    }
}
