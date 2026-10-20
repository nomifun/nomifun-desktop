//! Typed, bounded display data derived from an explicit Kernel denial. The
//! carrier is part of the original tool result, never a second permission log.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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
        let Some(tool) = computer_tools().tools.iter().find(|tool|
            tool.action_id == self.action_id && tool.driver_tool == self.operation) else { return false; };
        self.schema == "system-permission-required.v1" && self.capability == "computer"
            && self.execution == "not_executed"
            && !self.permissions.is_empty()
            && self.permissions.iter().enumerate().all(|(index, permission)| {
                !self.permissions[..index].contains(permission) && match permission {
                    SystemPermissionKind::Accessibility => tool.system_permissions.accessibility,
                    SystemPermissionKind::ScreenRecording => tool.system_permissions.screen_recording != "never",
                }
            })
    }
}

#[derive(Deserialize)]
struct ComputerTools { tools: Vec<FrozenComputerAction> }
#[derive(Deserialize)]
struct FrozenComputerAction { action_id: String, driver_tool: String, system_permissions: ComputerPermissions }
#[derive(Deserialize)]
struct ComputerPermissions { accessibility: bool, screen_recording: String }

fn computer_tools() -> &'static ComputerTools {
    static TOOLS: OnceLock<ComputerTools> = OnceLock::new();
    TOOLS.get_or_init(|| serde_json::from_str(include_str!(
        "../../nomifun-agent-contracts/contracts/computer-tools.v1.json"
    )).expect("the generated Computer permission surface must be valid"))
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

    #[test]
    fn frozen_operation_accepts_only_its_missing_permission_subset() {
        let tool = computer_tools().tools.iter().find(|tool|
            tool.system_permissions.accessibility && tool.system_permissions.screen_recording == "window_screenshot").unwrap();
        let mut request = SystemPermissionRequired {
            schema: "system-permission-required.v1".into(), capability: "computer".into(),
            action_id: tool.action_id.clone(), operation: tool.driver_tool.clone(),
            permissions: vec![SystemPermissionKind::ScreenRecording], execution: "not_executed".into(),
        };
        let message = serde_json::to_string(&request).unwrap();
        let carrier = system_permission_error_carrier(SYSTEM_PERMISSION_REQUIRED, &message).unwrap();
        assert!(project_system_permission_required(Some("computer"), Some(&tool.action_id), true, &carrier).is_some());
        assert!(project_system_permission_required(Some("computer"), Some(&tool.action_id), false, &carrier).is_none());
        assert!(project_system_permission_required(Some("computer"), Some("foreign/action"), true, &carrier).is_none());
        assert!(system_permission_error_carrier("OTHER_ERROR", &message).is_none());
        request.permissions.push(SystemPermissionKind::ScreenRecording);
        assert!(!request.valid());
        request.permissions.clear();
        assert!(!request.valid());
        request.permissions.push(SystemPermissionKind::Accessibility);
        request.operation = "foreign_operation".into();
        assert!(!request.valid());
    }
}
