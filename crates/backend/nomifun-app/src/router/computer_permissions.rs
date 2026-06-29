//! Computer-use OS permission status + prompt endpoints (macOS TCC:
//! Accessibility / Screen Recording).
//!
//! These run IN the host process, so the live grant state reported here — and
//! the system prompt the request endpoint triggers — are evaluated against the
//! host app's own code identity (the desktop shell), which is exactly the
//! identity the user must grant. That makes this the authoritative answer to
//! "did my grant actually take effect for the running app?", where a visibly-on
//! System Settings toggle bound to a stale identity reports `false`.
//!
//! Off macOS (and on non-`computer-use` builds) the status fields are `null`
//! and the request endpoint is a no-op, so the shared web UI degrades cleanly.

use axum::Json;
use nomifun_api_types::ApiResponse;
use serde::{Deserialize, Serialize};

/// Build target, so the UI only renders the macOS permission panel where the
/// two TCC gates actually exist.
const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    "other"
};

#[derive(Serialize)]
pub(super) struct ComputerPermissionStatus {
    /// Accessibility (input synthesis + a11y tree). `null` where the platform
    /// has no such gate (non-macOS).
    accessibility: Option<bool>,
    /// Screen Recording (screenshots + Set-of-Marks overlay). `null` off macOS.
    screen_recording: Option<bool>,
    /// `"macos" | "windows" | "linux" | "other"`.
    platform: &'static str,
    /// The app name woven into permission guidance ("NomiFun"), so the UI names
    /// the same entry the user sees in System Settings.
    app_label: String,
}

fn current_status() -> ComputerPermissionStatus {
    #[cfg(feature = "computer-use")]
    {
        let s = nomi_computer::permissions::permission_status();
        ComputerPermissionStatus {
            accessibility: s.accessibility,
            screen_recording: s.screen_recording,
            platform: PLATFORM,
            app_label: nomi_computer::host_app_label(),
        }
    }
    #[cfg(not(feature = "computer-use"))]
    {
        ComputerPermissionStatus {
            accessibility: None,
            screen_recording: None,
            platform: PLATFORM,
            app_label: "this app".to_string(),
        }
    }
}

/// GET /api/computer/permissions — live grant state for the two macOS TCC
/// permissions computer-use needs. Read-only, cheap, safe to poll.
pub(super) async fn computer_permission_status() -> Json<ApiResponse<ComputerPermissionStatus>> {
    Json(ApiResponse::ok(current_status()))
}

#[derive(Deserialize)]
pub(super) struct PermissionRequestBody {
    /// `"accessibility"` or `"screen_recording"`.
    kind: String,
}

/// POST /api/computer/permissions/request — trigger the macOS authorization
/// prompt for the named permission. This is the path that REGISTERS the app in
/// the relevant System Settings list (so it stops being "missing from the
/// list") and shows the system dialog. Returns the post-call status.
///
/// A freshly-granted Screen Recording permission only takes effect after the
/// app is COMPLETELY relaunched (macOS caches it per process); the UI surfaces
/// that as a "quit & reopen" hint, so this returning `false` right after the
/// grant is expected, not an error. No-op off macOS / non-`computer-use` builds.
pub(super) async fn request_computer_permission(
    Json(body): Json<PermissionRequestBody>,
) -> Json<ApiResponse<ComputerPermissionStatus>> {
    #[cfg(feature = "computer-use")]
    {
        // The TCC prompt FFI returns quickly but is a blocking system call, so
        // keep it off the async worker thread.
        let _ = tokio::task::spawn_blocking(move || match body.kind.as_str() {
            "accessibility" => {
                nomi_computer::permissions::request_accessibility();
            }
            "screen_recording" => {
                nomi_computer::permissions::request_screen_recording();
            }
            _ => {}
        })
        .await;
    }
    #[cfg(not(feature = "computer-use"))]
    {
        // `kind` is meaningful only on computer-use builds; read it so the field
        // isn't dead code here (the web host has no desktop to grant).
        let _ = body.kind;
    }
    Json(ApiResponse::ok(current_status()))
}

/// POST /api/computer/permissions/open-settings — deep-link straight to the
/// System Settings pane for the named permission, so the user lands on the exact
/// list (and the exact "{app}" row) rather than hunting through Privacy &
/// Security. macOS only; no-op elsewhere. (We can't route this through the
/// shell `open-external` endpoint — it allows only http(s) URLs, not the
/// `x-apple.systempreferences:` scheme.)
pub(super) async fn open_permission_settings(
    Json(body): Json<PermissionRequestBody>,
) -> Json<ApiResponse<()>> {
    #[cfg(target_os = "macos")]
    {
        let url = match body.kind.as_str() {
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            "screen_recording" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            _ => "x-apple.systempreferences:com.apple.preference.security?Privacy",
        };
        // Detached so the handler never blocks on the GUI app launching.
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = body.kind;
    }
    Json(ApiResponse::ok(()))
}
