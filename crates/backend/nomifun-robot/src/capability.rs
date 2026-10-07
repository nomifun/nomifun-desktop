//! Canonical Agent-facing `robot` Module and Action identities.
//!
//! Pairing and connection are resource facts, while the voice/audio loop is a
//! domain service. Neither is an authorable Agent Action. The only grants an
//! Agent can hold are the four operations below.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::registry::RobotRecord;

pub const ROBOT_MODULE_ID: &str = "robot";
pub const ROBOT_VISION_ACTION_ID: &str = "robot/vision";
pub const ROBOT_DISPLAY_ACTION_ID: &str = "robot/display";
pub const ROBOT_MOTION_ACTION_ID: &str = "robot/motion";
pub const ROBOT_DEVICE_ACTION_ID: &str = "robot/device";

pub const ROBOT_ACTION_IDS: [&str; 4] = [
    ROBOT_VISION_ACTION_ID,
    ROBOT_DISPLAY_ACTION_ID,
    ROBOT_MOTION_ACTION_ID,
    ROBOT_DEVICE_ACTION_ID,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RobotAction {
    Vision,
    Display,
    Motion,
    Device,
}

impl RobotAction {
    pub const ALL: [Self; 4] = [Self::Vision, Self::Display, Self::Motion, Self::Device];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Vision => ROBOT_VISION_ACTION_ID,
            Self::Display => ROBOT_DISPLAY_ACTION_ID,
            Self::Motion => ROBOT_MOTION_ACTION_ID,
            Self::Device => ROBOT_DEVICE_ACTION_ID,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            ROBOT_VISION_ACTION_ID => Some(Self::Vision),
            ROBOT_DISPLAY_ACTION_ID => Some(Self::Display),
            ROBOT_MOTION_ACTION_ID => Some(Self::Motion),
            ROBOT_DEVICE_ACTION_ID => Some(Self::Device),
            _ => None,
        }
    }

    pub const fn resource_operation(self) -> &'static str {
        match self {
            Self::Vision => "vision",
            Self::Display => "display",
            Self::Motion => "motion",
            Self::Device => "device",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Vision => "Camera and vision",
            Self::Display => "Display",
            Self::Motion => "Physical motion",
            Self::Device => "Other device controls",
        }
    }
}

pub fn validate_action_grants(
    grants: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<BTreeSet<RobotAction>, String> {
    let mut resolved = BTreeSet::new();
    for grant in grants {
        let grant = grant.as_ref();
        let action = RobotAction::parse(grant)
            .ok_or_else(|| format!("{grant:?} is not an action declared by the robot module"))?;
        resolved.insert(action);
    }
    if resolved.is_empty() {
        return Err("the robot module requires at least one explicit action grant".to_owned());
    }
    Ok(resolved)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RobotAvailabilityState {
    Available,
    Offline,
    PermissionDenied,
    MissingHardware,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotActionAvailability {
    pub action_id: String,
    pub state: RobotAvailabilityState,
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotModuleAvailability {
    pub module_id: String,
    pub robot_id: String,
    pub robot_name: String,
    pub connected: bool,
    pub actions: Vec<RobotActionAvailability>,
}

impl RobotModuleAvailability {
    pub fn action(&self, action: RobotAction) -> Option<&RobotActionAvailability> {
        self.actions
            .iter()
            .find(|availability| availability.action_id == action.id())
    }
}

/// Project device state for the capability UI and session admission. Device
/// permissions are a second authority and can only narrow the Agent grant.
pub fn module_availability(
    record: &RobotRecord,
    connected: bool,
    advertised_actions: &BTreeSet<RobotAction>,
) -> RobotModuleAvailability {
    let actions = RobotAction::ALL
        .into_iter()
        .map(|action| {
            let (state, guidance) = if !connected {
                (
                    RobotAvailabilityState::Offline,
                    Some("Connect the paired robot to this desktop before using this action.".to_owned()),
                )
            } else if !record.permissions.allows_action(action) {
                (
                    RobotAvailabilityState::PermissionDenied,
                    Some(format!(
                        "Enable {} for this robot in Device settings.",
                        action.display_name()
                    )),
                )
            } else if !advertised_actions.contains(&action) {
                (
                    RobotAvailabilityState::MissingHardware,
                    Some(format!(
                        "The connected robot does not advertise hardware for {}.",
                        action.display_name()
                    )),
                )
            } else {
                (RobotAvailabilityState::Available, None)
            };
            RobotActionAvailability {
                action_id: action.id().to_owned(),
                state,
                guidance,
            }
        })
        .collect();
    RobotModuleAvailability {
        module_id: ROBOT_MODULE_ID.to_owned(),
        robot_id: record.robot_id.clone(),
        robot_name: record.name.clone(),
        connected,
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RobotPermissions;

    fn record(permissions: RobotPermissions) -> RobotRecord {
        RobotRecord {
            robot_id: "robot-1".to_owned(),
            client_id: "client-1".to_owned(),
            name: "Desk bot".to_owned(),
            companion_id: Some("companion-1".to_owned()),
            token_hash: "digest".to_owned(),
            activation_code: None,
            board: "test".to_owned(),
            firmware_version: "1".to_owned(),
            last_seen: Some(1),
            created_at: 1,
            permissions,
            authorization_revision: 1,
        }
    }

    #[test]
    fn exact_action_ids_reject_retired_authoring_identities() {
        assert_eq!(RobotAction::ALL.map(RobotAction::id), ROBOT_ACTION_IDS);
        assert!(validate_action_grants(ROBOT_ACTION_IDS).is_ok());
        assert!(validate_action_grants(["robot/undeclared"]).is_err());
    }

    #[test]
    fn availability_distinguishes_offline_permission_and_missing_hardware() {
        let robot = record(RobotPermissions::default());
        let advertised = BTreeSet::from([RobotAction::Display, RobotAction::Motion]);
        let offline = module_availability(&robot, false, &advertised);
        assert_eq!(
            offline.action(RobotAction::Display).unwrap().state,
            RobotAvailabilityState::Offline
        );

        let online = module_availability(&robot, true, &advertised);
        assert_eq!(
            online.action(RobotAction::Display).unwrap().state,
            RobotAvailabilityState::Available
        );
        assert_eq!(
            online.action(RobotAction::Motion).unwrap().state,
            RobotAvailabilityState::PermissionDenied
        );
        assert_eq!(
            online.action(RobotAction::Vision).unwrap().state,
            RobotAvailabilityState::PermissionDenied
        );

        let mut permissions = RobotPermissions::default();
        permissions.vision = true;
        let missing = module_availability(&record(permissions), true, &advertised);
        assert_eq!(
            missing.action(RobotAction::Vision).unwrap().state,
            RobotAvailabilityState::MissingHardware
        );
    }
}
