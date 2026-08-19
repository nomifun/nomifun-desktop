//! Recognizing responses that are not API responses at all.
//!
//! An OpenAI-compatible gateway typically serves its marketing site or admin SPA
//! from the same host as its API. Requesting a path that is *almost* right —
//! `/chat/completions` when the gateway wants `/v1/chat/completions` — therefore
//! returns `200 OK` with an HTML document rather than a `404`.
//!
//! Status-only success checks accept that. The SSE reader then discards every
//! line that is not `data:`, the stream ends without a terminator, and the user
//! is told the provider "stream ended before finish_reason" — a message that
//! points at the model instead of at the URL, and which is retried twice before
//! being surfaced. Detecting markup here turns that into an actionable
//! statement about the address.

use reqwest::header::{CONTENT_TYPE, HeaderMap};

/// What to tell the user when a response body is a document, not an API payload.
pub const NON_API_DIAGNOSTIC: &str = "the URL answered with a web page, not an API response";

/// Content types no JSON or SSE model API ever answers with.
const MARKUP_CONTENT_TYPES: &[&str] = &[
    "text/html",
    "application/xhtml",
    "text/xml",
    "application/xml",
];

/// The response's content type when it is one an API never serves.
///
/// Returns the offending content-type string so callers can name it in a
/// diagnostic. `None` means the content type is absent or plausible — absent is
/// deliberately not an error, because some correct providers omit it.
pub fn is_non_api_content_type(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(CONTENT_TYPE)?.to_str().ok()?;
    let essence = value.split(';').next().unwrap_or(value).trim();
    MARKUP_CONTENT_TYPES
        .iter()
        .any(|candidate| essence.eq_ignore_ascii_case(candidate))
        .then(|| value.trim().to_owned())
}

/// Does this body prefix look like a markup document?
///
/// The backstop for a provider that serves HTML with a wrong or missing
/// content-type. Skips a UTF-8 BOM and leading whitespace, then looks for the
/// handful of document openings that cannot begin a JSON or SSE payload.
pub fn looks_like_markup(prefix: &[u8]) -> bool {
    let without_bom = prefix.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(prefix);
    let trimmed = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |start| &without_bom[start..]);
    let head = &trimmed[..trimmed.len().min(64)];
    let Ok(text) = std::str::from_utf8(head) else {
        // A partial multi-byte character at the cut is not markup evidence.
        return false;
    };
    let lowered = text.to_ascii_lowercase();
    ["<!doctype", "<html", "<?xml", "<head", "<body"]
        .iter()
        .any(|marker| lowered.starts_with(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(content_type: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
        map
    }

    #[test]
    fn html_content_types_are_rejected_with_their_value() {
        assert_eq!(
            is_non_api_content_type(&headers("text/html; charset=utf-8")).as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            is_non_api_content_type(&headers("TEXT/HTML")).as_deref(),
            Some("TEXT/HTML")
        );
    }

    #[test]
    fn api_content_types_pass() {
        for value in [
            "application/json",
            "application/json; charset=utf-8",
            "text/event-stream",
            "audio/mpeg",
            "application/octet-stream",
        ] {
            assert!(
                is_non_api_content_type(&headers(value)).is_none(),
                "{value} must be allowed"
            );
        }
    }

    #[test]
    fn a_missing_content_type_is_not_treated_as_markup() {
        assert!(is_non_api_content_type(&HeaderMap::new()).is_none());
    }

    #[test]
    fn markup_bodies_are_detected_through_bom_and_whitespace() {
        assert!(looks_like_markup(b"<!doctype html>"));
        assert!(looks_like_markup(b"<!DOCTYPE HTML PUBLIC>"));
        assert!(looks_like_markup(b"\n\n  <html lang=\"zh-CN\">"));
        assert!(looks_like_markup(b"\xEF\xBB\xBF<!doctype html>"));
        assert!(looks_like_markup(b"<?xml version=\"1.0\"?>"));
    }

    #[test]
    fn json_and_sse_bodies_are_not_markup() {
        assert!(!looks_like_markup(b"{\"id\":\"x\"}"));
        assert!(!looks_like_markup(b"data: {\"delta\":\"hi\"}"));
        assert!(!looks_like_markup(b""));
        assert!(!looks_like_markup(b"[1,2,3]"));
        // A comparison operator in a JSON string must not trip the detector.
        assert!(!looks_like_markup(b"{\"expr\":\"<html>\"}"));
    }
}
