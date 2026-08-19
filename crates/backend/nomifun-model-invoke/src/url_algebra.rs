//! The single URL algebra shared by every lane that turns a configured
//! connection root plus a protocol endpoint template into a request URL.
//!
//! Before this module existed there were three independent implementations of
//! "append a path to a base URL" (the invoke runtime, save-time validation, and
//! the catalog fetcher's auto-fix) plus two contradictory `/v1` policies. None
//! of them knew that the manifest ships endpoint templates under two mutually
//! exclusive conventions:
//!
//! - `openai.chat_text` submits to `/chat/completions`, so the version segment
//!   must already live in the connection root (`https://host/v1`);
//! - `anthropic.messages` submits to `/v1/messages` and `gemini` to
//!   `/v1beta/...`, so the root must *not* carry a version.
//!
//! A user configuring an OpenAI-compatible gateway is offered exactly those
//! protocols and was never told which convention applied, so pairing a root of
//! `https://host/v1` with a documented path of `/v1/chat/completions` produced
//! `https://host/v1/v1/chat/completions` — a 404. [`join_endpoint`] collapses
//! that seam, [`EndpointRootShape`] lets the manifest state the convention, and
//! [`root_candidates`] probes roots without ever manufacturing a doubled
//! version segment.

use nomifun_api_types::EndpointRootShape;

/// Root suffixes worth probing when a configured root does not answer. This is
/// the one vocabulary; the catalog auto-fixer consumes it rather than keeping a
/// private copy that could drift.
pub const ROOT_VERSION_SUFFIXES: &[&str] = &[
    "/v1",
    "/api/v1",
    "/openai/v1",
    "/compatible-mode/v1",
    "/v2",
    "/api/v3",
    "/api/paas/v4",
    "/compatibility/v1",
];

/// Does this single path segment name an API version?
///
/// Matches `v1`, `v2`, `v4`, and the suffixed forms Google uses (`v1beta`,
/// `v1alpha1`). Matching is ASCII case-insensitive so an uppercase `/V1` still
/// de-duplicates instead of silently doubling.
pub fn is_version_segment(segment: &str) -> bool {
    let Some(rest) = segment.strip_prefix('v').or_else(|| segment.strip_prefix('V')) else {
        return false;
    };
    // A version is `v` followed by a number, optionally with a channel suffix:
    // v1, v4, v1beta, v1alpha1. Anything non-alphanumeric disqualifies it.
    rest.starts_with(|ch: char| ch.is_ascii_digit())
        && rest.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn path_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|segment| !segment.is_empty()).collect()
}

/// The path segments of an absolute connection root, or `None` when the value
/// is not a parseable absolute URL.
fn root_path_segments(base_url: &str) -> Option<Vec<String>> {
    let parsed = reqwest::Url::parse(base_url.trim()).ok()?;
    Some(path_segments(parsed.path()).into_iter().map(str::to_owned).collect())
}

/// Does any path segment of this root name an API version?
pub fn root_declares_version(base_url: &str) -> bool {
    root_path_segments(base_url)
        .unwrap_or_default()
        .iter()
        .any(|segment| is_version_segment(segment))
}

/// Is this root shaped the way the protocol's endpoint template expects?
pub fn root_matches_shape(base_url: &str, shape: EndpointRootShape) -> bool {
    match shape {
        EndpointRootShape::VersionedRoot => root_declares_version(base_url),
        EndpointRootShape::OriginRoot => !root_declares_version(base_url),
    }
}

/// Split an endpoint template into its path and its query/fragment tail. The
/// tail is preserved byte-for-byte: `?alt=sse` and `?model={model}` are part of
/// the protocol contract.
fn split_template(endpoint: &str) -> (&str, &str) {
    match endpoint.find(['?', '#']) {
        Some(index) => endpoint.split_at(index),
        None => (endpoint, ""),
    }
}

/// How many leading template segments duplicate the tail of the root?
///
/// Only an overlap that contains a version segment is treated as duplication.
/// Without that guard a root of `.../videos` plus a template of `/videos/{id}`
/// would collapse into a wrong URL; with it, only the version seam — the one
/// place two conventions actually collide — is de-duplicated.
fn duplicated_prefix_len(root: &[String], template: &[&str]) -> usize {
    let max = root.len().min(template.len());
    for take in (1..=max).rev() {
        let root_tail = &root[root.len() - take..];
        let matches = root_tail
            .iter()
            .zip(template.iter())
            .all(|(left, right)| left.as_str().eq_ignore_ascii_case(right));
        if matches && root_tail.iter().any(|segment| is_version_segment(segment)) {
            return take;
        }
    }
    0
}

/// Join a connection root and an endpoint template into one request URL.
///
/// An endpoint that is already an absolute URL wins verbatim — that is the
/// escape hatch for a provider whose real path genuinely repeats a version
/// segment. Otherwise the template is appended to the root, collapsing a
/// duplicated version seam exactly once.
pub fn join_endpoint(base_url: &str, endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if reqwest::Url::parse(endpoint).is_ok() {
        return endpoint.to_owned();
    }
    let root = base_url.trim().trim_end_matches('/');
    let (template_path, tail) = split_template(endpoint);
    let template_segments = path_segments(template_path);

    let drop = match root_path_segments(root) {
        Some(root_segments) if !root_segments.is_empty() => {
            duplicated_prefix_len(&root_segments, &template_segments)
        }
        // A root we cannot parse gets the old, purely textual behavior rather
        // than a guess about where its path begins.
        _ => 0,
    };

    let remaining = &template_segments[drop..];
    if remaining.is_empty() {
        return format!("{root}{tail}");
    }
    format!("{root}/{}{tail}", remaining.join("/"))
}

/// Strip the longest known version suffix from a root, yielding the bare root.
fn strip_version_suffix(root: &str) -> &str {
    let mut best = root;
    for suffix in ROOT_VERSION_SUFFIXES {
        if let Some(stripped) = root.strip_suffix(suffix) {
            // Longest match wins so `/api/paas/v4` is not left as `/api/paas`.
            if stripped.len() < best.len() {
                best = stripped;
            }
        }
    }
    best.trim_end_matches('/')
}

/// Deterministic, de-duplicated probe roots, most-likely first.
///
/// The configured root is always tried first, then the bare root, then the bare
/// root plus each known version suffix. Because suffixes are appended to the
/// *bare* root, this can never emit `.../v1/v1` — the shape that produced the
/// reported 404 when the previous implementation appended unconditionally.
pub fn root_candidates(base_url: &str) -> Vec<String> {
    let configured = base_url.trim().trim_end_matches('/');
    let bare = strip_version_suffix(configured);

    let mut candidates: Vec<String> = Vec::with_capacity(ROOT_VERSION_SUFFIXES.len() + 2);
    let mut push = |value: String| {
        if !value.is_empty() && !candidates.contains(&value) {
            candidates.push(value);
        }
    };
    push(configured.to_owned());
    push(bare.to_owned());
    for suffix in ROOT_VERSION_SUFFIXES {
        push(format!("{bare}{suffix}"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_segments_are_recognized() {
        for value in ["v1", "v2", "v4", "v1beta", "v1alpha1", "V1", "v10"] {
            assert!(is_version_segment(value), "{value} should be a version");
        }
        for value in [
            "", "v", "api", "videos", "models", "paas", "chat", "1", "vbeta", "v1-beta",
        ] {
            assert!(!is_version_segment(value), "{value} must not be a version");
        }
    }

    #[test]
    fn doubled_version_seam_collapses_once() {
        assert_eq!(
            join_endpoint("https://www.cheapapi.xin/v1", "/v1/chat/completions"),
            "https://www.cheapapi.xin/v1/chat/completions"
        );
        assert_eq!(
            join_endpoint("https://api.anthropic.com/v1", "/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            join_endpoint("https://ark.example.com/api/v3", "/api/v3/contents/generations/tasks"),
            "https://ark.example.com/api/v3/contents/generations/tasks"
        );
    }

    #[test]
    fn version_free_template_is_appended_unchanged() {
        assert_eq!(
            join_endpoint("https://www.cheapapi.xin/v1", "/chat/completions"),
            "https://www.cheapapi.xin/v1/chat/completions"
        );
        assert_eq!(
            join_endpoint("https://open.bigmodel.cn/api/paas/v4", "/chat/completions"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
    }

    #[test]
    fn origin_root_template_keeps_its_version() {
        assert_eq!(
            join_endpoint("https://api.anthropic.com", "/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            join_endpoint(
                "https://generativelanguage.googleapis.com",
                "/v1beta/models/{model}:streamGenerateContent?alt=sse"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn overlap_without_a_version_segment_is_not_collapsed() {
        // A genuinely repeated non-version segment is a real path, not a seam.
        assert_eq!(
            join_endpoint("https://api.example.com/videos", "/videos/{id}"),
            "https://api.example.com/videos/videos/{id}"
        );
        // Distinct paths that merely both contain a version stay distinct.
        assert_eq!(
            join_endpoint(
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "/api/v1/services/embeddings/text-embedding/text-embedding"
            ),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/api/v1/services/embeddings/text-embedding/text-embedding"
        );
    }

    #[test]
    fn query_tail_is_preserved_and_slashes_normalized() {
        assert_eq!(
            join_endpoint("https://api.example.com/v1/", "chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            join_endpoint("https://api.example.com", "/realtime?model={model}"),
            "https://api.example.com/realtime?model={model}"
        );
        assert_eq!(
            join_endpoint("https://api.example.com/v1", "/v1?alt=sse"),
            "https://api.example.com/v1?alt=sse"
        );
    }

    #[test]
    fn absolute_endpoint_wins_verbatim() {
        assert_eq!(
            join_endpoint("https://api.example.com/v1", "https://other.example.com/v1/v1/odd"),
            "https://other.example.com/v1/v1/odd"
        );
    }

    #[test]
    fn root_shape_classifies_configured_roots() {
        assert!(root_matches_shape(
            "https://api.openai.com/v1",
            EndpointRootShape::VersionedRoot
        ));
        assert!(!root_matches_shape(
            "https://api.openai.com",
            EndpointRootShape::VersionedRoot
        ));
        assert!(root_matches_shape(
            "https://api.anthropic.com",
            EndpointRootShape::OriginRoot
        ));
        assert!(!root_matches_shape(
            "https://api.anthropic.com/v1",
            EndpointRootShape::OriginRoot
        ));
        // Version anywhere in the path counts, not just the final segment.
        assert!(root_matches_shape(
            "https://qianfan.baidubce.com/v2/coding",
            EndpointRootShape::VersionedRoot
        ));
    }

    #[test]
    fn candidates_lead_with_configured_then_bare_and_never_double_a_version() {
        let candidates = root_candidates("https://www.cheapapi.xin");
        assert_eq!(candidates[0], "https://www.cheapapi.xin");
        assert!(candidates.contains(&"https://www.cheapapi.xin/v1".to_owned()));

        let from_versioned = root_candidates("https://www.cheapapi.xin/v1");
        assert_eq!(from_versioned[0], "https://www.cheapapi.xin/v1");
        // The bare root must be probed — a user who already typed the correct
        // versioned root previously had no candidate that could confirm it.
        assert_eq!(from_versioned[1], "https://www.cheapapi.xin");
        for candidate in &from_versioned {
            assert!(
                !candidate.contains("/v1/v1"),
                "candidate must never double a version: {candidate}"
            );
        }
    }

    #[test]
    fn candidates_are_deduplicated_and_strip_the_longest_suffix() {
        let candidates = root_candidates("https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(candidates[1], "https://open.bigmodel.cn");
        let unique: std::collections::BTreeSet<_> = candidates.iter().collect();
        assert_eq!(unique.len(), candidates.len(), "candidates must be unique");
    }
}
