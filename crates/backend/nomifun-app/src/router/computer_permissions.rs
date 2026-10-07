//! Host OS permission inventory and guidance endpoints.
//!
//! The desktop backend is linked into the NomiFun process. Status probes and
//! prompts therefore use the exact code identity that macOS TCC evaluates,
//! instead of a terminal, browser, or helper process guessing on its behalf.
//! The inventory covers the capability prerequisites this backend can probe
//! authoritatively: ASR microphone input and Computer Use Accessibility /
//! Screen Recording. Notification state stays with Tauri's native notification
//! plugin. Browser website and local-network permissions remain deliberately
//! on-demand; this module opens the relevant System Settings pane when their
//! OS-level gate blocks a user-approved operation.

use axum::Json;
use nomifun_api_types::ApiResponse;
use serde::{Deserialize, Serialize};

const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    "other"
};

fn app_label() -> String {
    #[cfg(feature = "computer-use")]
    {
        nomi_computer::host_app_label()
    }
    #[cfg(not(feature = "computer-use"))]
    {
        "NomiFun".to_owned()
    }
}

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
    permissions: Vec<SystemPermissionEntry>,
}

#[cfg(target_os = "macos")]
fn microphone_state() -> SystemPermissionState {
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio,
    };

    let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
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
fn microphone_state() -> SystemPermissionState {
    // Windows/Linux media capture is owned by the renderer and the OS/browser
    // privacy layer. The settings UI offers a real, user-triggered microphone
    // test there instead of presenting a guessed backend status.
    SystemPermissionState::Unknown
}

fn computer_states() -> (SystemPermissionState, SystemPermissionState) {
    #[cfg(all(target_os = "macos", feature = "computer-use"))]
    {
        let status = nomi_computer::permissions::permission_status();
        let accessibility = if status.accessibility == Some(true) {
            SystemPermissionState::Granted
        } else {
            // AXIsProcessTrusted does not distinguish "never asked" from
            // "disabled". `not_determined` truthfully means the user still has
            // an authorization step to complete; the request action registers
            // the app and opens the exact pane.
            SystemPermissionState::NotDetermined
        };
        let screen_recording = if status.screen_recording == Some(true) {
            SystemPermissionState::Granted
        } else {
            SystemPermissionState::NotDetermined
        };
        (accessibility, screen_recording)
    }
    #[cfg(all(not(target_os = "macos"), feature = "computer-use"))]
    {
        // There is no macOS-style up-front TCC switch on these hosts. Native
        // calls may still fail across secure desktop / integrity / display
        // server boundaries, and those remain operation-level diagnostics.
        (
            SystemPermissionState::NotRequired,
            SystemPermissionState::NotRequired,
        )
    }
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

fn current_system_status() -> SystemPermissionStatus {
    let (accessibility, screen_recording) = computer_states();
    let microphone = microphone_state();
    let macos = cfg!(target_os = "macos");
    let computer_use_available = cfg!(feature = "computer-use");
    SystemPermissionStatus {
        platform: PLATFORM,
        app_label: app_label(),
        permissions: vec![
            SystemPermissionEntry {
                kind: "microphone",
                state: microphone,
                can_request: macos && microphone == SystemPermissionState::NotDetermined,
                can_open_settings: macos,
                requires_restart_after_grant: false,
                capabilities: &["voice_input"],
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
                can_request: macos
                    && computer_use_available
                    && screen_recording != SystemPermissionState::Granted,
                can_open_settings: macos && computer_use_available,
                requires_restart_after_grant: macos && computer_use_available,
                capabilities: &["computer_use"],
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
async fn request_microphone_permission() {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
    use std::sync::{Arc, Mutex};

    // Keep every Objective-C value in this inner scope. RcBlock and NSString
    // references are deliberately !Send and an Axum handler future must not
    // retain either one across its await point.
    let receiver = {
        let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
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
async fn request_microphone_permission() {}

async fn request_computer_kind(kind: &str) {
    #[cfg(feature = "computer-use")]
    {
        let kind = kind.to_owned();
        // The CoreGraphics / Accessibility prompt calls are synchronous FFI.
        let _ = tokio::task::spawn_blocking(move || match kind.as_str() {
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
        let _ = kind;
    }
}

/// POST /api/system/permissions/request — request only the named up-front
/// permission. Browser website grants are intentionally not accepted here.
pub(super) async fn request_system_permission(
    Json(body): Json<PermissionRequestBody>,
) -> Json<ApiResponse<SystemPermissionStatus>> {
    match body.kind.as_str() {
        "microphone" => request_microphone_permission().await,
        "accessibility" | "screen_recording" => request_computer_kind(&body.kind).await,
        _ => {}
    }
    Json(ApiResponse::ok(current_system_status()))
}

/// POST /api/system/permissions/open-settings — deep-link to an exact macOS
/// privacy pane. Some kinds (camera/location/notifications) are on-demand
/// Browser or platform permissions and therefore have no global request API,
/// but still need a recovery path after the user previously denied them.
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
            "microphone" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            "camera" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera",
            "location" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"
            }
            "local_network" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork"
            }
            "notifications" => "x-apple.systempreferences:com.apple.Notifications-Settings.extension?bundleIdentifier=com.nomifun.desktop",
            "full_disk_access" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
            }
            _ => "x-apple.systempreferences:com.apple.preference.security?Privacy",
        };
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = body.kind;
    }
    Json(ApiResponse::ok(()))
}

// ---------------------------------------------------------------------------
// Backward-compatible Computer Use API
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(super) struct ComputerPermissionStatus {
    accessibility: Option<bool>,
    screen_recording: Option<bool>,
    platform: &'static str,
    app_label: String,
}

fn current_computer_status() -> ComputerPermissionStatus {
    #[cfg(feature = "computer-use")]
    {
        let status = nomi_computer::permissions::permission_status();
        ComputerPermissionStatus {
            accessibility: status.accessibility,
            screen_recording: status.screen_recording,
            platform: PLATFORM,
            app_label: app_label(),
        }
    }
    #[cfg(not(feature = "computer-use"))]
    {
        ComputerPermissionStatus {
            accessibility: None,
            screen_recording: None,
            platform: PLATFORM,
            app_label: app_label(),
        }
    }
}

pub(super) async fn computer_permission_status() -> Json<ApiResponse<ComputerPermissionStatus>> {
    Json(ApiResponse::ok(current_computer_status()))
}

pub(super) async fn request_computer_permission(
    Json(body): Json<PermissionRequestBody>,
) -> Json<ApiResponse<ComputerPermissionStatus>> {
    request_computer_kind(&body.kind).await;
    Json(ApiResponse::ok(current_computer_status()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            vec!["microphone", "accessibility", "screen_recording"]
        );
        assert_eq!(status.permissions[0].capabilities, &["voice_input"]);
        assert_eq!(status.permissions[1].capabilities, &["computer_use"]);
        assert_eq!(status.permissions[2].capabilities, &["computer_use"]);
    }

    #[cfg(all(not(target_os = "macos"), feature = "computer-use"))]
    #[test]
    fn non_macos_computer_permissions_do_not_block_launch() {
        let (accessibility, screen) = computer_states();
        assert_eq!(accessibility, SystemPermissionState::NotRequired);
        assert_eq!(screen, SystemPermissionState::NotRequired);
    }

    #[cfg(not(feature = "computer-use"))]
    #[test]
    fn headless_build_does_not_claim_computer_permission_readiness() {
        let (accessibility, screen) = computer_states();
        assert_eq!(accessibility, SystemPermissionState::Unknown);
        assert_eq!(screen, SystemPermissionState::Unknown);
    }
}
