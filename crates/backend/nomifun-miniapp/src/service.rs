//! `MiniAppService`: owner-scoped mini-app CRUD plus the one unscoped read the
//! serve route needs.
//!
//! All validation lives here as pure functions so the limits are testable
//! without a database and identical on create and update. The service is the only
//! writer of the `miniapps` table, so a document that reached storage satisfies
//! every bound below.
use std::sync::Arc;

use nomifun_common::{ConversationId, MiniAppId};
use nomifun_db::{CreateMiniAppParams, IMiniAppRepository, UpdateMiniAppParams};

use crate::dto::{CreateMiniAppRequest, MiniAppResponse, UpdateMiniAppRequest};

/// Longest display name. A name is a card label, not a description.
pub const MINI_APP_NAME_MAX_CHARS: usize = 100;
/// Longest description. Two lines of card subtitle.
pub const MINI_APP_DESCRIPTION_MAX_CHARS: usize = 500;
/// Longest icon. An emoji (which can be several chars once modifiers and
/// zero-width joiners are counted) or a two-or-three letter monogram — never a
/// sentence smuggled into the grid.
pub const MINI_APP_ICON_MAX_CHARS: usize = 16;
/// Largest storable document. Generous for one self-contained page (CDN links
/// keep libraries out of the body), and small enough that a runaway generation
/// cannot fill the database one row at a time.
pub const MINI_APP_HTML_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Errors surfaced by the mini-app service.
#[derive(Debug, thiserror::Error)]
pub enum MiniAppServiceError {
    #[error("mini-app not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// Mini-app service. Cheap to clone (`Arc` internals).
#[derive(Clone)]
pub struct MiniAppService {
    repo: Arc<dyn IMiniAppRepository>,
}

impl MiniAppService {
    pub fn new(repo: Arc<dyn IMiniAppRepository>) -> Self {
        Self { repo }
    }

    /// The owner's apps, most recently updated first.
    pub async fn list(&self, user_id: &str) -> Result<Vec<MiniAppResponse>, MiniAppServiceError> {
        let rows = self
            .repo
            .list(user_id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(MiniAppResponse::from).collect())
    }

    /// One owned app.
    pub async fn get(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        self.repo
            .find(user_id, id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?
            .map(MiniAppResponse::from)
            .ok_or(MiniAppServiceError::NotFound)
    }

    /// Solidify a new app for `user_id`.
    pub async fn create(
        &self,
        user_id: &str,
        req: CreateMiniAppRequest,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        let name = validate_name(&req.name)?;
        let description = validate_description(req.description.as_deref().unwrap_or_default())?;
        let icon = validate_icon(req.icon.as_deref().unwrap_or_default())?;
        let html = validate_html(&req.html)?;
        let source_conversation_id = req
            .source_conversation_id
            .as_deref()
            .map(validate_source_conversation_id)
            .transpose()?;

        let row = self
            .repo
            .create(
                user_id,
                CreateMiniAppParams {
                    name: &name,
                    description: &description,
                    icon: icon.as_deref(),
                    html,
                    source_conversation_id: source_conversation_id.as_ref().map(|id| id.as_str()),
                },
            )
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?;
        Ok(MiniAppResponse::from(row))
    }

    /// Edit an owned app. At least one field must be present.
    pub async fn update(
        &self,
        user_id: &str,
        id: &MiniAppId,
        req: UpdateMiniAppRequest,
    ) -> Result<MiniAppResponse, MiniAppServiceError> {
        if req.is_empty() {
            return Err(MiniAppServiceError::BadRequest(
                "an update must change at least one field".into(),
            ));
        }
        let name = req.name.as_deref().map(validate_name).transpose()?;
        let description = req
            .description
            .as_deref()
            .map(validate_description)
            .transpose()?;
        // `Some(None)` clears the icon; the DTO's empty string is how a client
        // says "remove it", and `validate_icon` already normalizes that to None.
        let icon = req.icon.as_deref().map(validate_icon).transpose()?;
        let html = req.html.as_deref().map(validate_html).transpose()?;

        let row = self
            .repo
            .update(
                user_id,
                id,
                UpdateMiniAppParams {
                    name: name.as_deref(),
                    description: description.as_deref(),
                    icon: icon.as_ref().map(|value| value.as_deref()),
                    html,
                },
            )
            .await
            .map_err(map_not_found)?;
        Ok(MiniAppResponse::from(row))
    }

    /// Delete an owned app.
    pub async fn delete(
        &self,
        user_id: &str,
        id: &MiniAppId,
    ) -> Result<(), MiniAppServiceError> {
        self.repo.delete(user_id, id).await.map_err(map_not_found)
    }

    /// The stored HTML document, for the auth-exempt serve route.
    ///
    /// Unscoped by necessity: an iframe subresource load presents no credentials,
    /// so there is no owner to compare against. The id is an unguessable bare
    /// UUIDv7 and the repository read returns the body alone, so a caller holding
    /// a link learns the document and nothing about who owns it.
    pub async fn serve_html(&self, id: &MiniAppId) -> Result<String, MiniAppServiceError> {
        self.repo
            .find_by_id_any_owner(id)
            .await
            .map_err(|e| MiniAppServiceError::Internal(e.to_string()))?
            .map(|document| document.html)
            .ok_or(MiniAppServiceError::NotFound)
    }
}

/// Trim, then require a non-empty name within the length cap.
pub(crate) fn validate_name(raw: &str) -> Result<String, MiniAppServiceError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(MiniAppServiceError::BadRequest("name is required".into()));
    }
    if name.chars().count() > MINI_APP_NAME_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "name must be at most {MINI_APP_NAME_MAX_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

/// Trim; an absent description is the empty string, not NULL (the column has a
/// `''` default, so "no description" has one spelling).
pub(crate) fn validate_description(raw: &str) -> Result<String, MiniAppServiceError> {
    let description = raw.trim();
    if description.chars().count() > MINI_APP_DESCRIPTION_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "description must be at most {MINI_APP_DESCRIPTION_MAX_CHARS} characters"
        )));
    }
    Ok(description.to_string())
}

/// Trim; an empty icon is `None` so the grid falls back to its default glyph
/// instead of rendering a blank box.
pub(crate) fn validate_icon(raw: &str) -> Result<Option<String>, MiniAppServiceError> {
    let icon = raw.trim();
    if icon.is_empty() {
        return Ok(None);
    }
    if icon.chars().count() > MINI_APP_ICON_MAX_CHARS {
        return Err(MiniAppServiceError::BadRequest(format!(
            "icon must be at most {MINI_APP_ICON_MAX_CHARS} characters"
        )));
    }
    Ok(Some(icon.to_string()))
}

/// Require a non-blank document within the size cap, and store it verbatim.
///
/// The body is NOT trimmed: leading whitespace inside a `<pre>` is the author's,
/// and the byte length reported back to the client must be the length of what was
/// stored.
pub(crate) fn validate_html(raw: &str) -> Result<&str, MiniAppServiceError> {
    if raw.trim().is_empty() {
        return Err(MiniAppServiceError::BadRequest("html is required".into()));
    }
    if raw.len() > MINI_APP_HTML_MAX_BYTES {
        return Err(MiniAppServiceError::BadRequest(format!(
            "html must be at most {MINI_APP_HTML_MAX_BYTES} bytes"
        )));
    }
    Ok(raw)
}

/// A supplied provenance id must be a canonical bare UUIDv7 — the column's CHECK
/// enforces the same thing, and failing here names the field instead of surfacing
/// a constraint violation as an internal error.
pub(crate) fn validate_source_conversation_id(
    raw: &str,
) -> Result<ConversationId, MiniAppServiceError> {
    ConversationId::parse(raw).map_err(|e| {
        MiniAppServiceError::BadRequest(format!("source_conversation_id is invalid: {e}"))
    })
}

fn map_not_found(e: nomifun_db::DbError) -> MiniAppServiceError {
    match e {
        nomifun_db::DbError::NotFound(_) => MiniAppServiceError::NotFound,
        other => MiniAppServiceError::Internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: Result<impl std::fmt::Debug, MiniAppServiceError>) -> String {
        match result {
            Err(MiniAppServiceError::BadRequest(m)) => m,
            other => panic!("expected a bad-request rejection, got {other:?}"),
        }
    }

    #[test]
    fn name_is_trimmed_required_and_capped() {
        assert_eq!(validate_name("  Timer  ").unwrap(), "Timer");
        assert!(message(validate_name("   ")).contains("required"));
        // The cap counts characters, not bytes: 100 CJK names are common and a
        // byte cap would reject a third of them.
        let hundred = "小".repeat(MINI_APP_NAME_MAX_CHARS);
        assert_eq!(validate_name(&hundred).unwrap().chars().count(), 100);
        assert!(message(validate_name(&"小".repeat(MINI_APP_NAME_MAX_CHARS + 1))).contains("100"));
    }

    #[test]
    fn description_is_optional_trimmed_and_capped() {
        assert_eq!(validate_description("").unwrap(), "");
        assert_eq!(validate_description(" hi ").unwrap(), "hi");
        assert!(validate_description(&"a".repeat(MINI_APP_DESCRIPTION_MAX_CHARS)).is_ok());
        assert!(
            message(validate_description(&"a".repeat(MINI_APP_DESCRIPTION_MAX_CHARS + 1)))
                .contains("500")
        );
    }

    #[test]
    fn blank_icon_becomes_none_and_long_icon_is_refused() {
        assert_eq!(validate_icon("  ").unwrap(), None);
        assert_eq!(validate_icon(" ⏱ ").unwrap().as_deref(), Some("⏱"));
        assert!(message(validate_icon(&"x".repeat(MINI_APP_ICON_MAX_CHARS + 1))).contains("16"));
    }

    #[test]
    fn html_is_required_stored_verbatim_and_size_capped() {
        assert!(message(validate_html("   \n ")).contains("required"));
        // Verbatim: whitespace inside the document is the author's, and the byte
        // length the client is told must be the length of what was stored.
        assert_eq!(validate_html("\n<h1>hi</h1>\n").unwrap(), "\n<h1>hi</h1>\n");
        let at_cap = "a".repeat(MINI_APP_HTML_MAX_BYTES);
        assert!(validate_html(&at_cap).is_ok());
        let over_cap = "a".repeat(MINI_APP_HTML_MAX_BYTES + 1);
        assert!(message(validate_html(&over_cap)).contains("bytes"));
    }

    #[test]
    fn source_conversation_id_must_be_a_canonical_uuidv7() {
        let good = "0190f5fe-7c00-7a00-8000-000000000002";
        assert_eq!(
            validate_source_conversation_id(good).unwrap().as_str(),
            good
        );
        for bad in [
            "",
            "conv-1",
            // v4, not v7 — the column's CHECK would reject it as a constraint
            // violation, which reaches the client as a 500 rather than a field error.
            "9f1b5c62-2f3a-4b19-9c1e-2f4d6a8b0c11",
            // Uppercase is not canonical here even though it parses as a UUID.
            "0190F5FE-7C00-7A00-8000-000000000002",
        ] {
            assert!(
                validate_source_conversation_id(bad).is_err(),
                "{bad} must be refused"
            );
        }
    }
}
