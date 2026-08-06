//! Wire shapes shared by the device face and the management REST face.

use serde::{Deserialize, Serialize};

use crate::registry::RobotRecord;

/// A robot as shown in the UI. Never carries the token or its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotDto {
    pub robot_id: String,
    pub name: String,
    pub companion_id: Option<String>,
    pub board: String,
    pub firmware_version: String,
    /// RFC 3339, or `null` if never seen.
    pub last_seen: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp_millis(0).expect("epoch is valid"))
        .to_rfc3339()
}

impl From<&RobotRecord> for RobotDto {
    fn from(record: &RobotRecord) -> Self {
        Self {
            robot_id: record.robot_id.clone(),
            name: record.name.clone(),
            companion_id: record.companion_id.clone(),
            board: record.board.clone(),
            firmware_version: record.firmware_version.clone(),
            last_seen: record.last_seen.map(ms_to_rfc3339),
            created_at: ms_to_rfc3339(record.created_at),
        }
    }
}

/// The fields we read out of the firmware's device report body. Everything else
/// in the report (partition table, chip info, heap) is ignored on purpose.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportBody {
    #[serde(default)]
    pub mac_address: Option<String>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub application: DeviceReportApplication,
    #[serde(default)]
    pub board: DeviceReportBoard,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportApplication {
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceReportBoard {
    #[serde(default, rename = "type")]
    pub board_type: Option<String>,
}
