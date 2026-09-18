//! Canonical `computer` capability-module contract owned by the native desktop
//! implementation.
//!
//! The public action identities are deliberately separate from the lower-level
//! operations accepted by [`crate::tool::ComputerTool`]. A compiled Agent
//! snapshot grants one or more module actions; the native operation is then
//! admitted only when it belongs to that exact action.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// One product capability, independent of the accessibility/input backend.
pub const COMPUTER_MODULE_ID: &str = "computer";

pub const COMPUTER_OBSERVE_ACTION_ID: &str = "computer/observe";
pub const COMPUTER_A11Y_OBSERVE_ACTION_ID: &str = "computer/a11y.observe";
pub const COMPUTER_INPUT_ACTION_ID: &str = "computer/input";
pub const COMPUTER_LAUNCH_ACTION_ID: &str = "computer/launch";

pub const COMPUTER_ACTION_IDS: [&str; 4] = [
    COMPUTER_OBSERVE_ACTION_ID,
    COMPUTER_A11Y_OBSERVE_ACTION_ID,
    COMPUTER_INPUT_ACTION_ID,
    COMPUTER_LAUNCH_ACTION_ID,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAction {
    Observe,
    A11yObserve,
    Input,
    Launch,
}

impl ComputerAction {
    pub const ALL: [Self; 4] = [
        Self::Observe,
        Self::A11yObserve,
        Self::Input,
        Self::Launch,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Observe => COMPUTER_OBSERVE_ACTION_ID,
            Self::A11yObserve => COMPUTER_A11Y_OBSERVE_ACTION_ID,
            Self::Input => COMPUTER_INPUT_ACTION_ID,
            Self::Launch => COMPUTER_LAUNCH_ACTION_ID,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            COMPUTER_OBSERVE_ACTION_ID => Some(Self::Observe),
            COMPUTER_A11Y_OBSERVE_ACTION_ID => Some(Self::A11yObserve),
            COMPUTER_INPUT_ACTION_ID => Some(Self::Input),
            COMPUTER_LAUNCH_ACTION_ID => Some(Self::Launch),
            _ => None,
        }
    }

    /// Resolve one private native operation to its sole authorizing product
    /// action. This mapping is intentionally total for every operation exposed
    /// by `ComputerTool` and has no legacy capability-ID aliases.
    pub fn for_native_operation(operation: &str) -> Option<Self> {
        match operation {
            "screenshot" | "cursor_position" | "list_windows" | "wait" => {
                Some(Self::Observe)
            }
            "observe" => Some(Self::A11yObserve),
            "click_element"
            | "set_element_value"
            | "right_click_element"
            | "double_click_element"
            | "left_click"
            | "right_click"
            | "middle_click"
            | "double_click"
            | "triple_click"
            | "mouse_move"
            | "left_click_drag"
            | "type"
            | "key"
            | "scroll"
            | "focus_window" => Some(Self::Input),
            "launch" => Some(Self::Launch),
            _ => None,
        }
    }

    pub const fn resource_operation(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::A11yObserve => "observe",
            Self::Input => "input",
            Self::Launch => "launch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAvailabilityState {
    Available,
    PermissionDenied,
    NoDesktop,
    UnsupportedPlatform,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerActionAvailability {
    pub action_id: String,
    pub state: ComputerAvailabilityState,
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerModuleAvailability {
    pub module_id: String,
    pub platform: String,
    pub actions: Vec<ComputerActionAvailability>,
}

impl ComputerModuleAvailability {
    pub fn action(&self, action: ComputerAction) -> Option<&ComputerActionAvailability> {
        self.actions
            .iter()
            .find(|availability| availability.action_id == action.id())
    }
}

/// Return a truthful point-in-time availability projection without prompting
/// for permission or synthesizing any input.
pub fn module_availability() -> ComputerModuleAvailability {
    let permissions = crate::permissions::permission_status();
    let desktop = desktop_is_available();
    let supported = cfg!(any(target_os = "windows", target_os = "macos"));

    let actions = ComputerAction::ALL
        .into_iter()
        .map(|action| {
            let (state, guidance) = if !supported {
                (
                    ComputerAvailabilityState::UnsupportedPlatform,
                    Some("Computer control is available only in the supported Windows or macOS desktop app.".to_owned()),
                )
            } else if matches!(
                action,
                ComputerAction::Observe | ComputerAction::A11yObserve | ComputerAction::Input
            ) && !desktop
            {
                (
                    ComputerAvailabilityState::NoDesktop,
                    Some(if matches!(action, ComputerAction::Observe) {
                        crate::permissions::screen_capture_hint_detailed()
                    } else {
                        crate::permissions::accessibility_hint_detailed()
                    }),
                )
            } else if matches!(action, ComputerAction::Observe)
                && permissions.screen_recording == Some(false)
            {
                (
                    ComputerAvailabilityState::PermissionDenied,
                    Some(crate::permissions::screen_capture_hint_detailed()),
                )
            } else if matches!(action, ComputerAction::A11yObserve | ComputerAction::Input)
                && permissions.accessibility == Some(false)
            {
                (
                    ComputerAvailabilityState::PermissionDenied,
                    Some(crate::permissions::accessibility_hint_detailed()),
                )
            } else if matches!(action, ComputerAction::Observe) && desktop {
                (ComputerAvailabilityState::Available, None)
            } else if cfg!(target_os = "windows") {
                // Windows has no TCC-style grant to request. Native calls can
                // still fail on the secure desktop or across an integrity
                // boundary; those failures remain operation results.
                (ComputerAvailabilityState::Available, None)
            } else if permissions.accessibility.is_some()
                || permissions.screen_recording.is_some()
                || matches!(action, ComputerAction::Launch)
            {
                (ComputerAvailabilityState::Available, None)
            } else {
                (
                    ComputerAvailabilityState::Unknown,
                    Some("Computer availability could not be verified in this desktop process.".to_owned()),
                )
            };
            ComputerActionAvailability {
                action_id: action.id().to_owned(),
                state,
                guidance,
            }
        })
        .collect();

    ComputerModuleAvailability {
        module_id: COMPUTER_MODULE_ID.to_owned(),
        platform: std::env::consts::OS.to_owned(),
        actions,
    }
}

fn desktop_is_available() -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        crate::macos_main::run_blocking(|| {
            xcap::Monitor::all()
                .map(|monitors| !monitors.is_empty())
                .map_err(|error| error.to_string())
        })
        .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

/// Validate an action allowlist received from an Agent snapshot. Unknown or
/// empty identities fail closed instead of silently widening the module.
pub fn validate_action_grants(
    grants: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<BTreeSet<ComputerAction>, String> {
    let mut resolved = BTreeSet::new();
    for grant in grants {
        let grant = grant.as_ref();
        let action = ComputerAction::parse(grant)
            .ok_or_else(|| format!("{grant:?} is not an action declared by the computer module"))?;
        resolved.insert(action);
    }
    if resolved.is_empty() {
        return Err("the computer module requires at least one explicit action grant".to_owned());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_action_ids_and_native_groups_are_stable() {
        assert_eq!(
            ComputerAction::ALL.map(ComputerAction::id),
            COMPUTER_ACTION_IDS
        );
        assert_eq!(
            ComputerAction::for_native_operation("screenshot"),
            Some(ComputerAction::Observe)
        );
        assert_eq!(
            ComputerAction::for_native_operation("observe"),
            Some(ComputerAction::A11yObserve)
        );
        assert_eq!(
            ComputerAction::for_native_operation("click_element"),
            Some(ComputerAction::Input)
        );
        assert_eq!(
            ComputerAction::for_native_operation("launch"),
            Some(ComputerAction::Launch)
        );
        assert_eq!(ComputerAction::for_native_operation("open_url"), None);
    }

    #[test]
    fn grants_reject_fragments_and_unknown_actions() {
        let grants = validate_action_grants([
            COMPUTER_OBSERVE_ACTION_ID,
            COMPUTER_INPUT_ACTION_ID,
        ])
        .unwrap();
        assert_eq!(grants.len(), 2);
        assert!(validate_action_grants(["computer/undeclared"]).is_err());
        assert!(validate_action_grants(std::iter::empty::<&str>()).is_err());
    }

    #[test]
    fn availability_reports_every_product_action_without_prompting() {
        let availability = module_availability();
        assert_eq!(availability.module_id, COMPUTER_MODULE_ID);
        assert_eq!(availability.actions.len(), COMPUTER_ACTION_IDS.len());
        for action in ComputerAction::ALL {
            assert!(availability.action(action).is_some());
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(
                availability.action(ComputerAction::Launch).unwrap().state,
                ComputerAvailabilityState::Available
            );
            assert!(matches!(
                availability.action(ComputerAction::Input).unwrap().state,
                ComputerAvailabilityState::Available | ComputerAvailabilityState::NoDesktop
            ));
        }
    }
}
