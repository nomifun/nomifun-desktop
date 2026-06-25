//! Tests for the visual fallback module (P7B).
//!
//! These are pure-logic tests that do NOT require a Chrome binary.

use nomi_browser::visual_fallback::{
    should_try_visual, som_overlay, to_css_point, ElementRect, PixelBox, VisualFallback,
    VisualLocateResult, VisualLocator,
};
use nomi_browser_engine::BrowserError;

/// **THE KEYSTONE TEST**: vision models return device/image pixels. The engine's input layer
/// is DPR-free (CSS pixels). The facade MUST divide by DPR before dispatching.
///
/// `to_css_point(200, 400, dpr=2.0)` => `(100.0, 200.0)` (divides by DPR).
/// `to_css_point(200, 400, dpr=1.0)` => `(200.0, 400.0)` (identity when DPR is 1).
#[test]
fn pixel_to_css_divides_by_dpr() {
    // DPR 2.0: Retina display — device pixels are 2x CSS pixels.
    let (cx, cy) = to_css_point(200.0, 400.0, 2.0);
    assert_eq!(cx, 100.0, "x must be divided by DPR");
    assert_eq!(cy, 200.0, "y must be divided by DPR");

    // DPR 1.0: identity — device pixels == CSS pixels.
    let (cx, cy) = to_css_point(200.0, 400.0, 1.0);
    assert_eq!(cx, 200.0, "dpr=1.0 must be identity for x");
    assert_eq!(cy, 400.0, "dpr=1.0 must be identity for y");

    // DPR 1.5: fractional scale factor.
    let (cx, cy) = to_css_point(300.0, 450.0, 1.5);
    assert_eq!(cx, 200.0, "x/1.5 = 200");
    assert_eq!(cy, 300.0, "y/1.5 = 300");
}

/// Visual fallback must ONLY be attempted when DOM/aria anchoring fails with
/// NodeStale or NotConnected. It must NOT run when `resolve_ref` succeeds, and
/// must NOT run on unrelated errors (timeout, session lost, blocked, etc.).
#[test]
fn fallback_only_invoked_on_anchor_failure() {
    // Anchor succeeded — never try visual.
    assert!(!should_try_visual(&Ok(())), "must NOT fallback on successful anchor");

    // NodeStale — ref from old generation, should try visual.
    assert!(
        should_try_visual(&Err(BrowserError::NodeStale { generation: 5 })),
        "must fallback on NodeStale"
    );

    // NotConnected — element detached from DOM, should try visual.
    assert!(
        should_try_visual(&Err(BrowserError::NotConnected)),
        "must fallback on NotConnected"
    );

    // SessionLost — NOT a visual-fallback candidate.
    assert!(
        !should_try_visual(&Err(BrowserError::SessionLost { recoverable: false })),
        "must NOT fallback on SessionLost"
    );

    // Timeout — NOT a visual-fallback candidate.
    assert!(
        !should_try_visual(&Err(BrowserError::Timeout {
            phase: nomi_browser_engine::NavPhase::Action
        })),
        "must NOT fallback on Timeout"
    );

    // Blocked — NOT a visual-fallback candidate.
    assert!(
        !should_try_visual(&Err(BrowserError::Blocked {
            reason: "denied".into()
        })),
        "must NOT fallback on Blocked"
    );

    // Other — NOT a visual-fallback candidate (generic errors are not anchor-specific).
    assert!(
        !should_try_visual(&Err(BrowserError::Other("something went wrong".into()))),
        "must NOT fallback on Other"
    );
}

/// A fake vision locator that returns a fixed pixel bounding box (simulating what a
/// real vision model would return after analyzing a screenshot).
struct FakeLocator {
    /// The pixel-space bounding box the fake "finds".
    pixel_box: PixelBox,
}

#[async_trait::async_trait]
impl VisualLocator for FakeLocator {
    async fn locate(
        &self,
        _screenshot: &[u8],
        _instruction: &str,
    ) -> Result<VisualLocateResult, String> {
        Ok(VisualLocateResult {
            pixel_box: self.pixel_box,
            confidence: 0.95,
        })
    }
}

/// VisualFallback::locate_and_target calls the locator with the redacted screenshot,
/// receives pixel coords, and maps them to CSS pixels via DPR division.
#[tokio::test]
async fn visual_fallback_locates_and_maps() {
    // Fake locator returns a box centered at (200, 400) in device pixels.
    let locator = FakeLocator {
        pixel_box: PixelBox {
            x: 180.0,
            y: 380.0,
            width: 40.0,
            height: 40.0,
        },
    };
    let fallback = VisualFallback::new(std::sync::Arc::new(locator));

    // DPR = 2.0 → center pixel (200, 400) → CSS (100, 200).
    let fake_screenshot = b"fake-png-data";
    let result = fallback
        .locate_and_target(fake_screenshot, "Click the Submit button", 2.0)
        .await
        .expect("locate_and_target should succeed with a fake locator");

    assert_eq!(result.x, 100.0, "CSS x = pixel_center_x / dpr = 200/2");
    assert_eq!(result.y, 200.0, "CSS y = pixel_center_y / dpr = 400/2");

    // DPR = 1.0 → identity.
    let result = fallback
        .locate_and_target(fake_screenshot, "Click the Submit button", 1.0)
        .await
        .expect("locate_and_target should succeed");

    assert_eq!(result.x, 200.0, "CSS x = pixel_center_x / 1.0 = 200");
    assert_eq!(result.y, 400.0, "CSS y = pixel_center_y / 1.0 = 400");
}

/// SoM overlay assigns deterministic 1..N labels to element rects, sorted by position
/// (top-to-bottom, left-to-right). The numbering is stable across repeated calls.
#[test]
fn som_overlay_numbers_boxes_stably() {
    let rects = vec![
        // Bottom-right element (should be numbered LAST due to sort order).
        ElementRect { x: 300.0, y: 200.0, width: 50.0, height: 30.0 },
        // Top-left element (should be numbered FIRST).
        ElementRect { x: 10.0, y: 10.0, width: 100.0, height: 40.0 },
        // Middle element (between top and bottom).
        ElementRect { x: 150.0, y: 100.0, width: 80.0, height: 30.0 },
        // Same y as first, but further right (should be numbered second).
        ElementRect { x: 200.0, y: 10.0, width: 60.0, height: 40.0 },
    ];

    let fake_png = b"fake-png-bytes";
    let result = som_overlay(fake_png, &rects);

    // Should have 4 labels.
    assert_eq!(result.label_map.len(), 4);

    // Label 1: top-left (y=10, x=10) — the topmost, leftmost.
    assert_eq!(result.label_map[0].number, 1);
    assert_eq!(result.label_map[0].rect.x, 10.0);
    assert_eq!(result.label_map[0].rect.y, 10.0);

    // Label 2: top-right (y=10, x=200) — same row as label 1, but further right.
    assert_eq!(result.label_map[1].number, 2);
    assert_eq!(result.label_map[1].rect.x, 200.0);
    assert_eq!(result.label_map[1].rect.y, 10.0);

    // Label 3: middle (y=100, x=150).
    assert_eq!(result.label_map[2].number, 3);
    assert_eq!(result.label_map[2].rect.x, 150.0);
    assert_eq!(result.label_map[2].rect.y, 100.0);

    // Label 4: bottom-right (y=200, x=300).
    assert_eq!(result.label_map[3].number, 4);
    assert_eq!(result.label_map[3].rect.x, 300.0);
    assert_eq!(result.label_map[3].rect.y, 200.0);

    // Stability: calling with the same rects produces the same numbering.
    let result2 = som_overlay(fake_png, &rects);
    assert_eq!(result.label_map, result2.label_map, "numbering must be deterministic");

    // Empty rects → empty label map.
    let empty_result = som_overlay(fake_png, &[]);
    assert!(empty_result.label_map.is_empty());

    // With invalid PNG bytes, annotated_png falls back to input unchanged.
    assert_eq!(result.annotated_png, fake_png.as_slice());
}

/// SoM overlay with a real PNG: annotated output must (a) decode as valid PNG,
/// (b) differ from input (proving drawing happened), (c) label_map is unchanged.
#[test]
fn som_overlay_draws_on_real_png() {
    use image::{ImageFormat, RgbaImage, Rgba};
    use std::io::Cursor;

    // Create a small 200×200 solid-gray PNG.
    let img = RgbaImage::from_pixel(200, 200, Rgba([128, 128, 128, 255]));
    let mut input_buf = Cursor::new(Vec::new());
    img.write_to(&mut input_buf, ImageFormat::Png).unwrap();
    let input_png = input_buf.into_inner();

    let rects = vec![
        ElementRect { x: 20.0, y: 50.0, width: 80.0, height: 40.0 },
        ElementRect { x: 10.0, y: 10.0, width: 60.0, height: 30.0 },
        ElementRect { x: 100.0, y: 120.0, width: 50.0, height: 25.0 },
    ];

    let result = som_overlay(&input_png, &rects);

    // (a) annotated_png is a valid PNG and decodes successfully.
    let decoded = image::load_from_memory_with_format(&result.annotated_png, ImageFormat::Png);
    assert!(decoded.is_ok(), "annotated_png must be a valid PNG");

    // (b) annotated_png DIFFERS from the input (drawing happened).
    assert_ne!(
        result.annotated_png, input_png,
        "annotated_png must differ from input (overlay was drawn)"
    );

    // (c) label_map numbering is correct and stable.
    assert_eq!(result.label_map.len(), 3);
    // Sorted by y then x: (10,10)=1, (20,50)=2, (100,120)=3
    assert_eq!(result.label_map[0].number, 1);
    assert_eq!(result.label_map[0].rect.x, 10.0);
    assert_eq!(result.label_map[0].rect.y, 10.0);
    assert_eq!(result.label_map[1].number, 2);
    assert_eq!(result.label_map[1].rect.x, 20.0);
    assert_eq!(result.label_map[1].rect.y, 50.0);
    assert_eq!(result.label_map[2].number, 3);
    assert_eq!(result.label_map[2].rect.x, 100.0);
    assert_eq!(result.label_map[2].rect.y, 120.0);

    // Verify output dimensions match input.
    let out_img = decoded.unwrap().to_rgba8();
    assert_eq!(out_img.dimensions(), (200, 200));
}

/// Edge case: rects that are partially or fully off-screen must not panic.
#[test]
fn som_overlay_clips_offscreen_rects() {
    use image::{ImageFormat, RgbaImage, Rgba};
    use std::io::Cursor;

    let img = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).unwrap();
    let input_png = buf.into_inner();

    let rects = vec![
        // Partially off-screen (extends beyond image bounds).
        ElementRect { x: 80.0, y: 80.0, width: 50.0, height: 50.0 },
        // Fully off-screen.
        ElementRect { x: 200.0, y: 200.0, width: 30.0, height: 30.0 },
        // Negative coords.
        ElementRect { x: -10.0, y: -10.0, width: 50.0, height: 50.0 },
        // Zero-size rect (degenerate).
        ElementRect { x: 50.0, y: 50.0, width: 0.0, height: 0.0 },
    ];

    // Must not panic.
    let result = som_overlay(&input_png, &rects);

    // All 4 rects get labels even if drawing is clipped.
    assert_eq!(result.label_map.len(), 4);
    // Output is a valid PNG.
    assert!(image::load_from_memory_with_format(&result.annotated_png, ImageFormat::Png).is_ok());
}
