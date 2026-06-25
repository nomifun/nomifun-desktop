//! **P7B: Visual Fallback** — last-resort vision-model-based element location when
//! DOM/aria anchoring fails (NodeStale / NotConnected / no match).
//!
//! # Architecture
//!
//! Visual fallback is a **last resort after** DOM/aria anchoring fails — never the primary
//! path. The engine stays LLM-free (no vision-model call in nomi-browser-engine). The
//! orchestration lives here in the facade (`nomi-browser`).
//!
//! # Coordinate Rule (THE KEYSTONE)
//!
//! Vision models return **image/device pixel** coordinates. The engine's input layer is
//! **DPR-free (CSS pixels)**. The facade MUST convert before dispatching:
//!
//! ```text
//! to_css_point(px, py, dpr) = (px / dpr, py / dpr)
//! ```
//!
//! This division is performed ONCE by the facade, immediately after receiving coordinates
//! from the vision locator, before any engine dispatch.

use std::io::Cursor;
use std::sync::Arc;

use image::{ImageFormat, Rgba, RgbaImage};
use nomi_browser_engine::BrowserError;

// ─── Types ──────────────────────────────────────────────────────────────────

/// A point in CSS pixel space (DPR-free), ready for engine dispatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CssPoint {
    pub x: f64,
    pub y: f64,
}

/// A bounding box in device/image pixel space (as returned by a vision model).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelBox {
    /// Top-left x in device pixels.
    pub x: f64,
    /// Top-left y in device pixels.
    pub y: f64,
    /// Width in device pixels.
    pub width: f64,
    /// Height in device pixels.
    pub height: f64,
}

impl PixelBox {
    /// Center point of this box in device pixels.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// Result from the vision locator: a pixel-space bounding box + confidence.
#[derive(Clone, Debug)]
pub struct VisualLocateResult {
    /// The detected element's bounding box in device/image pixels.
    pub pixel_box: PixelBox,
    /// Confidence score from the vision model (0.0..=1.0).
    pub confidence: f64,
}

/// **P7B SoM result**: which numbered label the vision model picked + its confidence.
/// `label` is a **1-based** index into a [`SomOverlayResult::label_map`] (matching the
/// numbers drawn on the annotated screenshot). The caller validates `1..=n_labels` and maps
/// the label back to its [`SomLabel::rect`] center for the click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SomLabelResult {
    /// 1-based label number the model chose (index into the label_map).
    pub label: usize,
    /// Confidence score from the vision model (0.0..=1.0).
    pub confidence: f64,
}

/// A rect for SoM overlay (in device pixel space, from observe element entries).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// SoM label map entry: label number → element rect.
#[derive(Clone, Debug, PartialEq)]
pub struct SomLabel {
    pub number: usize,
    pub rect: ElementRect,
}

// ─── Trait: VisualLocator ───────────────────────────────────────────────────

/// Trait for the vision model locator seam. The facade injects an implementation
/// that calls a vision model to locate an element by description in a screenshot.
///
/// Mirrors the `ExtractModel` injection pattern: `Option<Arc<dyn VisualLocator>>`,
/// default `None` → fallback Unavailable / graceful degradation.
///
/// # Security
///
/// The screenshot passed to `locate` / `locate_labeled` is the engine's native page screenshot.
/// Secret protection relies on the **browser rendering** password/secret inputs masked (e.g.
/// `type=password` shows dots) — there is no post-capture pixel blackout. This is the same
/// screenshot the regular `screenshot` action and the raw-bbox path use; the SoM overlay only
/// adds numbered boxes drawn from element geometry (no values/names reach the model via the
/// overlay). Callers MUST NOT feed a screenshot that renders secrets as plaintext.
///
/// # Implementation notes
///
/// - The real adapter is `SessionVisualLocator` in nomi-agent's bootstrap: it reuses the
///   session `LlmProvider` (NOT nomi-computer — nomi-browser stays free of that dep) and sends
///   the screenshot as a `ContentBlock::Image`. It implements both `locate` (bbox) and
///   `locate_labeled` (SoM).
/// - For tests, a fake locator returns predetermined boxes / labels.
#[async_trait::async_trait]
pub trait VisualLocator: Send + Sync {
    /// Locate an element matching `instruction` in the given `screenshot` (PNG bytes).
    ///
    /// Returns the element's bounding box in device/image pixel space, or an error
    /// string if the element cannot be found.
    async fn locate(
        &self,
        screenshot: &[u8],
        instruction: &str,
    ) -> Result<VisualLocateResult, String>;

    /// **P7B SoM mode**: given a screenshot that already has numbered labels drawn on its
    /// clickable elements (a Set-of-Marks overlay) plus how many labels exist (`n_labels`),
    /// return which label number matches `instruction`. Returns a 1-based label the caller
    /// maps back to a known rect — far more reliable than free-form pixel regression when the
    /// candidate set is finite and visible.
    ///
    /// Default impl returns `Err` (not implemented) so existing locators (fakes/tests) keep
    /// compiling unchanged; only the real session adapter overrides it. The annotated
    /// screenshot must NEVER be passed to [`Self::locate`] (the overlay would confuse the
    /// bbox path) — these are deliberately separate methods.
    async fn locate_labeled(
        &self,
        annotated_screenshot: &[u8],
        instruction: &str,
        n_labels: usize,
    ) -> Result<SomLabelResult, String> {
        let _ = (annotated_screenshot, instruction, n_labels);
        Err("SoM label locator not implemented by this VisualLocator".to_string())
    }
}

/// Type alias for the optional locator injection (mirrors `ExtractModelRef`).
pub type VisualLocatorRef = Option<Arc<dyn VisualLocator>>;

// ─── Coordinate Mapping ─────────────────────────────────────────────────────

/// Convert device/image pixel coordinates (from a vision model) to CSS pixels
/// (the engine's DPR-free input coordinate space).
///
/// # Why
///
/// Screenshots are captured at device pixel resolution (e.g. 2x on Retina).
/// Vision models return coordinates in that pixel space. But the browser engine's
/// entire input path operates in CSS pixels (zero DPR) — so we MUST divide by
/// `devicePixelRatio` before dispatching any click/move to the engine.
///
/// # Panics
///
/// Does not panic. If `dpr` is zero or negative, returns `(0.0, 0.0)` (defensive).
pub fn to_css_point(px: f64, py: f64, dpr: f64) -> (f64, f64) {
    if dpr <= 0.0 {
        return (0.0, 0.0);
    }
    (px / dpr, py / dpr)
}

// ─── Fallback Gating ────────────────────────────────────────────────────────

/// Determine whether visual fallback should be attempted based on the anchor
/// resolution result from the engine.
///
/// Returns `true` ONLY when the anchor failed with:
/// - `NodeStale` — ref not in current generation (needs re-observe or visual).
/// - `NotConnected` — element detached from live DOM.
///
/// Returns `false` for all other errors (session lost, timeout, blocked, etc.)
/// and obviously for successful resolution (`Ok`).
pub fn should_try_visual(anchor_result: &Result<(), BrowserError>) -> bool {
    match anchor_result {
        Ok(()) => false, // Anchor succeeded — never run visual.
        Err(BrowserError::NodeStale { .. }) => true,
        Err(BrowserError::NotConnected) => true,
        // Catch-all: any other error type is NOT a visual-fallback candidate.
        _ => false,
    }
}

// ─── VisualFallback Orchestrator ────────────────────────────────────────────

/// The visual fallback orchestrator. Takes a failed-anchor context + a screenshot,
/// calls the vision locator, maps pixel→CSS coords, returns a target point for
/// engine dispatch.
pub struct VisualFallback {
    locator: Arc<dyn VisualLocator>,
}

impl VisualFallback {
    /// Create a new `VisualFallback` orchestrator with the given locator.
    pub fn new(locator: Arc<dyn VisualLocator>) -> Self {
        Self { locator }
    }

    /// Locate the target element visually and return its CSS-pixel click point.
    ///
    /// # Arguments
    ///
    /// - `redacted_screenshot`: PNG bytes of the current page screenshot, already
    ///   with password/secret regions blacked out (the same redaction observe uses).
    ///   **SECURITY**: this must be the redacted screenshot — raw screenshots must
    ///   never reach the vision model.
    /// - `instruction`: a natural-language description of what to locate (e.g.
    ///   "the Submit button", "the search input field").
    /// - `dpr`: the page's `devicePixelRatio` — used to convert from screenshot
    ///   pixel coordinates to CSS pixels.
    ///
    /// # Returns
    ///
    /// A `CssPoint` ready for engine dispatch via `click_at(Point { x, y })`.
    pub async fn locate_and_target(
        &self,
        redacted_screenshot: &[u8],
        instruction: &str,
        dpr: f64,
    ) -> Result<CssPoint, String> {
        let result = self.locator.locate(redacted_screenshot, instruction).await?;

        // Get the center of the detected bounding box in device pixels.
        let (center_px, center_py) = result.pixel_box.center();

        // THE KEYSTONE: convert from device pixels to CSS pixels.
        let (css_x, css_y) = to_css_point(center_px, center_py, dpr);

        Ok(CssPoint { x: css_x, y: css_y })
    }
}

// ─── SoM Overlay ────────────────────────────────────────────────────────────

/// Result of SoM overlay annotation.
#[derive(Clone, Debug)]
pub struct SomOverlayResult {
    /// The annotated PNG bytes (boxes + numbers drawn on the screenshot).
    pub annotated_png: Vec<u8>,
    /// Deterministic label map: 1-based number → element rect. Ordered by
    /// position (top-to-bottom, left-to-right) for stability.
    pub label_map: Vec<SomLabel>,
}

// ─── Bitmap Digit Font (3×5, embedded) ─────────────────────────────────────

/// Distinct, high-contrast mark colors cycled by label so neighbors differ.
const SOM_PALETTE: [[u8; 3]; 6] = [
    [255, 59, 48],  // red
    [0, 122, 255],  // blue
    [52, 199, 89],  // green
    [255, 149, 0],  // orange
    [175, 82, 222], // purple
    [255, 45, 85],  // pink
];

/// 3×5 bitmap font, digits 0-9. Each row's low 3 bits are pixels (MSB = left).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

const SOM_SCALE: i64 = 3; // pixels per font cell
const SOM_DIGIT_W: i64 = 3 * SOM_SCALE;
const SOM_DIGIT_H: i64 = 5 * SOM_SCALE;
const SOM_GAP: i64 = SOM_SCALE;
const SOM_PAD: i64 = SOM_SCALE;

// ─── Drawing Helpers ────────────────────────────────────────────────────────

fn som_put(img: &mut RgbaImage, x: i64, y: i64, c: [u8; 3], iw: u32, ih: u32) {
    if x < 0 || y < 0 || x >= iw as i64 || y >= ih as i64 {
        return;
    }
    img.put_pixel(x as u32, y as u32, Rgba([c[0], c[1], c[2], 255]));
}

fn som_fill_rect(
    img: &mut RgbaImage,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    c: [u8; 3],
    iw: u32,
    ih: u32,
) {
    for dy in 0..h {
        for dx in 0..w {
            som_put(img, x + dx, y + dy, c, iw, ih);
        }
    }
}

fn som_draw_rect_border(
    img: &mut RgbaImage,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    c: [u8; 3],
    thickness: i64,
    iw: u32,
    ih: u32,
) {
    for k in 0..thickness {
        // top / bottom
        for dx in 0..w {
            som_put(img, x + dx, y + k, c, iw, ih);
            som_put(img, x + dx, y + h - 1 - k, c, iw, ih);
        }
        // left / right
        for dy in 0..h {
            som_put(img, x + k, y + dy, c, iw, ih);
            som_put(img, x + w - 1 - k, y + dy, c, iw, ih);
        }
    }
}

fn som_label_size(n: usize) -> (i64, i64) {
    let digits = n.max(1).to_string().len() as i64;
    let w = SOM_PAD * 2 + digits * SOM_DIGIT_W + (digits - 1) * SOM_GAP;
    let h = SOM_PAD * 2 + SOM_DIGIT_H;
    (w, h)
}

fn som_draw_label(
    img: &mut RgbaImage,
    ex: i64,
    ey: i64,
    n: usize,
    bg: [u8; 3],
    iw: u32,
    ih: u32,
) {
    let (lw, lh) = som_label_size(n);
    // Prefer just above the element's top-left; if no room, place inside.
    let lx = ex.max(0);
    let ly = if ey - lh >= 0 { ey - lh } else { ey };
    som_fill_rect(img, lx, ly, lw, lh, bg, iw, ih);

    let fg = [255u8, 255, 255]; // white digits on the colored chip
    let mut cx = lx + SOM_PAD;
    let cy = ly + SOM_PAD;
    for ch in n.to_string().chars() {
        let d = ch.to_digit(10).unwrap_or(0) as usize;
        som_draw_digit(img, cx, cy, DIGITS[d], fg, iw, ih);
        cx += SOM_DIGIT_W + SOM_GAP;
    }
}

fn som_draw_digit(
    img: &mut RgbaImage,
    x: i64,
    y: i64,
    glyph: [u8; 5],
    c: [u8; 3],
    iw: u32,
    ih: u32,
) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..3i64 {
            if bits & (1 << (2 - col)) != 0 {
                som_fill_rect(
                    img,
                    x + col * SOM_SCALE,
                    y + row as i64 * SOM_SCALE,
                    SOM_SCALE,
                    SOM_SCALE,
                    c,
                    iw,
                    ih,
                );
            }
        }
    }
}

// ─── SoM Core ───────────────────────────────────────────────────────────────

/// Generate a Set-of-Mark (SoM) overlay on a screenshot: number each element
/// rect deterministically 1..N and return the label map.
///
/// # Ordering
///
/// Elements are sorted by position: primary sort by `y` (top-to-bottom), secondary
/// by `x` (left-to-right). This gives deterministic, stable numbering across runs.
///
/// # PNG Annotation
///
/// Draws colored rectangle borders and numbered labels on the screenshot using an
/// embedded 3×5 bitmap digit font. If the input cannot be decoded as a valid PNG,
/// falls back to returning the original bytes unchanged (best-effort — never panic).
///
/// # Invariant
///
/// SoM overlay must NOT mutate page DOM. It is drawn on the captured PNG (server-side)
/// so it never leaks into a subsequent observe.
pub fn som_overlay(png: &[u8], rects: &[ElementRect]) -> SomOverlayResult {
    // Sort rects by position: top-to-bottom, then left-to-right.
    let mut indexed: Vec<(usize, &ElementRect)> = rects.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        let y_cmp = a.1.y.partial_cmp(&b.1.y).unwrap_or(std::cmp::Ordering::Equal);
        if y_cmp == std::cmp::Ordering::Equal {
            a.1.x.partial_cmp(&b.1.x).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            y_cmp
        }
    });

    // Assign deterministic 1-based labels.
    let label_map: Vec<SomLabel> = indexed
        .iter()
        .enumerate()
        .map(|(label_idx, (_orig_idx, rect))| SomLabel {
            number: label_idx + 1,
            rect: **rect,
        })
        .collect();

    // Draw annotations on the PNG (best-effort: fall back to original on decode failure).
    let annotated_png = som_draw_annotations(png, &label_map);

    SomOverlayResult {
        annotated_png,
        label_map,
    }
}

/// Draw SoM annotations onto the PNG. Returns original bytes on decode failure.
fn som_draw_annotations(png: &[u8], labels: &[SomLabel]) -> Vec<u8> {
    // Decode input PNG; fall back gracefully if invalid.
    let dyn_img = match image::load_from_memory_with_format(png, ImageFormat::Png) {
        Ok(img) => img,
        Err(_) => return png.to_vec(),
    };
    let mut img = dyn_img.to_rgba8();
    let (iw, ih) = img.dimensions();

    if labels.is_empty() {
        // Nothing to draw — return original unchanged.
        return png.to_vec();
    }

    for label in labels {
        let color = SOM_PALETTE[(label.number - 1) % SOM_PALETTE.len()];
        let x = label.rect.x.round() as i64;
        let y = label.rect.y.round() as i64;
        let w = label.rect.width.round() as i64;
        let h = label.rect.height.round() as i64;

        // Skip degenerate rects.
        if w <= 0 || h <= 0 {
            continue;
        }

        som_draw_rect_border(&mut img, x, y, w, h, color, 2, iw, ih);
        som_draw_label(&mut img, x, y, label.number, color, iw, ih);
    }

    // Re-encode to PNG.
    let mut buf = Cursor::new(Vec::new());
    match img.write_to(&mut buf, ImageFormat::Png) {
        Ok(()) => buf.into_inner(),
        Err(_) => png.to_vec(), // Defensive: should never happen, but don't panic.
    }
}


