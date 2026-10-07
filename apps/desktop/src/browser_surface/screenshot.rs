//! Explicit, bounded PNG viewport capture. No continuous frame stream.
use super::native::{self, View};
use base64::{Engine, engine::general_purpose::STANDARD};
use nomifun_browser_platform::runtime::{BrowserScreenshot, BrowserTabTarget, WorkspaceError};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

const MAX_EDGE: u32 = 1600;
const MAX_ENCODED: usize = 3 * 1024 * 1024;

/// Rendering and native HWND visibility are separate. A hidden child may need
/// its compositor running to produce pixels; never show or focus the HWND.
#[cfg(windows)]
struct RenderLease {
    view: View,
    restore_hidden: bool,
    rendering: Arc<AtomicBool>,
}
#[cfg(windows)]
impl Drop for RenderLease {
    fn drop(&mut self) {
        self.rendering.store(false, Ordering::Release);
        if self.restore_hidden {
            let _ = self.view.with_webview(|platform| {
                let _ = unsafe { platform.controller().SetIsVisible(false) };
            });
        }
    }
}
#[cfg(windows)]
impl RenderLease {
    async fn start(
        view: &View,
        rendering: Arc<AtomicBool>,
    ) -> Result<Self, WorkspaceError> {
        let mut lease = Self {
            view: view.clone(),
            restore_hidden: false,
            rendering,
        };
        lease.rendering.store(true, Ordering::Release);
        let (tx, rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let result = (|| -> windows::core::Result<bool> {
                let mut visible = windows::core::BOOL::default();
                unsafe {
                    platform.controller().IsVisible(&mut visible)?;
                }
                if !visible.as_bool() {
                    unsafe {
                        platform.controller().SetIsVisible(true)?;
                    }
                }
                Ok(!visible.as_bool())
            })();
            if let Err(Ok(true)) = tx.send(result) {
                let _ = unsafe { platform.controller().SetIsVisible(false) };
            }
        })
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        let restore_hidden = rx
            .await
            .map_err(|_| WorkspaceError::NativeCommandFailed)?
            .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        lease.restore_hidden = restore_hidden;
        Ok(lease)
    }
    async fn finish(&mut self) -> Result<(), WorkspaceError> {
        if self.restore_hidden {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.view
                .with_webview(move |platform| {
                    let _ = tx.send(unsafe { platform.controller().SetIsVisible(false) });
                })
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            rx.await
                .map_err(|_| WorkspaceError::NativeCommandFailed)?
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            self.restore_hidden = false;
        }
        self.rendering.store(false, Ordering::Release);
        Ok(())
    }
}

// CEF capture reads the same native page's compositor without showing or
// focusing its NSView. Native fixture acceptance covers hidden capture.
#[cfg(target_os = "macos")]
struct RenderLease { rendering: Arc<AtomicBool> }
#[cfg(target_os = "macos")]
impl Drop for RenderLease { fn drop(&mut self) { self.rendering.store(false, Ordering::Release); } }
#[cfg(target_os = "macos")]
impl RenderLease {
    async fn start(view: &View, rendering: Arc<AtomicBool>) -> Result<Self, WorkspaceError> {
        if view.page.protocol.is_closed() { return Err(WorkspaceError::NativeCommandFailed); }
        rendering.store(true, Ordering::Release);
        Ok(Self { rendering })
    }
    async fn finish(&mut self) -> Result<(), WorkspaceError> { self.rendering.store(false, Ordering::Release); Ok(()) }
}

fn geometry(metrics: &Value) -> Result<(f64, f64, f64, f64, f64), WorkspaceError> {
    let viewport = &metrics["cssVisualViewport"];
    let numbers: Option<Vec<f64>> = ["pageX", "pageY", "clientWidth", "clientHeight", "zoom"]
        .iter()
        .map(|key| viewport[key].as_f64())
        .collect();
    let Some(values) = numbers else {
        return Err(WorkspaceError::NativeCommandFailed);
    };
    let (x, y, width, height) = (values[0], values[1], values[2], values[3]);
    let zoom=values[4];
    if !values.iter().all(|number| number.is_finite())
        || x < 0.0
        || y < 0.0
        || width < 1.0
        || height < 1.0
        || width > 32768.0
        || height > 32768.0
        || zoom <= 0.0 || zoom > 8.0
    {
        return Err(WorkspaceError::ObservationLimit);
    }
    Ok((x, y, width, height, zoom))
}

fn dimensions(data: &str) -> Result<(u32, u32), WorkspaceError> {
    if data.len() > MAX_ENCODED {
        return Err(WorkspaceError::ObservationLimit);
    }
    let png = STANDARD
        .decode(data)
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    if png.len() < 33 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[8..16] != b"\0\0\0\x0dIHDR" {
        return Err(WorkspaceError::NativeCommandFailed);
    }
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(WorkspaceError::ObservationLimit);
    }
    Ok((width, height))
}

fn capture_scale(width: f64, height: f64, density: f64) -> Result<f64, WorkspaceError> {
    if !density.is_finite() || density <= 0.0 || density > 8.0 {
        return Err(WorkspaceError::ObservationLimit);
    }
    Ok((f64::from(MAX_EDGE) / (width.max(height) * density)).min(1.0))
}

async fn pixel_density(view: &View) -> Result<f64, WorkspaceError> {
    let tree = native::protocol_call(view, "Page.getFrameTree", json!({}))
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let frame = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or(WorkspaceError::NativeCommandFailed)?;
    // A fixed, host-owned read in an isolated world: page scripts cannot spoof
    // window.devicePixelRatio here, and the model cannot supply an expression.
    let world = native::protocol_call(
        view,
        "Page.createIsolatedWorld",
        json!({"frameId":frame,"worldName":"nomifun-viewport-capture"}),
    )
    .await
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or(WorkspaceError::NativeCommandFailed)?;
    let result = native::protocol_call(
        view,
        "Runtime.evaluate",
        json!({"expression":"window.devicePixelRatio","contextId":context,"returnByValue":true}),
    )
    .await
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    result["result"]["value"]
        .as_f64()
        .ok_or(WorkspaceError::NativeCommandFailed)
}

pub(super) async fn capture(
    view: &View,
    target: BrowserTabTarget,
    cancel: &CancellationToken,
    active: Arc<AtomicBool>,
) -> Result<BrowserScreenshot, WorkspaceError> {
    if cancel.is_cancelled() {
        return Err(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled.into());
    }
    let mut rendering = RenderLease::start(view, active).await?;
    let result = capture_rendered(view, target, cancel).await;
    rendering.finish().await?;
    result
}

async fn capture_rendered(
    view: &View,
    target: BrowserTabTarget,
    cancel: &CancellationToken,
) -> Result<BrowserScreenshot, WorkspaceError> {
    if cancel.is_cancelled() {
        return Err(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled.into());
    }
    let metrics = native::protocol_call(view, "Page.getLayoutMetrics", json!({}))
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    let (x, y, width, height, zoom) = geometry(&metrics)?;
    let density = pixel_density(view).await?;
    let scale = capture_scale(width, height, density)?;
    if cancel.is_cancelled() {
        return Err(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled.into());
    }
    // CDP's capture clip is in zoomed viewport units. DPR already includes
    // browser zoom for the output pixel budget; apply zoom to the clip only.
    // Scale the capture, not the page layout. Native completion is always awaited.
    let mut result = native::protocol_call(
        view,
        "Page.captureScreenshot",
        json!({"format":"png","captureBeyondViewport":false,"fromSurface":true,
        "clip":{"x":x*zoom,"y":y*zoom,"width":width*zoom,"height":height*zoom,"scale":scale}}),
    )
    .await
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    if cancel.is_cancelled() {
        return Err(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled.into());
    }
    let after = native::protocol_call(view, "Page.getLayoutMetrics", json!({}))
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    if geometry(&after)? != (x, y, width, height, zoom) || pixel_density(view).await? != density {
        return Err(WorkspaceError::StaleObservation);
    }
    let Value::String(data) = result["data"].take() else {
        return Err(WorkspaceError::NativeCommandFailed);
    };
    let (image_width, image_height) = dimensions(&data)?;
    Ok(BrowserScreenshot {
        target,
        width: image_width,
        height: image_height,
        viewport_width: width,
        viewport_height: height,
        png_base64: data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_scale_bounds_physical_pixels_without_changing_css_geometry() {
        for density in [1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0] {
            let scale = capture_scale(1200.0, 800.0, density).unwrap();
            assert!(scale <= 1.0);
            assert!(1200.0 * density * scale <= 1600.0 + 0.00001);
        }
        for density in [0.0, -1.0, 9.0, f64::NAN, f64::INFINITY] {
            assert!(capture_scale(1200.0, 800.0, density).is_err());
        }
    }
    #[test]
    fn invalid_and_oversize_viewports_are_rejected() {
        assert!(geometry(&json!({})).is_err());
        for width in [0.0, -1.0, 32769.0] {
            assert!(geometry(&json!({"cssVisualViewport":{"pageX":0,"pageY":0,"clientWidth":width,"clientHeight":600,"zoom":1}})).is_err());
        }
        assert_eq!(geometry(&json!({"cssVisualViewport":{"pageX":0,"pageY":120,"clientWidth":880,"clientHeight":600,"zoom":1}})).unwrap(),(0.0,120.0,880.0,600.0,1.0));
    }
    #[test]
    fn invalid_png_and_excess_payload_are_rejected() {
        assert!(dimensions("not base64").is_err());
        assert!(dimensions(&STANDARD.encode(b"not a PNG")).is_err());
        assert_eq!(
            dimensions(&"A".repeat(MAX_ENCODED + 1)).unwrap_err(),
            WorkspaceError::ObservationLimit
        );
    }
}
