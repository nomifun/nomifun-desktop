//! HTTP DTOs for mini-apps.
//!
//! Wire shape is snake_case (preset-style, matching the `miniapps` client
//! namespace), and request DTOs are `Deserialize`-only with
//! `deny_unknown_fields`. No response carries the stored HTML document: the
//! library grid and the detail view need a size, not megabytes, and the one
//! consumer of the body is the serve route that streams it straight into an
//! iframe. `html_size` is the stored document's byte length so a client can show
//! "how big is this app" without asking for it.
//!
//! One field is not a column: `has_unpublished_changes` compares the on-disk
//! working copy's mtime against the publish instant, so the detail page can say
//! "you have iterated since you last published" without downloading either
//! version.
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
    /// The conversation this app was solidified from, when there was one. Pure
    /// provenance: it drives no navigation and may name a conversation the user
    /// has since deleted (the link is then NULL). Its one reader is the preview
    /// panel's 「替换已有小程序」 picker, which preselects the app this conversation
    /// published before.
    pub source_conversation_id: Option<String>,
    /// Byte length of the stored HTML document.
    pub html_size: i64,
    /// When the stored snapshot last became current. `null` for apps solidified
    /// before publishing was an explicit act.
    pub published_at: Option<TimestampMs>,
    /// Whether the on-disk working copy is newer than the published snapshot —
    /// i.e. whether iterating has produced changes the runner is not serving yet.
    /// Derived per request from the working copy's mtime, never stored: a stored
    /// flag would go stale the moment an agent wrote the file, which is every
    /// turn.
    pub has_unpublished_changes: bool,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl MiniAppResponse {
    /// Project a row plus the one fact the row cannot know.
    ///
    /// There is deliberately no `From<MiniAppRow>`: `has_unpublished_changes`
    /// lives on disk, and a conversion that defaulted it to `false` would let a
    /// new call site silently tell every user their edits are published.
    pub fn new(row: MiniAppRow, has_unpublished_changes: bool) -> Self {
        MiniAppResponse {
            miniapp_id: row.miniapp_id,
            name: row.name,
            description: row.description,
            icon: row.icon,
            source_conversation_id: row.source_conversation_id,
            html_size: row.html_size,
            published_at: row.published_at,
            has_unpublished_changes,
            created_at: row.created_at,
            updated_at: row.updated_at,
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

/// Where a mini-app's source lives on disk, after ensuring it is there.
///
/// The answer to `POST /api/miniapps/{miniapp_id}/workspace`, which provisions the
/// app's directory and materializes the working copy when it is missing. It
/// creates no conversation: 「继续迭代」 takes this path and writes it into the first
/// message of an ORDINARY conversation, which then reads and edits the file with
/// ordinary file tools.
///
/// `source_path` is never an input. The client does not send a path — the server
/// derives this one from `miniapp_id` and runs it through the escape guard — and
/// the client only reads it back to compose that message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MiniAppWorkspaceResponse {
    /// Absolute path of `{work_dir}/miniapps/{miniapp_id}/miniapp.html`. Absolute
    /// because its reader is a model working in some other conversation's
    /// workspace, for which a relative path would name nothing.
    pub source_path: String,
}

/// Import intake. Exactly one of `html` / `path` must be present — both is a
/// client bug and neither leaves nothing to judge, so each is rejected with its
/// own message instead of being silently coerced.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct MiniAppImportRequest {
    /// Overrides the document's `<title>` when naming the app.
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    /// The document itself (paste, or a renderer that already holds the bytes).
    pub html: Option<String>,
    /// An absolute path the user picked: one `.html` document, or the folder that
    /// holds its `index.html`.
    pub path: Option<String>,
}

/// What an import or a validation says.
///
/// `app` is `Some` only on a successful import, so one response type serves both
/// routes and a client can never mistake "reported" for "adopted".
#[derive(Debug, Clone, Serialize)]
pub struct MiniAppImportResponse {
    pub report: crate::validation::ImportReport,
    /// Rule ids that were actually repaired — never the ones the catalogue merely
    /// hoped to repair.
    pub applied_fixes: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<MiniAppResponse>,
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
            published_at: Some(1_700_000_000_400),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_500,
        }
    }

    #[test]
    fn response_is_snake_case_and_carries_no_html() {
        let value = serde_json::to_value(MiniAppResponse::new(row(), true)).expect("serialize");
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
            "published_at",
            "has_unpublished_changes",
            "created_at",
            "updated_at",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(value["html_size"], 2048);
        assert_eq!(value["published_at"], 1_700_000_000_400i64);
        assert_eq!(value["has_unpublished_changes"], true);
    }

    /// The publish state is a per-request fact, so the projection must carry
    /// whatever it is told and nothing else — no defaulting, no inference from
    /// the timestamps in the row.
    #[test]
    fn the_unpublished_flag_is_whatever_the_caller_measured() {
        let value = serde_json::to_value(MiniAppResponse::new(row(), false)).expect("serialize");
        assert_eq!(value["has_unpublished_changes"], false);
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

    /// One field, snake_case, and it is a path — the client's whole use for this
    /// route is reading it back into the first message of an ordinary
    /// conversation.
    #[test]
    fn the_workspace_response_carries_only_the_source_path() {
        let value = serde_json::to_value(MiniAppWorkspaceResponse {
            source_path: "/w/miniapps/0190f5fe-7c00-7a00-8000-000000000001/miniapp.html".into(),
        })
        .expect("serialize");
        let keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["source_path"]);
        assert_eq!(
            value["source_path"],
            "/w/miniapps/0190f5fe-7c00-7a00-8000-000000000001/miniapp.html"
        );
    }
}
