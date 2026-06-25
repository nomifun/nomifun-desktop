//! Screen capture via xcap, downscaled and PNG/base64-encoded for the LLM.
//!
//! Coordinate systems: `Monitor::width()/height()` report the size enigo's
//! absolute mouse coordinates use (logical points on macOS, device pixels on
//! Windows/Linux), while `capture_image()` returns physical pixels (2x on
//! Retina). `CaptureGeometry` records both so clicks can be mapped back.

use base64::Engine as _;
use xcap::Monitor;

use nomi_types::tool::ToolImage;

use crate::permissions;
use crate::scale::fit_within;

/// Geometry of the most recent capture, used to map LLM (screenshot-pixel)
/// coordinates to absolute screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureGeometry {
    /// Dimensions of the image the LLM sees (after downscaling).
    pub img_w: u32,
    pub img_h: u32,
    /// Monitor size in the coordinate system input synthesis uses.
    pub logical_w: u32,
    pub logical_h: u32,
    /// Monitor origin in the global (multi-display) coordinate space.
    pub origin_x: i32,
    pub origin_y: i32,
}

/// A completed screen capture. `image` is the downscaled RGBA frame (not yet
/// encoded) so callers can draw a Set-of-Marks overlay before encoding; use
/// `encode_png` to produce the `ToolImage`.
#[derive(Debug)]
pub struct CapturedScreen {
    pub image: image::RgbaImage,
    pub geometry: CaptureGeometry,
    /// Raw captured frame size in physical pixels (before downscaling).
    pub physical_w: u32,
    pub physical_h: u32,
    /// Index of the captured monitor within `Monitor::all()`.
    pub display_index: usize,
}

/// Encode a (possibly overlay-annotated) RGBA frame as a base64 PNG `ToolImage`.
pub fn encode_png(img: &image::RgbaImage) -> Result<ToolImage, String> {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode screenshot as PNG: {e}"))?;
    Ok(ToolImage {
        media_type: "image/png".to_string(),
        data: base64::engine::general_purpose::STANDARD.encode(&png),
    })
}

/// Pick the monitor to capture: explicit index, else the primary, else the
/// first one listed.
fn select_monitor(monitors: &[Monitor], display: Option<usize>) -> Result<usize, String> {
    if monitors.is_empty() {
        return Err(format!(
            "No displays found. {}",
            permissions::screen_capture_hint_detailed()
        ));
    }
    match display {
        Some(idx) => {
            if idx < monitors.len() {
                Ok(idx)
            } else {
                Err(format!(
                    "Display {idx} does not exist; {} display(s) available (0-{}).",
                    monitors.len(),
                    monitors.len() - 1
                ))
            }
        }
        None => Ok(monitors
            .iter()
            .position(|m| m.is_primary().unwrap_or(false))
            .unwrap_or(0)),
    }
}

/// Capture a monitor, downscale to `max_edge`, and encode as base64 PNG.
/// Blocking: call from `spawn_blocking`.
pub fn capture_screen(display: Option<usize>, max_edge: u32) -> Result<CapturedScreen, String> {
    // Proactive + authoritative on macOS: a denied Screen Recording grant lets
    // capture "succeed" with a black frame, so fail fast with a clear message
    // instead of relying solely on the downstream black-frame heuristic.
    if permissions::screen_recording_granted() == Some(false) {
        return Err(format!(
            "Screen Recording permission is not granted, so the screen cannot be captured. {}",
            permissions::screen_capture_hint_detailed()
        ));
    }
    let monitors = Monitor::all().map_err(|e| {
        format!(
            "Failed to enumerate displays: {e}. {}",
            permissions::screen_capture_hint_detailed()
        )
    })?;
    let display_index = select_monitor(&monitors, display)?;
    let monitor = &monitors[display_index];

    let frame = monitor.capture_image().map_err(|e| {
        format!(
            "Failed to capture the screen: {e}. {}",
            permissions::screen_capture_hint_detailed()
        )
    })?;
    permissions::screenshot_permission_check(&frame)?;

    let (physical_w, physical_h) = frame.dimensions();
    if physical_w == 0 || physical_h == 0 {
        return Err(format!(
            "Capture returned an empty frame. {}",
            permissions::screen_capture_hint_detailed()
        ));
    }

    let (img_w, img_h) = fit_within(physical_w, physical_h, max_edge);
    let scaled = if (img_w, img_h) == (physical_w, physical_h) {
        frame
    } else {
        image::imageops::resize(&frame, img_w, img_h, image::imageops::FilterType::Triangle)
    };

    let logical_w = monitor.width().unwrap_or(physical_w);
    let logical_h = monitor.height().unwrap_or(physical_h);
    let origin_x = monitor.x().unwrap_or(0);
    let origin_y = monitor.y().unwrap_or(0);

    Ok(CapturedScreen {
        image: scaled,
        geometry: CaptureGeometry {
            img_w,
            img_h,
            logical_w,
            logical_h,
            origin_x,
            origin_y,
        },
        physical_w,
        physical_h,
        display_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Requires a real display and (on macOS) Screen Recording permission.
    #[test]
    #[ignore]
    fn capture_primary_screen_real() {
        let captured = capture_screen(None, 1568).expect("capture should succeed");
        assert!(captured.image.width() > 0 && captured.image.height() > 0);
        let encoded = encode_png(&captured.image).expect("encode should succeed");
        assert_eq!(encoded.media_type, "image/png");
        assert!(!encoded.data.is_empty());
        assert!(captured.geometry.img_w.max(captured.geometry.img_h) <= 1568);
        assert!(captured.physical_w >= captured.geometry.img_w);
    }

    #[test]
    #[ignore]
    fn capture_invalid_display_errors_real() {
        let err = capture_screen(Some(99), 1568).unwrap_err();
        assert!(err.contains("99"), "error should name the display: {err}");
    }
}
