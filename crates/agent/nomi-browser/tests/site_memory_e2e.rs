//! **P7A: Site-memory real-Chrome smoke test** (`#[ignore]`, requires NOMIFUN_CHROME_BINARY).
//!
//! Navigates to `https://example.com`, hovers an element (non-navigating action),
//! verifies site memory records the element, then observes again to confirm hints
//! are attached.
//!
//! Run:
//!   export NOMIFUN_CHROME_BINARY="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
//!   cargo nextest run -p nomi-browser --run-ignored all -E 'test(site_memory_real_chrome)'

use std::sync::Arc;

use nomi_browser::site_memory::{InMemorySink, SiteMemoryStore};
use nomi_browser::BrowserTool;
use nomi_tools::Tool;
use serde_json::json;

fn isolated_data_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("nomifun-p7a-site-memory-smoke")
}

/// **Real-Chrome smoke**: navigate example.com, hover a heading (non-navigating),
/// verify site memory records the element; then observe again and verify hints appear.
#[tokio::test]
#[ignore = "requires NOMIFUN_CHROME_BINARY + network access to example.com"]
async fn site_memory_real_chrome_remember_across_navigations() {
    let sink = InMemorySink::new();
    let store = Arc::new(SiteMemoryStore::new(Box::new(sink)));
    let tool = BrowserTool::with_data_dir(isolated_data_dir(), false)
        .with_site_memory(store.clone());

    // ── 1. Navigate to example.com ────────────────────────────────────────────
    let nav = tool
        .execute(json!({"action": "navigate", "url": "https://example.com"}))
        .await;
    assert!(!nav.is_error, "navigate should succeed: {}", nav.content);

    // ── 2. Observe: get the page structure ────────────────────────────────────
    let obs1 = tool.execute(json!({"action": "observe"})).await;
    assert!(!obs1.is_error, "observe should succeed: {}", obs1.content);
    let obs_text = &obs1.content;

    // Find a heading ref ("Example Domain") — hover it (non-navigating action).
    let heading_ref = obs_text
        .lines()
        .find(|line| line.contains("heading") && line.contains("Example Domain") && line.contains("[ref="))
        .and_then(|line| {
            let start = line.find("[ref=")? + 5;
            let end = line[start..].find(']')? + start;
            Some(line[start..end].to_string())
        })
        .expect("should find a ref for the 'Example Domain' heading");

    // ── 3. Hover the heading → triggers site-memory recording ─────────────────
    let hover = tool
        .execute(json!({"action": "hover", "ref": heading_ref}))
        .await;
    assert!(!hover.is_error, "hover should succeed: {}", hover.content);

    // ── 4. Verify site memory recorded the hover ──────────────────────────────
    let hints = store.query("example.com");
    assert!(
        !hints.is_empty(),
        "site memory should have recorded at least one entry for example.com"
    );
    assert!(
        hints.iter().any(|h| h.accessible_name.contains("Example Domain")),
        "site memory should remember the 'Example Domain' heading; got: {hints:?}"
    );

    // ── 5. Observe again — hints should appear in the output ──────────────────
    let obs2 = tool.execute(json!({"action": "observe"})).await;
    assert!(!obs2.is_error, "2nd observe should succeed: {}", obs2.content);

    // The 2nd observe should include site-memory hints.
    let obs2_text = &obs2.content;
    assert!(
        obs2_text.contains("site-memory-hints"),
        "2nd observe should include site-memory hints; got:\n{obs2_text}"
    );
    assert!(
        obs2_text.contains("Example Domain"),
        "hints should mention the remembered 'Example Domain' heading"
    );
}
