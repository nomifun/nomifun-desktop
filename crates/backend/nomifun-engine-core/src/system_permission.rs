//! Typed, bounded display data derived from an explicit Kernel denial. The
//! carrier is part of the original tool result, never a second permission log.

use serde::{Deserialize, Serialize};

pub const SYSTEM_PERMISSION_REQUIRED: &str = "SYSTEM_PERMISSION_REQUIRED";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionKind {
    ScreenRecording,
    Accessibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemPermissionRequired {
    pub schema: String,
    pub capability: String,
    pub action_id: String,
    pub operation: String,
    pub permissions: Vec<SystemPermissionKind>,
    pub execution: String,
}

impl SystemPermissionRequired {
    fn valid(&self) -> bool {
        let expected = match (self.action_id.as_str(), self.operation.as_str()) {
            ("computer/observe", "screenshot") => Some(SystemPermissionKind::ScreenRecording),
            ("computer/a11y.observe", "observe") => Some(SystemPermissionKind::Accessibility),
            ("computer/input", "click_element" | "set_element_value" | "right_click_element"
                | "double_click_element" | "left_click" | "right_click" | "middle_click"
                | "double_click" | "triple_click" | "mouse_move" | "left_click_drag"
                | "type" | "key" | "scroll" | "focus_window") => Some(SystemPermissionKind::Accessibility),
            _ => None,
        };
        self.schema == "system-permission-required.v1" && self.capability == "computer"
            && self.execution == "not_executed"
            && expected.is_some_and(|expected| self.permissions == [expected])
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KernelToolError {
    schema: String,
    error_code: String,
    permission_required: SystemPermissionRequired,
}

/// Called only with the code/message of a typed CapabilityKernel error.
/// Arbitrary failure prose and unrelated codes preserve their normal path.
pub fn system_permission_error_carrier(code: &str, message: &str) -> Option<String> {
    if code != SYSTEM_PERMISSION_REQUIRED { return None; }
    let permission_required: SystemPermissionRequired = serde_json::from_str(message).ok()?;
    if !permission_required.valid() { return None; }
    serde_json::to_string(&KernelToolError {
        schema: "kernel-tool-error.v1".into(), error_code: code.into(), permission_required,
    }).ok()
}

/// Both live and history projection require the original admitted action and
/// explicit failed result. A successful result or foreign capability cannot
/// turn a JSON-looking tool body into a permission prompt.
pub fn project_system_permission_required(
    capability_id: Option<&str>, action_id: Option<&str>, is_error: bool, output: &str,
) -> Option<SystemPermissionRequired> {
    if !is_error || capability_id != Some("computer") { return None; }
    let carrier: KernelToolError = serde_json::from_str(output).ok()?;
    (carrier.schema == "kernel-tool-error.v1" && carrier.error_code == SYSTEM_PERMISSION_REQUIRED
        && carrier.permission_required.valid()
        && action_id == Some(carrier.permission_required.action_id.as_str()))
        .then_some(carrier.permission_required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permission_projection_requires_the_explicit_failure_code_and_admitted_action() {
        let message = json!({"schema":"system-permission-required.v1", "capability":"computer",
            "action_id":"computer/observe", "operation":"screenshot", "permissions":["screen_recording"],
            "execution":"not_executed"}).to_string();
        let carrier = system_permission_error_carrier(SYSTEM_PERMISSION_REQUIRED, &message).unwrap();
        assert!(project_system_permission_required(Some("computer"), Some("computer/observe"), true, &carrier).is_some());
        for (capability, action, is_error, body) in [
            ("computer", "computer/observe", false, carrier.as_str()),
            ("browser", "computer/observe", true, carrier.as_str()),
            ("computer", "computer/input", true, carrier.as_str()),
            ("computer", "computer/observe", true, message.as_str()),
        ] { assert!(project_system_permission_required(Some(capability), Some(action), is_error, body).is_none()); }
        assert!(system_permission_error_carrier("ROLE_HOST_PROVIDER_FAILURE", &message).is_none());
        for (key, value) in [("execution",json!("unknown")), ("permissions",json!([])),
            ("operation",json!("wait")), ("extra",json!(true))] {
            let mut malformed: serde_json::Value = serde_json::from_str(&message).unwrap();
            malformed[key] = value;
            assert!(system_permission_error_carrier(SYSTEM_PERMISSION_REQUIRED, &malformed.to_string()).is_none());
        }
    }
}
