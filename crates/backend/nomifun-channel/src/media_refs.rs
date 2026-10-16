//! Creative Studio asset references in public assistant text. Tool receipts
//! carry typed result_asset_ids and never require parsing raw tool output.

use regex::Regex;
use std::sync::OnceLock;

/// Matches a Creative Studio file URL and captures the asset id, e.g.
/// `/api/creative-studio/files/{uuidv7}` (host optional, `?thumb=1` tolerated).
fn files_url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"/api/creative-studio/files/([0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})",
        )
        .unwrap()
    })
}

/// Extract asset ids from assistant text by matching Creative Studio file URLs.
pub fn asset_ids_from_text(text: &str) -> Vec<String> {
    files_url_re()
        .captures_iter(text)
        .map(|c| c[1].to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_extracts_capability_urls() {
        let text = "图来咯～ ![cat](/api/creative-studio/files/0190f5fe-7c00-7a00-8000-000000000081) and http://127.0.0.1:8080/api/creative-studio/files/0190f5fe-7c00-7a00-8000-000000000082?thumb=1";
        assert_eq!(
            asset_ids_from_text(text),
            vec![
                "0190f5fe-7c00-7a00-8000-000000000081",
                "0190f5fe-7c00-7a00-8000-000000000082"
            ]
        );
    }

    #[test]
    fn text_without_urls_is_empty() {
        assert!(asset_ids_from_text("just text, no image").is_empty());
    }
}
