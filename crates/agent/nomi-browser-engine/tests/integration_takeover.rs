//! Task 3: `bring_to_front` engine seam — `#[ignore]` real-Chrome test (headful).
//!
//! Verifies that `CdpBackend::bring_to_front()` successfully sends `Page.bringToFront`
//! + `Target.activateTarget` when the engine is headful. Also verifies that a headless
//! engine returns `BrowserError::Unsupported`.
//!
//! Manual run (requires a display + Chrome):
//!   NOMIFUN_CHROME_BINARY="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//!     cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(bring_)'

mod common;

use nomi_browser_engine::BrowserEngine;

/// Headful engine: `bring_to_front` succeeds (no error).
#[tokio::test]
#[ignore = "requires NOMIFUN_CHROME_BINARY + display (headful)"]
async fn bring_window_to_front_succeeds_headful() {
    let backend = common::build_backend_for_fixture_headful("bring-front").await;
    // Navigate to a simple page so there's something to foreground.
    let _nav = backend
        .navigate(&common::fixture_url("act-c1.html"), false)
        .await
        .expect("navigate");
    // bring_to_front should succeed on a headful engine.
    backend
        .bring_to_front()
        .await
        .expect("bring_to_front on headful engine must succeed");
}

/// Headless engine: `bring_to_front` returns `Unsupported` gracefully.
#[tokio::test]
#[ignore = "requires NOMIFUN_CHROME_BINARY (headless, no display needed)"]
async fn bring_to_front_headless_returns_unsupported() {
    let backend = common::build_backend_for_fixture("bring-front-headless").await;
    let result = backend.bring_to_front().await;
    assert!(
        matches!(
            result,
            Err(nomi_browser_engine::BrowserError::Unsupported { .. })
        ),
        "headless bring_to_front must return Unsupported, got {result:?}"
    );
}
