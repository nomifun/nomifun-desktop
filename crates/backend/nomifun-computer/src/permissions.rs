//! Exact host-process OS prerequisite probes. No permissions are requested by a call.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub accessibility: Option<bool>,
    pub screen_recording: Option<bool>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerPermissionKind {
    Accessibility,
    ScreenRecording,
}
pub fn permission_status() -> PermissionStatus {
    PermissionStatus {
        accessibility: accessibility_granted(),
        screen_recording: screen_recording_granted(),
    }
}
pub fn accessibility_granted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        Some(macos::accessibility(false))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
pub fn screen_recording_granted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        Some(unsafe { macos::CGPreflightScreenCaptureAccess() })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
pub fn request_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::accessibility(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
pub fn request_screen_recording() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe { macos::CGRequestScreenCaptureAccess() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
pub fn required_permissions(action: &str, args: &Value) -> Vec<ComputerPermissionKind> {
    use ComputerPermissionKind::*;
    let Some(tool) = crate::surface::tool(action) else {
        return Vec::new();
    };
    use crate::surface::ScreenRecordingRequirement;
    let mut required = Vec::new();
    if tool.system_permissions.accessibility {
        required.push(Accessibility);
    }
    let screenshot = match tool.system_permissions.screen_recording {
        ScreenRecordingRequirement::Never => false,
        ScreenRecordingRequirement::Always => true,
        ScreenRecordingRequirement::WindowScreenshot => args
            .get("include_screenshot")
            .and_then(Value::as_bool)
            .unwrap_or(tool.system_permissions.screenshot_default),
        ScreenRecordingRequirement::BatchScreenshot => {
            args.get("observe").is_some_and(Value::is_object)
                && args
                    .pointer("/observe/include_screenshot")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
        }
    };
    if screenshot {
        required.push(ScreenRecording);
    }
    required
}
pub fn missing_permissions(
    action: &str,
    args: &Value,
    status: PermissionStatus,
) -> Vec<ComputerPermissionKind> {
    required_permissions(action, args)
        .into_iter()
        .filter(|kind| match kind {
            ComputerPermissionKind::Accessibility => status.accessibility == Some(false),
            ComputerPermissionKind::ScreenRecording => status.screen_recording == Some(false),
        })
        .collect()
}
pub fn required_carrier(action: &str, args: &Value, status: PermissionStatus) -> Option<Value> {
    let missing = missing_permissions(action, args, status);
    if missing.is_empty() {
        return None;
    }
    let tool = crate::surface::tool(action)?;
    Some(
        json!({"schema":"system-permission-required.v1","capability":"computer","action_id":tool.action_id,"operation":tool.driver_tool,"permissions":missing,"execution":"not_executed"}),
    )
}
#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::{
        base::TCFType,
        boolean::CFBoolean,
        dictionary::{CFDictionary, CFDictionaryRef},
        string::{CFString, CFStringRef},
    };
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        pub fn CGPreflightScreenCaptureAccess() -> bool;
        pub fn CGRequestScreenCaptureAccess() -> bool;
    }
    pub fn accessibility(prompt: bool) -> bool {
        unsafe {
            if !prompt {
                return AXIsProcessTrusted() != 0;
            }
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let options = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::true_value().as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permissions_follow_frozen_actions_and_only_confirmed_denial() {
        let denied = PermissionStatus {
            accessibility: Some(false),
            screen_recording: Some(false),
        };
        assert!(required_carrier("computer/launch_app", &json!({}), denied).is_none());
        let window = required_carrier(
            "computer/get_window_state",
            &json!({"include_screenshot":false}),
            denied,
        )
        .unwrap();
        assert_eq!(window["operation"], "get_window_state");
        assert_eq!(window["permissions"], json!(["accessibility"]));
        assert_eq!(
            required_permissions("computer/verify_state", &json!({})),
            vec![ComputerPermissionKind::Accessibility]
        );
        assert!(
            required_carrier(
                "computer/click",
                &json!({}),
                PermissionStatus {
                    accessibility: None,
                    screen_recording: None
                }
            )
            .is_none()
        );
    }
}
