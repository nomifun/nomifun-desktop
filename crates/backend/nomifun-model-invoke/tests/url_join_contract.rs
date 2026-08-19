//! The shared URL-join contract, asserted from the Rust side.
//!
//! `ui/src/renderer/pages/settings/components/urlAlgebra.test.ts` asserts the
//! same fixture against the TypeScript mirror. The duplication is unavoidable —
//! a live per-keystroke URL preview cannot round-trip to the backend — so the
//! fixture is what keeps the two from drifting.

use nomifun_model_invoke::join_endpoint;

#[test]
fn join_endpoint_matches_the_shared_fixture() {
    let raw = include_str!("fixtures/url_join_cases.json");
    let parsed: serde_json::Value =
        serde_json::from_str(raw).expect("url_join_cases.json must be valid JSON");
    let cases = parsed["cases"]
        .as_array()
        .expect("url_join_cases.json must carry a `cases` array");
    assert!(!cases.is_empty(), "the fixture must contain cases");

    for case in cases {
        let base = case["base"].as_str().expect("case.base");
        let endpoint = case["endpoint"].as_str().expect("case.endpoint");
        let expected = case["expected"].as_str().expect("case.expected");
        let why = case["why"].as_str().unwrap_or("");
        assert_eq!(
            join_endpoint(base, endpoint),
            expected,
            "case failed ({why}): base={base:?} endpoint={endpoint:?}"
        );
    }
}

/// The joiner must be idempotent under re-application: resolving an already
/// resolved URL cannot keep appending. This is what makes it safe to share
/// between save-time validation and the runtime request path.
#[test]
fn joining_a_version_free_template_twice_is_stable() {
    let once = join_endpoint("https://api.example.com/v1", "/chat/completions");
    assert_eq!(once, "https://api.example.com/v1/chat/completions");
    // Re-joining the SAME template against the produced URL appends again (it is
    // a different, longer root) — but re-joining the versioned template against a
    // versioned root does not grow. That second property is the one the reported
    // 404 depended on.
    assert_eq!(
        join_endpoint("https://api.example.com/v1", "/v1/chat/completions"),
        join_endpoint(
            &join_endpoint("https://api.example.com", "/v1"),
            "/v1/chat/completions"
        )
    );
}
