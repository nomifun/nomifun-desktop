//! Host OS permission inventory and guidance endpoints.
//!
//! The desktop backend is linked into the NomiFun process. Status probes and
//! prompts therefore use the exact code identity that macOS TCC evaluates,
//! instead of a terminal, browser, or helper process guessing on its behalf.
//! Camera/microphone and Computer grants are probed live. Full Disk Access,
//! target-specific Apple Events and local-network access have no truthful
//! process-wide Boolean here: their inventory remains unknown and the actual
//! operation supplies the result. Notification state stays with Tauri's plugin.

use axum::{Json, http::StatusCode, response::{IntoResponse, Response}};
use nomifun_api_types::{ApiResponse, ErrorResponse};
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

pub(super) fn computer_status_routes(services: &crate::services::AppServices) -> axum::Router {
    #[cfg(feature = "computer-use")]
    let handler = {
        let computer = services.computer.clone();
        axum::routing::get(move || {
            let computer = computer.clone();
            async move { Json(ApiResponse::ok(computer.status())) }
        })
    };
    #[cfg(not(feature = "computer-use"))]
    let handler = {
        let _ = services;
        axum::routing::get(|| async { Json(ApiResponse::ok(serde_json::json!({
            "worker_version": null, "platform": PLATFORM, "backend": null,
            "session_type": null, "wayland": null, "atspi": null,
            "last_error": "Computer is unavailable in this build", "running": false,
        }))) })
    };
    axum::Router::new().route("/api/computer/status", handler)
}

const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    "other"
};

fn app_label() -> String { "NomiFun".to_owned() }

// ---------------------------------------------------------------------------
// Canonical system-permission inventory
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SystemPermissionState {
    Granted,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Denied,
    NotDetermined,
    /// The native API proves the grant is absent but cannot distinguish an
    /// initial request from a previous denial (AX and screen-capture preflight).
    #[cfg_attr(not(all(target_os = "macos", feature = "computer-use")), allow(dead_code))]
    NotGranted,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Restricted,
    #[cfg_attr(any(target_os = "macos", not(feature = "computer-use")), allow(dead_code))]
    NotRequired,
    Unknown,
}

#[derive(Debug, Serialize)]
pub(super) struct SystemPermissionEntry {
    kind: &'static str,
    state: SystemPermissionState,
    can_request: bool,
    can_open_settings: bool,
    requires_restart_after_grant: bool,
    capabilities: &'static [&'static str],
}

#[derive(Debug, Serialize)]
pub(super) struct SystemPermissionStatus {
    platform: &'static str,
    app_label: String,
    /// Diagnostic identity of this actual probe process, not a configured
    /// production identifier or a Terminal's permission state.
    process_id: u32,
    app_bundle_identifier: Option<String>,
    permissions: Vec<SystemPermissionEntry>,
}

#[cfg(target_os = "macos")]
fn capture_state(camera: bool) -> SystemPermissionState {
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo,
    };

    let Some(media_type) = (unsafe { if camera { AVMediaTypeVideo } else { AVMediaTypeAudio } }) else {
        return SystemPermissionState::Unknown;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) } {
        AVAuthorizationStatus::Authorized => SystemPermissionState::Granted,
        AVAuthorizationStatus::Denied => SystemPermissionState::Denied,
        AVAuthorizationStatus::Restricted => SystemPermissionState::Restricted,
        AVAuthorizationStatus::NotDetermined => SystemPermissionState::NotDetermined,
        _ => SystemPermissionState::Unknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn capture_state(_camera: bool) -> SystemPermissionState {
    // Windows/Linux media capture is owned by the renderer and the OS/browser
    // privacy layer. The settings UI offers a real, user-triggered microphone
    // test there instead of presenting a guessed backend status.
    SystemPermissionState::Unknown
}

fn computer_states() -> (SystemPermissionState, SystemPermissionState) {
    #[cfg(all(target_os = "macos", feature = "computer-use"))]
    {
        let status = nomifun_computer::permissions::permission_status();
        let accessibility = binary_grant_state(status.accessibility);
        let screen_recording = binary_grant_state(status.screen_recording);
        (accessibility, screen_recording)
    }
    #[cfg(all(any(target_os = "windows", target_os = "linux"), feature = "computer-use"))]
    {
        // There is no macOS-style up-front TCC switch on these hosts. Native
        // calls may still fail across secure desktop / integrity / display
        // server boundaries, and those remain operation-level diagnostics.
        (
            SystemPermissionState::NotRequired,
            SystemPermissionState::NotRequired,
        )
    }
    #[cfg(all(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), feature = "computer-use"))]
    { (SystemPermissionState::Unknown, SystemPermissionState::Unknown) }
    #[cfg(not(feature = "computer-use"))]
    {
        // A server/WebUI build has no local Computer provider. `not_required`
        // would incorrectly look launch-ready, so retain an explicit unknown
        // state and let the desktop product own the actionable permission UI.
        (
            SystemPermissionState::Unknown,
            SystemPermissionState::Unknown,
        )
    }
}

#[cfg(any(all(target_os = "macos", feature = "computer-use"), test))]
fn binary_grant_state(granted: Option<bool>) -> SystemPermissionState {
    match granted {
        Some(true) => SystemPermissionState::Granted,
        Some(false) => SystemPermissionState::NotGranted,
        None => SystemPermissionState::Unknown,
    }
}

fn current_system_status() -> SystemPermissionStatus {
    let (accessibility, screen_recording) = computer_states();
    let microphone = capture_state(false);
    let camera = capture_state(true);
    let macos = cfg!(target_os = "macos");
    let computer_use_available = cfg!(feature = "computer-use");
    SystemPermissionStatus {
        platform: PLATFORM,
        app_label: app_label(),
        process_id: std::process::id(),
        app_bundle_identifier: {
            #[cfg(target_os = "macos")]
            { macos_bundle_identifier() }
            #[cfg(not(target_os = "macos"))]
            { None }
        },
        permissions: vec![
            SystemPermissionEntry {
                kind: "microphone",
                state: microphone,
                can_request: macos && microphone == SystemPermissionState::NotDetermined,
                can_open_settings: macos,
                requires_restart_after_grant: false,
                capabilities: &["voice_input", "browser_use"],
            },
            SystemPermissionEntry {
                kind: "camera",
                state: camera,
                can_request: macos && camera == SystemPermissionState::NotDetermined,
                can_open_settings: macos,
                requires_restart_after_grant: false,
                capabilities: &["browser_use"],
            },
            SystemPermissionEntry {
                kind: "accessibility",
                state: accessibility,
                can_request: macos
                    && computer_use_available
                    && accessibility != SystemPermissionState::Granted,
                can_open_settings: macos && computer_use_available,
                requires_restart_after_grant: false,
                capabilities: &["computer_use"],
            },
            SystemPermissionEntry {
                kind: "screen_recording",
                state: screen_recording,
                // An existing CG grant still needs the user's explicit tiny
                // capture to verify the driver's separate capture consent.
                can_request: macos && computer_use_available,
                can_open_settings: macos && computer_use_available,
                requires_restart_after_grant: macos && computer_use_available,
                capabilities: &["computer_use"],
            },
            SystemPermissionEntry {
                kind: "full_disk_access",
                // Do not inspect TCC databases or read unrelated private files
                // to manufacture a global FDA status. Root is not an FDA grant.
                state: SystemPermissionState::Unknown,
                can_request: false,
                can_open_settings: macos,
                requires_restart_after_grant: macos,
                capabilities: &["local_files"],
            },
            SystemPermissionEntry {
                kind: "automation",
                // Apple Events grants belong to a sender + target + event.
                // Only the actual cross-app operation can establish its result.
                state: SystemPermissionState::Unknown,
                can_request: false,
                can_open_settings: macos,
                requires_restart_after_grant: false,
                capabilities: &["application_automation"],
            },
            SystemPermissionEntry {
                kind: "local_network",
                // macOS prompts on a real LAN operation, not a global request.
                state: SystemPermissionState::Unknown,
                can_request: false,
                can_open_settings: macos,
                requires_restart_after_grant: false,
                capabilities: &["local_connections", "browser_use"],
            },
        ],
    }
}

/// GET /api/system/permissions — one live inventory for every settings and
/// contextual permission gate in the renderer.
pub(super) async fn system_permission_status() -> Json<ApiResponse<SystemPermissionStatus>> {
    Json(ApiResponse::ok(current_system_status()))
}

#[derive(Deserialize)]
pub(super) struct PermissionRequestBody {
    kind: String,
}

#[cfg(target_os = "macos")]
async fn request_capture_permission(camera: bool) {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio, AVMediaTypeVideo};
    use std::sync::{Arc, Mutex};

    // Keep every Objective-C value in this inner scope. RcBlock and NSString
    // references are deliberately !Send and an Axum handler future must not
    // retain either one across its await point.
    let receiver = {
        let Some(media_type) = (unsafe { if camera { AVMediaTypeVideo } else { AVMediaTypeAudio } }) else {
            return;
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        let callback_sender = Arc::clone(&sender);
        let callback = RcBlock::new(move |_granted: Bool| {
            if let Some(sender) = callback_sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = sender.send(());
            }
        });
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &callback);
        }
        // AVFoundation copies the completion block before returning.
        receiver
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(60), receiver).await;
}

#[cfg(not(target_os = "macos"))]
async fn request_capture_permission(_camera: bool) {}

async fn request_computer_kind(kind: &str) {
    #[cfg(feature = "computer-use")]
    {
        let kind = kind.to_owned();
        // The Accessibility prompt is synchronous FFI.
        let _ = tokio::task::spawn_blocking(move || match kind.as_str() {
            "accessibility" => {
                nomifun_computer::permissions::request_accessibility();
            }
            _ => {}
        })
        .await;
    }
    #[cfg(not(feature = "computer-use"))]
    {
        let _ = kind;
    }
}

/// POST /api/system/permissions/request — request only the named up-front
/// permission for this host. Browser website grants are separate from these
/// native camera/microphone grants and remain owned by the visible page.
pub(super) fn request_system_permission(services: &crate::services::AppServices) -> axum::routing::MethodRouter {
    #[cfg(feature = "computer-use")]
    let computer = services.computer.clone();
    #[cfg(not(feature = "computer-use"))]
    let _ = services;
    axum::routing::post(move |Json(body): Json<PermissionRequestBody>| {
        #[cfg(feature = "computer-use")]
        let computer = computer.clone();
        async move {
            request_system_permission_inner(body, #[cfg(feature = "computer-use")] computer).await
        }
    })
}

async fn request_system_permission_inner(
    body: PermissionRequestBody,
    #[cfg(feature = "computer-use")] computer: std::sync::Arc<nomifun_computer::Computer>,
) -> Response {
    match body.kind.as_str() {
        "microphone" => request_capture_permission(false).await,
        "camera" => request_capture_permission(true).await,
        "accessibility" => request_computer_kind(&body.kind).await,
        "screen_recording" => {
            #[cfg(all(target_os = "macos", feature = "computer-use"))]
            {
                // This endpoint is local-owner protected. No GET inventory,
                // cached status or model action may request this capture.
                let granted = tokio::task::spawn_blocking(nomifun_computer::permissions::request_screen_recording).await;
                match granted {
                    Ok(true) => {}
                    Ok(false) => return permission_request_failure(StatusCode::CONFLICT,
                        "SYSTEM_PERMISSION_REQUIRED", "Screen Recording is not granted; verification capture was not dispatched", current_system_status()),
                    Err(error) => return permission_request_failure(StatusCode::BAD_GATEWAY,
                        "SYSTEM_PERMISSION_REQUEST_FAILED", error.to_string(), current_system_status()),
                }
                if let Err(error) = computer.request_screen_capture().await {
                    return permission_request_failure(StatusCode::BAD_GATEWAY, error.code,
                        error.message, current_system_status());
                }
            }
            #[cfg(not(all(target_os = "macos", feature = "computer-use")))]
            return permission_request_failure(StatusCode::NOT_IMPLEMENTED,
                "COMPUTER_PLATFORM_UNSUPPORTED", "Explicit screen capture consent is only available in a macOS Computer build", current_system_status());
        }
        _ => {}
    }
    #[cfg(all(feature = "computer-use", not(target_os = "macos")))]
    let _ = computer;
    Json(ApiResponse::ok(current_system_status())).into_response()
}

fn permission_request_failure(status: StatusCode, code: impl Into<String>, message: impl Into<String>, inventory: SystemPermissionStatus) -> Response {
    (status, Json(ErrorResponse::new_with_details(message, code,
        serde_json::json!({"system_permissions": inventory})))).into_response()
}

/// POST /api/system/permissions/open-settings — deep-link to an exact macOS
/// privacy pane. Some kinds (camera/location/notifications) are on-demand
/// Browser or platform permissions and therefore have no global request API,
/// but still need a recovery path after the user previously denied them.
pub(super) async fn open_permission_settings(
    Json(body): Json<PermissionRequestBody>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    #[cfg(target_os = "macos")]
    {
        let bundle_id = macos_bundle_identifier();
        let url = permission_settings_url(&body.kind, bundle_id.as_deref());
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new("/usr/bin/open").args(["--", &url]).output()
        })
        .await
        .map_err(|error| AppError::Internal(format!("System Settings launch task failed: {error}")))?
        .map_err(|error| AppError::Internal(format!("Could not open System Settings: {error}")))?;
        if !output.status.success() {
            let detail: String = String::from_utf8_lossy(&output.stderr).trim().chars().take(512).collect();
            return Err(AppError::Internal(format!("macOS refused to open System Settings ({}): {detail}", output.status)));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = body.kind;
    }
    Ok(Json(ApiResponse::ok(())))
}

#[cfg(any(target_os = "macos", test))]
fn permission_settings_url(kind: &str, bundle_id: Option<&str>) -> String {
    if kind == "notifications" {
        let base = "x-apple.systempreferences:com.apple.Notifications-Settings.extension";
        // A bare/server process has no app bundle. Open the notification
        // inventory then, instead of inventing a production identity. Bundle
        // identifiers are restricted here so they cannot alter URL parameters.
        return match bundle_id.filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))) {
            Some(id) => format!("{base}?id={id}"),
            None => base.to_owned(),
        };
    }
    let pane = match kind {
        "accessibility" => "Privacy_Accessibility",
        "screen_recording" => "Privacy_ScreenCapture",
        "microphone" => "Privacy_Microphone",
        "camera" => "Privacy_Camera",
        "location" => "Privacy_LocationServices",
        "local_network" => "Privacy_LocalNetwork",
        "full_disk_access" => "Privacy_AllFiles",
        "automation" => "Privacy_Automation",
        _ => "Privacy",
    };
    format!("x-apple.systempreferences:com.apple.preference.security?{pane}")
}

/// CoreFoundation's main bundle identifies the exact running app, including
/// the development bundle. It is safe to read on an Axum worker and does not
/// require an AppKit main-thread marker or trigger any permission request.
#[cfg(target_os = "macos")]
fn macos_bundle_identifier() -> Option<String> {
    use std::ffi::{CStr, c_char, c_void};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> *const c_void;
        fn CFBundleGetIdentifier(bundle: *const c_void) -> *const c_void;
        fn CFStringGetLength(string: *const c_void) -> isize;
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, capacity: isize, encoding: u32) -> u8;
    }
    // These are Get-rule references owned by CoreFoundation. Copy the UTF-8
    // identifier immediately; no CF pointer crosses an async boundary.
    unsafe {
        let bundle = CFBundleGetMainBundle();
        if bundle.is_null() { return None; }
        let identifier = CFBundleGetIdentifier(bundle);
        if identifier.is_null() { return None; }
        let length = CFStringGetLength(identifier);
        if !(1..=4096).contains(&length) { return None; }
        let mut bytes = vec![0u8; length as usize * 4 + 1];
        const UTF8: u32 = 0x0800_0100;
        if CFStringGetCString(identifier, bytes.as_mut_ptr().cast(), bytes.len() as isize, UTF8) == 0 { return None; }
        CStr::from_ptr(bytes.as_ptr().cast()).to_str().ok().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_settings_use_the_current_bundle_and_do_not_invent_an_identity() {
        for id in ["com.nomifun.desktop", "com.nomifun.desktop.dev", "org.example.custom-build"] {
            assert!(permission_settings_url("notifications", Some(id)).ends_with(&format!("?id={id}")));
        }
        for id in [None, Some(""), Some("com.nomifun.desktop&unexpected=true")] {
            assert_eq!(permission_settings_url("notifications", id), "x-apple.systempreferences:com.apple.Notifications-Settings.extension");
        }
        assert!(permission_settings_url("screen_recording", None).ends_with("?Privacy_ScreenCapture"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn main_bundle_identity_can_be_read_without_an_appkit_main_thread() {
        // CLI test hosts commonly have no bundle, which is an honest None.
        // This read-only probe must also complete on a non-main worker.
        let result = std::thread::spawn(macos_bundle_identifier).join().unwrap();
        if let Some(id) = result { assert!(!id.is_empty()); }
    }

    #[test]
    fn inventory_covers_backend_probeable_host_permissions() {
        let status = current_system_status();
        let kinds = status
            .permissions
            .iter()
            .map(|permission| permission.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["microphone", "camera", "accessibility", "screen_recording", "full_disk_access", "automation", "local_network"]
        );
        assert_eq!(status.permissions[0].capabilities, &["voice_input", "browser_use"]);
        assert_eq!(status.permissions[1].capabilities, &["browser_use"]);
        for permission in &status.permissions[4..] {
            assert_eq!(permission.state, SystemPermissionState::Unknown);
            assert!(!permission.can_request);
        }
    }

    #[test]
    fn binary_preflight_never_invents_authorization_history() {
        assert_eq!(binary_grant_state(Some(true)), SystemPermissionState::Granted);
        assert_eq!(binary_grant_state(Some(false)), SystemPermissionState::NotGranted);
        assert_eq!(binary_grant_state(None), SystemPermissionState::Unknown);
    }

    #[tokio::test]
    async fn failed_capture_verification_keeps_granted_inventory_and_returns_http_error() {
        // Supply an inventory fixture: this test must never request native TCC
        // or capture any image, and a capture failure cannot revoke a CG fact.
        let inventory = SystemPermissionStatus {
            platform: "macos", app_label: "NomiFun".into(), process_id: 7,
            app_bundle_identifier: Some("com.nomifun.desktop.dev".into()),
            permissions: vec![SystemPermissionEntry {
                kind: "screen_recording", state: SystemPermissionState::Granted,
                can_request: true, can_open_settings: true,
                requires_restart_after_grant: true, capabilities: &["computer_use"],
            }],
        };
        let response = permission_request_failure(StatusCode::BAD_GATEWAY,
            "COMPUTER_SCREEN_CAPTURE_REQUEST_FAILED", "No verification frame", inventory);
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["success"], false);
        assert_eq!(body["code"], "COMPUTER_SCREEN_CAPTURE_REQUEST_FAILED");
        assert_eq!(body["details"]["system_permissions"]["permissions"][0]["state"], "granted");
        assert_eq!(body["details"]["system_permissions"]["permissions"][0]["can_request"], true);
    }

    #[cfg(not(feature = "computer-use"))]
    #[test]
    fn headless_build_does_not_claim_computer_permission_readiness() {
        let (accessibility, screen) = computer_states();
        assert_eq!(accessibility, SystemPermissionState::Unknown);
        assert_eq!(screen, SystemPermissionState::Unknown);
    }
}
