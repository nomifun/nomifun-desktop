use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `miniapps` table — one solidified, self-contained
/// single-file web tool owned by a user.
///
/// Deliberately carries `html_size` (a stored column holding the document's byte
/// length) instead of the document itself. Metadata reads are the common case —
/// the library grid lists every app the owner has — and each body may be
/// megabytes, so neither selecting them nor measuring them in SQL is affordable:
/// either would make a list request proportional to the total bytes ever
/// solidified. The body is read only by the serve path, through
/// [`MiniAppDocumentRow`].
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppRow {
    pub id: i64,
    pub miniapp_id: String,
    pub user_id: String,
    pub name: String,
    pub description: String,
    /// Optional emoji or very short label.
    pub icon: Option<String>,
    /// The conversation this app was solidified from, when there was one. Pure
    /// provenance: it drives no navigation, and the conversation it names may
    /// have been deleted since (the reference is SetNull).
    pub source_conversation_id: Option<String>,
    /// Byte length of the stored HTML document.
    pub html_size: i64,
    /// When the stored document (the published snapshot) was last written from
    /// the on-disk working copy. `None` for rows solidified before publishing
    /// became an explicit act, and for apps whose working copy was never
    /// materialized. Never inferred from `updated_at`, which a rename moves.
    pub published_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// The stored HTML document, and nothing else.
///
/// The serve path resolves an app by id without an owner predicate (an iframe
/// subresource load carries no credentials), so the row it reads must not be
/// able to disclose anything beyond the body it is about to render — no name, no
/// owner, no provenance. Keeping that read in its own shape is what makes that a
/// property of the type rather than of every future caller's discipline.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MiniAppDocumentRow {
    pub html: String,
}
